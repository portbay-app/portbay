<!--
  /domains — create, route, and secure the local hostnames PortBay serves.

  A "domain" here is a project's hostname (1:1 with the project). This page is
  the side-by-side editor: the left rail lists every domain with a filter +
  pagination; the right pane edits the selected one inline (no overlay drawer).
  Every field writes through the existing `update_project` command; the new
  routing knobs (path prefix, resolver mode, per-host cert management, wildcard
  subdomains, expose-when-running) live on `ProjectView.domain`.
-->
<script lang="ts">
  import { onMount, untrack } from "svelte";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import StatusDot from "$lib/components/atoms/StatusDot.svelte";

  import { safeInvoke } from "$lib/ipc";
  import { projects } from "$lib/stores/projects.svelte";
  import { errorBus } from "$lib/stores/errors.svelte";
  import { projectDetailPanel } from "$lib/stores/detailPanel.svelte";
  import { addProjectWizard } from "$lib/stores/wizard.svelte";
  import { statusLabel } from "$lib/types/status";
  import {
    defaultDomainConfig,
    typeLabel,
    type ProjectView,
    type ResolverMode,
  } from "$lib/types/projects";

  const PAGE_SIZE = 8;

  let selectedId = $state<string | null>(null);
  let query = $state<string>("");
  let page = $state<number>(1);
  let saving = $state<boolean>(false);
  let busy = $state<boolean>(false);
  let deleteArmed = $state<boolean>(false);

  // ── Stats ────────────────────────────────────────────────────────────────
  const total = $derived(projects.value.length);
  const activeHttps = $derived(
    projects.value.filter((p) => p.https && p.status === "running").length,
  );
  const conflicts = $derived(
    projects.value.filter((p) => p.status === "port_conflict").length,
  );
  const wildcards = $derived(
    projects.value.filter((p) => p.domain?.includeWildcardSubdomains).length,
  );

  // ── List + filter + pagination ─────────────────────────────────────────────
  const filtered = $derived.by<ProjectView[]>(() => {
    const q = query.trim().toLowerCase();
    const list = [...projects.value].sort((a, b) =>
      a.hostname.localeCompare(b.hostname),
    );
    if (!q) return list;
    return list.filter(
      (p) =>
        p.hostname.toLowerCase().includes(q) ||
        p.name.toLowerCase().includes(q),
    );
  });

  const pageCount = $derived(Math.max(1, Math.ceil(filtered.length / PAGE_SIZE)));
  const pageItems = $derived(
    filtered.slice((page - 1) * PAGE_SIZE, (page - 1) * PAGE_SIZE + PAGE_SIZE),
  );

  // Reset to the first page whenever the filter changes.
  $effect(() => {
    query;
    untrack(() => (page = 1));
  });
  // Keep the page in range if the list shrinks (e.g. after a delete).
  $effect(() => {
    if (page > pageCount) untrack(() => (page = pageCount));
  });

  const selected = $derived<ProjectView | null>(
    projects.value.find((p) => p.id === selectedId) ?? null,
  );

  onMount(() => {
    void projects.start();
  });

  // Auto-select the first domain once the list is loaded and nothing's chosen.
  $effect(() => {
    if (!selectedId && filtered.length > 0) {
      const first = filtered[0].id;
      untrack(() => (selectedId = first));
    }
  });

  // ── Editable draft ─────────────────────────────────────────────────────────
  interface Draft {
    hostname: string;
    port: string;
    https: boolean;
    autoStart: boolean;
    notes: string;
    pathPrefix: string;
    resolverMode: ResolverMode;
    autoManageCert: boolean;
    includeWildcardSubdomains: boolean;
    exposeWhenRunning: boolean;
  }

  let draft = $state<Draft | null>(null);
  let pristine = $state<string>("");
  let loadedFor = $state<string | null>(null);

  function loadDraft(p: ProjectView) {
    const d = p.domain ?? defaultDomainConfig();
    draft = {
      hostname: p.hostname,
      port: p.port != null ? String(p.port) : "",
      https: p.https,
      autoStart: p.autoStart,
      notes: d.notes ?? "",
      pathPrefix: d.pathPrefix ?? "",
      resolverMode: d.resolverMode ?? "auto",
      autoManageCert: d.autoManageCert ?? true,
      includeWildcardSubdomains: d.includeWildcardSubdomains ?? false,
      exposeWhenRunning: d.exposeWhenRunning ?? false,
    };
    pristine = JSON.stringify(draft);
    deleteArmed = false;
  }

  // Load the draft when the selection changes — but never clobber in-progress
  // edits when a background `projects.refresh()` reruns this (the id guard).
  $effect(() => {
    const id = selectedId;
    if (!id) {
      untrack(() => {
        draft = null;
        pristine = "";
        loadedFor = null;
      });
      return;
    }
    if (id === loadedFor) return;
    const p = projects.value.find((x) => x.id === id);
    if (p) {
      untrack(() => {
        loadDraft(p);
        loadedFor = id;
      });
    }
  });

  const dirty = $derived(draft !== null && JSON.stringify(draft) !== pristine);

  // ── Actions ────────────────────────────────────────────────────────────────
  function addDomain() {
    addProjectWizard.requestAdd();
  }

  function select(id: string) {
    selectedId = id;
  }

  async function save() {
    if (!selected || !draft || !dirty || saving) return;
    saving = true;
    const target = selected.id;
    const wildcardWas = selected.domain?.includeWildcardSubdomains ?? false;
    try {
      const domain = {
        notes: draft.notes.trim() ? draft.notes.trim() : null,
        pathPrefix: draft.pathPrefix.trim() ? draft.pathPrefix.trim() : null,
        resolverMode: draft.resolverMode,
        autoManageCert: draft.autoManageCert,
        includeWildcardSubdomains: draft.includeWildcardSubdomains,
        exposeWhenRunning: draft.exposeWhenRunning,
      };
      const patch: Record<string, unknown> = {
        hostname: draft.hostname.trim(),
        https: draft.https,
        autoStart: draft.autoStart,
        domain,
      };
      const portNum = Number.parseInt(draft.port, 10);
      if (!Number.isNaN(portNum)) patch.port = portNum;

      await safeInvoke("update_project", { id: target, patch });

      // Newly enabling wildcard on an HTTPS host needs the cert reissued so its
      // SAN covers `*.hostname` (the cert reconciler only adds it on issuance).
      if (draft.https && draft.includeWildcardSubdomains && !wildcardWas) {
        try {
          await safeInvoke("reissue_cert", { id: target });
        } catch {
          /* non-fatal — surfaced on the Certificates page */
        }
      }

      await projects.refresh();
      const fresh = projects.value.find((p) => p.id === target);
      if (fresh) {
        loadDraft(fresh);
        loadedFor = target;
      }
      errorBus.push({
        code: "DOMAIN_SAVED",
        whatHappened: `${patch.hostname} saved.`,
        whyItMatters: "Caddy reloaded with the updated routing.",
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });
    } catch {
      /* safeInvoke already pushed a toast */
    } finally {
      saving = false;
    }
  }

  function revert() {
    if (selected) loadDraft(selected);
  }

  async function remove() {
    if (!selected || busy) return;
    if (!deleteArmed) {
      deleteArmed = true;
      return;
    }
    busy = true;
    const target = selected.id;
    const name = selected.name;
    try {
      await safeInvoke("remove_project", { id: target });
      await projects.refresh();
      selectedId = null;
      loadedFor = null;
      deleteArmed = false;
      errorBus.push({
        code: "DOMAIN_REMOVED",
        whatHappened: `${name} removed.`,
        whyItMatters: "Its hostname, route, and certificate were cleaned up.",
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });
    } catch {
      /* toast pushed */
    } finally {
      busy = false;
    }
  }

  function openProject() {
    if (selected) projectDetailPanel.show(selected.id);
  }

  const resolverOptions: { value: ResolverMode; label: string }[] = [
    { value: "auto", label: "Automatic" },
    { value: "hosts", label: "Hosts file" },
    { value: "dnsmasq", label: "dnsmasq wildcard" },
  ];
</script>

{#snippet toggle(on: boolean, flip: () => void, label: string)}
  <button
    type="button"
    role="switch"
    aria-checked={on}
    aria-label={label}
    onclick={flip}
    class="relative inline-flex h-5 w-9 shrink-0 items-center rounded-full
           transition-colors active:scale-95 focus-visible:outline-none
           focus-visible:ring-2 focus-visible:ring-accent/40
           {on ? 'bg-accent' : 'bg-surface-2 border border-border'}"
  >
    <span
      class="inline-block h-3.5 w-3.5 rounded-full bg-white shadow-sm
             transition-transform duration-150
             {on ? 'translate-x-[18px]' : 'translate-x-0.5'}"
    ></span>
  </button>
{/snippet}

{#snippet toggleRow(
  label: string,
  help: string,
  on: boolean,
  flip: () => void,
)}
  <div class="flex items-start justify-between gap-4 py-2">
    <div class="min-w-0">
      <div class="text-[12.5px] text-fg">{label}</div>
      {#if help}
        <div class="mt-0.5 text-[11px] text-fg-subtle leading-relaxed">
          {help}
        </div>
      {/if}
    </div>
    {@render toggle(on, flip, label)}
  </div>
{/snippet}

<div class="h-full flex flex-col">
  <!-- Stats header -->
  <header
    class="shrink-0 flex items-center gap-6 flex-wrap px-6 py-4 border-b border-border"
  >
    <div class="min-w-0 flex-1">
      <h1 class="text-[20px] font-semibold text-fg leading-none">Domains</h1>
      <p class="mt-1.5 text-[12.5px] text-fg-subtle leading-relaxed">
        Create, route, and secure local hostnames for your projects.
      </p>
    </div>

    <dl class="flex items-stretch gap-2.5">
      {#snippet stat(icon: import("$lib/components/atoms/Icon.svelte").IconName, tint: string, label: string, value: number)}
        <div
          class="flex items-center gap-2.5 rounded-xl border border-border
                 bg-surface px-3.5 py-2.5 min-w-[124px]"
        >
          <span class="grid place-items-center w-8 h-8 rounded-lg bg-surface-2 {tint}">
            <Icon name={icon} size={16} />
          </span>
          <div class="leading-tight">
            <dt class="text-[11px] text-fg-subtle whitespace-nowrap">{label}</dt>
            <dd class="text-[17px] font-semibold tabular-nums text-fg">{value}</dd>
          </div>
        </div>
      {/snippet}
      {@render stat("globe", "text-fg-muted", "Total domains", total)}
      {@render stat("lock", "text-status-running", "Active HTTPS", activeHttps)}
      {@render stat(
        "circle-alert",
        conflicts > 0 ? "text-status-unhealthy" : "text-fg-subtle",
        "Conflicts",
        conflicts,
      )}
      {@render stat("star", "text-accent", "Wildcard", wildcards)}
    </dl>
  </header>

  <div class="flex-1 min-h-0 flex">
    <!-- Left rail — domain list -->
    <aside
      class="w-[320px] shrink-0 border-r border-border bg-surface/40 flex flex-col"
      aria-label="Domains"
    >
      <div class="shrink-0 p-3 space-y-2.5 border-b border-border/60">
        <button
          type="button"
          onclick={addDomain}
          class="w-full inline-flex items-center justify-center gap-1.5 h-9 rounded-lg
                 text-[13px] font-medium bg-accent text-on-accent
                 hover:brightness-110 active:scale-[0.99] transition"
        >
          <Icon name="plus" size={15} />
          Add Domain
        </button>
        <label class="relative flex items-center" aria-label="Filter domains">
          <Icon
            name="search"
            size={14}
            class="absolute left-2.5 text-fg-subtle pointer-events-none"
          />
          <input
            type="search"
            bind:value={query}
            placeholder="Filter domains…"
            class="h-8 w-full pl-8 pr-3 rounded-lg bg-surface border border-border
                   text-[12.5px] text-fg placeholder:text-fg-subtle
                   focus:outline-none focus:ring-2 focus:ring-accent/40"
          />
        </label>
      </div>

      <div class="flex-1 min-h-0 overflow-y-auto p-2 space-y-1">
        {#if filtered.length === 0}
          <p class="px-2 py-8 text-center text-[12.5px] text-fg-subtle">
            {#if total === 0}
              No domains yet. Add a project to claim its first hostname.
            {:else}
              No domains match “{query}”.
            {/if}
          </p>
        {:else}
          {#each pageItems as p (p.id)}
            {@const isActive = selectedId === p.id}
            <button
              type="button"
              onclick={() => select(p.id)}
              aria-current={isActive ? "true" : undefined}
              class="w-full flex items-center gap-2.5 px-2.5 py-2 rounded-lg text-left
                     transition-colors cursor-pointer focus-visible:outline-none
                     focus-visible:ring-2 focus-visible:ring-accent/40
                     {isActive
                ? 'bg-accent/10 ring-1 ring-inset ring-accent/40'
                : 'hover:bg-surface-2/60'}"
            >
              <Icon
                name="lock"
                size={14}
                class={p.https ? "text-status-running shrink-0" : "text-fg-subtle shrink-0"}
              />
              <span class="min-w-0 flex-1 leading-tight">
                <span class="block text-[12.5px] font-mono text-fg truncate">
                  {p.hostname}
                </span>
                <span class="block text-[11px] text-fg-subtle truncate">
                  {p.name}
                </span>
              </span>
              <span class="shrink-0 text-right leading-tight">
                {#if p.port != null}
                  <span class="block text-[12px] font-mono tabular-nums text-fg-muted">
                    {p.port}
                  </span>
                {/if}
                <span class="flex items-center justify-end gap-1">
                  <StatusDot status={p.status} size="sm" />
                  <span class="text-[10.5px] text-fg-subtle">
                    {statusLabel[p.status]}
                  </span>
                </span>
              </span>
              <Icon name="chevron-right" size={13} class="text-fg-subtle shrink-0" />
            </button>
          {/each}
        {/if}
      </div>

      {#if filtered.length > 0}
        <div
          class="shrink-0 flex items-center justify-between gap-2 px-3 py-2.5
                 border-t border-border/60"
        >
          <span class="text-[11px] text-fg-subtle">
            Showing {pageItems.length} of {filtered.length} domain{filtered.length ===
            1
              ? ""
              : "s"}
          </span>
          {#if pageCount > 1}
            <div class="flex items-center gap-1">
              <button
                type="button"
                onclick={() => (page = Math.max(1, page - 1))}
                disabled={page === 1}
                aria-label="Previous page"
                class="grid place-items-center w-6 h-6 rounded-md text-fg-subtle
                       hover:bg-surface-2 hover:text-fg disabled:opacity-40
                       disabled:cursor-not-allowed transition-colors"
              >
                <Icon name="chevron-right" size={13} class="rotate-180" />
              </button>
              {#each Array(pageCount) as _, i (i)}
                <button
                  type="button"
                  onclick={() => (page = i + 1)}
                  aria-current={page === i + 1 ? "page" : undefined}
                  class="grid place-items-center min-w-6 h-6 px-1.5 rounded-md text-[11.5px]
                         tabular-nums transition-colors
                         {page === i + 1
                    ? 'bg-accent text-on-accent'
                    : 'text-fg-muted hover:bg-surface-2 hover:text-fg'}"
                >
                  {i + 1}
                </button>
              {/each}
              <button
                type="button"
                onclick={() => (page = Math.min(pageCount, page + 1))}
                disabled={page === pageCount}
                aria-label="Next page"
                class="grid place-items-center w-6 h-6 rounded-md text-fg-subtle
                       hover:bg-surface-2 hover:text-fg disabled:opacity-40
                       disabled:cursor-not-allowed transition-colors"
              >
                <Icon name="chevron-right" size={13} />
              </button>
            </div>
          {/if}
        </div>
      {/if}
    </aside>

    <!-- Right pane — editor -->
    <section class="flex-1 min-w-0 overflow-y-auto">
      {#if !selected || !draft}
        <div class="h-full grid place-items-center">
          <div class="text-center max-w-xs px-6">
            <span
              class="inline-grid place-items-center w-12 h-12 rounded-xl bg-surface-2 text-fg-subtle mx-auto"
            >
              <Icon name="link" size={24} />
            </span>
            <p class="mt-3 text-[13px] text-fg-muted">
              Select a domain to edit its routing, certificate, and resolver
              settings.
            </p>
          </div>
        </div>
      {:else}
        {@const d = draft}
        <div class="max-w-xl mx-auto px-6 py-6">
          <div class="flex items-center justify-between gap-3">
            <h2 class="text-[15px] font-semibold text-fg">Edit Domain</h2>
            <span class="flex items-center gap-1.5 text-[11.5px] text-fg-subtle">
              <StatusDot status={selected.status} size="sm" />
              {statusLabel[selected.status]}
            </span>
          </div>

          <div class="mt-5 space-y-5">
            <!-- Hostname -->
            <div class="space-y-1.5">
              <label for="dom-host" class="block text-[12px] font-medium text-fg">
                Hostname
              </label>
              <input
                id="dom-host"
                bind:value={d.hostname}
                spellcheck="false"
                autocapitalize="off"
                class="w-full h-9 px-3 rounded-lg bg-bg border border-border font-mono
                       text-[13px] text-fg focus:outline-none focus:ring-2
                       focus:ring-accent/40"
              />
            </div>

            <!-- Project (read-only; domains are 1:1 with their project) -->
            <div class="space-y-1.5">
              <span class="block text-[12px] font-medium text-fg">Project</span>
              <button
                type="button"
                onclick={openProject}
                title="Open project details"
                class="w-full h-9 px-3 rounded-lg bg-surface-2/50 border border-border
                       flex items-center gap-2 text-left hover:bg-surface-2
                       transition-colors group"
              >
                <Icon name="package" size={14} class="text-fg-subtle shrink-0" />
                <span class="text-[13px] text-fg truncate flex-1">
                  {selected.name}
                </span>
                <span
                  class="text-[10.5px] uppercase tracking-wide px-1.5 py-0.5 rounded
                         bg-surface-2 text-fg-subtle shrink-0"
                >
                  {typeLabel[selected.type]}
                </span>
                <Icon
                  name="external-link"
                  size={12}
                  class="text-fg-subtle opacity-0 group-hover:opacity-100 transition-opacity shrink-0"
                />
              </button>
            </div>

            <!-- Target Port + Protocol -->
            <div class="grid grid-cols-2 gap-4">
              <div class="space-y-1.5">
                <label for="dom-port" class="block text-[12px] font-medium text-fg">
                  Target Port
                </label>
                <input
                  id="dom-port"
                  bind:value={d.port}
                  inputmode="numeric"
                  placeholder="—"
                  class="w-full h-9 px-3 rounded-lg bg-bg border border-border font-mono
                         tabular-nums text-[13px] text-fg focus:outline-none
                         focus:ring-2 focus:ring-accent/40"
                />
              </div>
              <div class="space-y-1.5">
                <span class="block text-[12px] font-medium text-fg">Protocol</span>
                <div
                  class="h-9 grid grid-cols-2 gap-1 p-1 rounded-lg bg-surface-2/60
                         border border-border"
                >
                  <button
                    type="button"
                    onclick={() => (d.https = false)}
                    class="inline-flex items-center justify-center gap-1 rounded-md
                           text-[12px] font-medium transition-colors
                           {!d.https ? 'bg-bg text-fg shadow-sm' : 'text-fg-subtle hover:text-fg-muted'}"
                  >
                    HTTP
                  </button>
                  <button
                    type="button"
                    onclick={() => (d.https = true)}
                    class="inline-flex items-center justify-center gap-1 rounded-md
                           text-[12px] font-medium transition-colors
                           {d.https ? 'bg-bg text-fg shadow-sm' : 'text-fg-subtle hover:text-fg-muted'}"
                  >
                    <Icon name="lock" size={11} />
                    HTTPS
                  </button>
                </div>
              </div>
            </div>

            <!-- Path Prefix -->
            <div class="space-y-1.5">
              <label for="dom-path" class="block text-[12px] font-medium text-fg">
                Path Prefix
              </label>
              <input
                id="dom-path"
                bind:value={d.pathPrefix}
                placeholder="/"
                spellcheck="false"
                class="w-full h-9 px-3 rounded-lg bg-bg border border-border font-mono
                       text-[13px] text-fg focus:outline-none focus:ring-2
                       focus:ring-accent/40"
              />
              <p class="text-[11px] text-fg-subtle leading-relaxed">
                Serve the app under a sub-path. The prefix is stripped before
                proxying, so the app still sees <code class="font-mono">/</code>.
                Leave as <code class="font-mono">/</code> for the whole host.
              </p>
            </div>

            <!-- Resolver Mode -->
            <div class="space-y-1.5">
              <label for="dom-resolver" class="block text-[12px] font-medium text-fg">
                Resolver Mode
              </label>
              <select
                id="dom-resolver"
                bind:value={d.resolverMode}
                class="w-full h-9 px-3 rounded-lg bg-bg border border-border text-[13px]
                       text-fg focus:outline-none focus:ring-2 focus:ring-accent/40"
              >
                {#each resolverOptions as o (o.value)}
                  <option value={o.value}>{o.label}</option>
                {/each}
              </select>
              <p class="text-[11px] text-fg-subtle leading-relaxed">
                {#if d.resolverMode === "auto"}
                  PortBay picks: the dnsmasq wildcard when installed, otherwise an
                  <code class="font-mono">/etc/hosts</code> entry.
                {:else if d.resolverMode === "hosts"}
                  Always write an <code class="font-mono">/etc/hosts</code> entry for
                  this host, even when the wildcard would cover it.
                {:else}
                  Rely on the dnsmasq wildcard — no <code class="font-mono">/etc/hosts</code>
                  entry. The host won't resolve until the resolver is installed.
                {/if}
              </p>
            </div>

            <!-- Toggles -->
            <div class="rounded-xl border border-border divide-y divide-border/60 px-4">
              {@render toggleRow(
                "Auto-manage certificate",
                "PortBay issues and renews this hostname's local certificate.",
                d.autoManageCert,
                () => (d.autoManageCert = !d.autoManageCert),
              )}
              {@render toggleRow(
                "Include wildcard subdomains",
                "Also route and certify *." + selected.hostname + ".",
                d.includeWildcardSubdomains,
                () =>
                  (d.includeWildcardSubdomains = !d.includeWildcardSubdomains),
              )}
            </div>

            {#if d.https && !d.autoManageCert}
              <p
                class="flex items-start gap-2 text-[11.5px] text-status-unhealthy
                       bg-status-unhealthy/10 rounded-lg px-3 py-2 leading-relaxed"
              >
                <Icon name="circle-alert" size={13} class="mt-px shrink-0" />
                HTTPS is on but certificate auto-management is off — provide a cert
                yourself, or this host won't serve over TLS.
              </p>
            {/if}
            {#if d.includeWildcardSubdomains}
              <p class="text-[11px] text-fg-subtle leading-relaxed -mt-2">
                Subdomains resolve only under the dnsmasq wildcard resolver — an
                <code class="font-mono">/etc/hosts</code> entry can't express a
                wildcard.
              </p>
            {/if}

            <!-- Notes -->
            <div class="space-y-1.5">
              <label for="dom-notes" class="block text-[12px] font-medium text-fg">
                Notes / Description
                <span class="text-fg-subtle font-normal">(optional)</span>
              </label>
              <textarea
                id="dom-notes"
                bind:value={d.notes}
                rows="2"
                placeholder="What is this domain for?"
                class="w-full px-3 py-2 rounded-lg bg-bg border border-border text-[13px]
                       text-fg placeholder:text-fg-subtle resize-y focus:outline-none
                       focus:ring-2 focus:ring-accent/40"
              ></textarea>
            </div>

            <!-- Behaviour toggles -->
            <div class="rounded-xl border border-border divide-y divide-border/60 px-4">
              {@render toggleRow(
                "Auto-start with project",
                "Start this project automatically when PortBay launches.",
                d.autoStart,
                () => (d.autoStart = !d.autoStart),
              )}
              {@render toggleRow(
                "Expose only when project is running",
                "Publish the route only while the process is up; otherwise the host falls through to PortBay's placeholder.",
                d.exposeWhenRunning,
                () => (d.exposeWhenRunning = !d.exposeWhenRunning),
              )}
            </div>
          </div>

          <!-- Footer actions -->
          <div
            class="mt-6 flex items-center gap-2 pt-4 border-t border-border"
          >
            <button
              type="button"
              onclick={remove}
              disabled={busy}
              class="inline-flex items-center gap-1.5 h-9 px-3 rounded-lg text-[12.5px]
                     font-medium transition-colors active:scale-[0.98]
                     disabled:opacity-50
                     {deleteArmed
                ? 'bg-status-crashed text-bg'
                : 'text-status-crashed border border-status-crashed/30 hover:bg-status-crashed/10'}"
            >
              <Icon name="x" size={13} />
              {deleteArmed ? "Confirm delete" : "Delete"}
            </button>

            {#if deleteArmed}
              <button
                type="button"
                onclick={() => (deleteArmed = false)}
                class="h-9 px-3 rounded-lg text-[12.5px] text-fg-muted hover:bg-surface-2
                       transition-colors"
              >
                Cancel
              </button>
            {/if}

            <div class="ml-auto flex items-center gap-2">
              {#if dirty}
                <button
                  type="button"
                  onclick={revert}
                  disabled={saving}
                  class="h-9 px-3 rounded-lg text-[12.5px] text-fg-muted hover:bg-surface-2
                         transition-colors disabled:opacity-50"
                >
                  Revert
                </button>
              {/if}
              <button
                type="button"
                onclick={save}
                disabled={!dirty || saving}
                class="inline-flex items-center gap-1.5 h-9 px-4 rounded-lg text-[12.5px]
                       font-medium bg-accent text-on-accent hover:brightness-110
                       active:scale-[0.98] transition disabled:opacity-50
                       disabled:cursor-not-allowed"
              >
                <Icon name={saving ? "refresh-cw" : "check"} size={13} class={saving ? "animate-spin" : ""} />
                {saving ? "Saving…" : "Save Changes"}
              </button>
            </div>
          </div>
        </div>
      {/if}
    </section>
  </div>
</div>
