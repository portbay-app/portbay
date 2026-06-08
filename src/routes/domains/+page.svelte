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
  import { groups } from "$lib/stores/groups.svelte";
  import { errorBus } from "$lib/stores/errors.svelte";
  import { projectDetailPanel } from "$lib/stores/detailPanel.svelte";
  import { entitlements } from "$lib/stores/entitlements.svelte";
  import { account } from "$lib/stores/account.svelte";
  import { trackEvent } from "$lib/telemetry";
  import AddProjectForm from "$lib/components/wizard/AddProjectForm.svelte";
  import { dns } from "$lib/stores/dns.svelte";
  import { statusLabel } from "$lib/types/status";
  import {
    defaultDomainConfig,
    defaultAcmeConfig,
    typeLabel,
    type ProjectView,
    type ResolverMode,
    type SslMode,
    type AcmeIssuer,
    type AcmeEnvironment,
    type AcmeDnsProvider,
    type AcmeKeyType,
  } from "$lib/types/projects";

  // The single system-wide domain suffix (e.g. "portbay.test"), surfaced from
  // the DNS resolver status. The Hostname field is a Cloudflare-style split
  // input: the user edits only the subdomain prefix and this is the inline
  // suffix shown after it.
  const FALLBACK_SUFFIX = "portbay.test";
  const systemSuffix = $derived(dns.status?.suffix ?? FALLBACK_SUFFIX);

  // Mirror the label rules in src-tauri/src/domain.rs: each dot-separated label
  // is 1–63 chars of [a-z0-9-] with no leading/trailing hyphen.
  const LABEL_RE = /^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?$/;
  function isValidSubPrefix(prefix: string): boolean {
    const p = prefix.trim();
    if (p === "") return true; // empty prefix = the suffix's root domain
    return p
      .split(".")
      .every((l) => l.length >= 1 && l.length <= 63 && LABEL_RE.test(l));
  }

  // Split a stored hostname into { subPrefix, suffix }. Prefer the active system
  // suffix; otherwise fall back to "first label is the prefix, the rest is the
  // suffix" so a hostname on a different/legacy suffix still round-trips.
  function splitHostname(
    hostname: string,
    sysSuffix: string,
  ): { subPrefix: string; suffix: string } {
    const host = hostname.trim().toLowerCase();
    if (host === sysSuffix) return { subPrefix: "", suffix: sysSuffix };
    if (host.endsWith("." + sysSuffix)) {
      return {
        subPrefix: host.slice(0, host.length - sysSuffix.length - 1),
        suffix: sysSuffix,
      };
    }
    const dot = host.indexOf(".");
    if (dot > 0) {
      return { subPrefix: host.slice(0, dot), suffix: host.slice(dot + 1) };
    }
    return { subPrefix: host, suffix: sysSuffix };
  }

  let selectedId = $state<string | null>(null);
  let query = $state<string>("");
  let collapsed = $state<Set<string>>(new Set());
  let saving = $state<boolean>(false);
  let busy = $state<boolean>(false);
  let deleteArmed = $state<boolean>(false);
  // When true the right pane shows the inline "Add Domain" form instead of the
  // per-domain editor — mirrors the Certificates page's add flow.
  let adding = $state<boolean>(false);

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

  // ── List + filter ──────────────────────────────────────────────────────────
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

  // ── Grouping ────────────────────────────────────────────────────────────────
  // Domains that belong to a project group are listed under that group's name
  // (a project in two groups shows under both). Everything else stays a plain,
  // header-less list below the groups — exactly like the rail with no groups.
  interface DomainSection {
    id: string;
    name: string;
    items: ProjectView[];
  }

  const groupSections = $derived.by<DomainSection[]>(() => {
    const list = filtered;
    const out: DomainSection[] = [];
    const grps = [...groups.value].sort((a, b) => a.name.localeCompare(b.name));
    for (const g of grps) {
      const ids = new Set(g.projectIds);
      const items = list.filter((p) => ids.has(p.id));
      if (items.length === 0) continue;
      out.push({ id: `group:${g.id}`, name: g.name, items });
    }
    return out;
  });

  // Domains not in any group — rendered as a regular list with no header.
  const ungroupedItems = $derived.by<ProjectView[]>(() => {
    const claimed = new Set<string>();
    for (const g of groups.value) for (const id of g.projectIds) claimed.add(id);
    return filtered.filter((p) => !claimed.has(p.id));
  });

  const grouped = $derived(groupSections.length > 0);

  function toggleSection(id: string) {
    const next = new Set(collapsed);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    collapsed = next;
  }

  const selected = $derived<ProjectView | null>(
    projects.value.find((p) => p.id === selectedId) ?? null,
  );

  onMount(() => {
    void projects.start();
    void groups.refresh();
    // Load the domain suffix for the split hostname input if not already cached.
    if (!dns.status) void dns.refresh();
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
    subPrefix: string;
    suffix: string;
    port: string;
    https: boolean;
    autoStart: boolean;
    notes: string;
    pathPrefix: string;
    resolverMode: ResolverMode;
    autoManageCert: boolean;
    sslMode: SslMode;
    customCertPath: string;
    customKeyPath: string;
    acmeIssuer: AcmeIssuer;
    acmeEnvironment: AcmeEnvironment;
    acmeEmail: string;
    acmeKeyType: AcmeKeyType;
    acmeEabKeyId: string;
    acmeEabHmacKey: string;
    acmeZerosslApiKey: string;
    acmeDnsProvider: AcmeDnsProvider;
    acmeDnsApiToken: string;
    acmeForceRequest: boolean;
    acmeDebug: boolean;
    includeWildcardSubdomains: boolean;
    exposeWhenRunning: boolean;
  }

  let draft = $state<Draft | null>(null);
  let pristine = $state<string>("");
  let loadedFor = $state<string | null>(null);

  function loadDraft(p: ProjectView) {
    const d = p.domain ?? defaultDomainConfig();
    const acme = d.acme ?? defaultAcmeConfig();
    const { subPrefix, suffix } = splitHostname(p.hostname, systemSuffix);
    draft = {
      subPrefix,
      suffix,
      port: p.port != null ? String(p.port) : "",
      https: p.https,
      autoStart: p.autoStart,
      notes: d.notes ?? "",
      pathPrefix: d.pathPrefix ?? "",
      resolverMode: d.resolverMode ?? "auto",
      autoManageCert: d.autoManageCert ?? true,
      sslMode: d.sslMode ?? "automatic_local",
      customCertPath: d.customCertPath ?? "",
      customKeyPath: d.customKeyPath ?? "",
      acmeIssuer: acme.issuer ?? "lets_encrypt",
      acmeEnvironment: acme.environment ?? "production",
      acmeEmail: acme.email ?? "",
      acmeKeyType: acme.keyType ?? "p384",
      acmeEabKeyId: acme.eabKeyId ?? "",
      acmeEabHmacKey: acme.eabHmacKey ?? "",
      acmeZerosslApiKey: acme.zerosslApiKey ?? "",
      acmeDnsProvider: acme.dnsProvider ?? "none",
      acmeDnsApiToken: acme.dnsApiToken ?? "",
      acmeForceRequest: acme.forceRequest ?? false,
      acmeDebug: acme.debug ?? false,
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

  // The suffix arrives asynchronously from the DNS store; if it lands after the
  // draft loaded and the user hasn't edited, re-split so the prefix/suffix
  // boundary matches the real suffix.
  $effect(() => {
    const sfx = systemSuffix;
    if (!selected || !draft || dirty) return;
    untrack(() => {
      if (!draft || !selected) return;
      const split = splitHostname(selected.hostname, sfx);
      if (
        split.subPrefix !== draft.subPrefix ||
        split.suffix !== draft.suffix
      ) {
        draft = { ...draft, ...split };
        pristine = JSON.stringify(draft);
      }
    });
  });

  // Inline suffix options. Today this is just the system suffix (rendered as a
  // static addon); the draft's own suffix is kept so a hostname on a different
  // suffix still round-trips. Pro custom domains (Phase 2) will append the
  // user's verified domains here, turning the addon into a real dropdown.
  const suffixOptions = $derived.by<string[]>(() => {
    const opts = [systemSuffix];
    if (draft?.suffix && !opts.includes(draft.suffix)) opts.push(draft.suffix);
    return opts;
  });

  const subPrefixValid = $derived(
    draft ? isValidSubPrefix(draft.subPrefix) : true,
  );

  const composedHostname = $derived.by<string>(() => {
    if (!draft) return "";
    const p = draft.subPrefix.trim().toLowerCase();
    return p ? `${p}.${draft.suffix}` : draft.suffix;
  });

  // ── Actions ────────────────────────────────────────────────────────────────
  // Adding a domain creates a project (a domain is 1:1 with its project), so it
  // goes through the same tier gate `addProjectWizard.requestAdd()` enforced:
  // within the project cap, open the inline form; at the cap, open the
  // sign-in / upgrade sheet instead.
  function addDomain() {
    const count = projects.value.length;
    if (!entitlements.canAddProject(count)) {
      trackEvent("project_limit_reached");
      account.open({ intent: entitlements.upgradePromptAt(count) ?? "pro" });
      return;
    }
    adding = true;
  }

  function select(id: string) {
    adding = false;
    selectedId = id;
  }

  async function save() {
    if (!selected || !draft || !dirty || saving || !subPrefixValid) return;
    saving = true;
    const target = selected.id;
    const wildcardWas = selected.domain?.includeWildcardSubdomains ?? false;
    try {
      const domain = {
        notes: draft.notes.trim() ? draft.notes.trim() : null,
        pathPrefix: draft.pathPrefix.trim() ? draft.pathPrefix.trim() : null,
        resolverMode: draft.resolverMode,
        autoManageCert:
          draft.sslMode === "automatic_local" ? draft.autoManageCert : false,
        sslMode: draft.sslMode,
        customCertPath: draft.customCertPath.trim()
          ? draft.customCertPath.trim()
          : null,
        customKeyPath: draft.customKeyPath.trim()
          ? draft.customKeyPath.trim()
          : null,
        acme:
          draft.sslMode === "public_acme"
            ? {
                issuer: draft.acmeIssuer,
                environment: draft.acmeEnvironment,
                email: draft.acmeEmail.trim() ? draft.acmeEmail.trim() : null,
                keyType: draft.acmeKeyType,
                eabKeyId: draft.acmeEabKeyId.trim()
                  ? draft.acmeEabKeyId.trim()
                  : null,
                eabHmacKey: draft.acmeEabHmacKey.trim()
                  ? draft.acmeEabHmacKey.trim()
                  : null,
                zerosslApiKey: draft.acmeZerosslApiKey.trim()
                  ? draft.acmeZerosslApiKey.trim()
                  : null,
                dnsProvider: draft.acmeDnsProvider,
                dnsApiToken: draft.acmeDnsApiToken.trim()
                  ? draft.acmeDnsApiToken.trim()
                  : null,
                forceRequest: draft.acmeForceRequest,
                debug: draft.acmeDebug,
              }
            : null,
        includeWildcardSubdomains: draft.includeWildcardSubdomains,
        exposeWhenRunning: draft.exposeWhenRunning,
      };
      const patch: Record<string, unknown> = {
        hostname: composedHostname,
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
        category: "infrastructure",
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
        category: "infrastructure",
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

  const sslModeOptions: { value: SslMode; label: string; disabled?: boolean }[] =
    [
      { value: "automatic_local", label: "Automatic local HTTPS" },
      { value: "custom_certificate", label: "Custom certificate" },
      { value: "self_signed", label: "Self-signed fallback" },
      { value: "public_acme", label: "Public ACME / AutoSSL" },
    ];

  const acmeIssuerOptions: { value: AcmeIssuer; label: string }[] = [
    { value: "lets_encrypt", label: "Let's Encrypt" },
    { value: "zero_ssl", label: "ZeroSSL" },
    { value: "google_trust_services", label: "Google Trust Services" },
  ];
  const acmeEnvironmentOptions: { value: AcmeEnvironment; label: string }[] = [
    { value: "production", label: "Production" },
    { value: "staging", label: "Staging" },
  ];
  const acmeKeyTypeOptions: { value: AcmeKeyType; label: string }[] = [
    { value: "p384", label: "ECC P-384" },
    { value: "p256", label: "ECC P-256" },
    { value: "rsa2048", label: "RSA 2048" },
    { value: "rsa4096", label: "RSA 4096" },
  ];
  const dnsProviderOptions: { value: AcmeDnsProvider; label: string }[] = [
    { value: "none", label: "HTTP/TLS challenge" },
    { value: "cloudflare", label: "Cloudflare" },
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

{#snippet domainRow(p: ProjectView)}
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
          {#each groupSections as section (section.id)}
            {@const isCollapsed = collapsed.has(section.id)}
            <button
              type="button"
              onclick={() => toggleSection(section.id)}
              aria-expanded={!isCollapsed}
              class="w-full flex items-center gap-1.5 px-2 pt-2 pb-1 text-left
                     text-[11px] uppercase tracking-wide text-fg-subtle
                     hover:text-fg transition-colors"
            >
              <Icon
                name="chevron-right"
                size={12}
                class="shrink-0 transition-transform {isCollapsed ? '' : 'rotate-90'}"
              />
              <span class="min-w-0 flex-1 truncate font-medium">{section.name}</span>
              <span class="shrink-0 font-mono">{section.items.length}</span>
            </button>
            {#if !isCollapsed}
              <div class="space-y-1 pb-1">
                {#each section.items as p (p.id)}
                  {@render domainRow(p)}
                {/each}
              </div>
            {/if}
          {/each}
          {#if grouped && ungroupedItems.length > 0}
            <div class="pt-1 mt-1 border-t border-border/40"></div>
          {/if}
          {#each ungroupedItems as p (p.id)}
            {@render domainRow(p)}
          {/each}
        {/if}
      </div>

      {#if filtered.length > 0}
        <div
          class="shrink-0 flex items-center gap-2 px-3 py-2.5 border-t border-border/60"
        >
          <span class="text-[11px] text-fg-subtle">
            {filtered.length} domain{filtered.length === 1 ? "" : "s"}
            {#if grouped}
              · {groupSections.length} group{groupSections.length === 1 ? "" : "s"}
            {/if}
          </span>
        </div>
      {/if}
    </aside>

    <!-- Right pane — editor (or the inline Add Domain form while adding) -->
    <section class="flex-1 min-w-0 overflow-y-auto">
      {#if adding}
        <AddProjectForm
          mode="project"
          heading="New Domain"
          onClose={(createdId) => {
            adding = false;
            if (createdId) selectedId = createdId;
          }}
        />
      {:else if !selected || !draft}
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
        <div class="px-6 py-6">
          <div class="flex items-center justify-between gap-3">
            <h2 class="text-[15px] font-semibold text-fg">Edit Domain</h2>
            <span class="flex items-center gap-1.5 text-[11.5px] text-fg-subtle">
              <StatusDot status={selected.status} size="sm" />
              {statusLabel[selected.status]}
            </span>
          </div>

          <div class="mt-5 space-y-5">
            <!-- Hostname — Cloudflare-style split: editable subdomain prefix + inline suffix -->
            <div class="space-y-1.5">
              <label for="dom-host" class="block text-[12px] font-medium text-fg">
                Hostname
              </label>
              <div
                class="flex items-stretch rounded-lg bg-bg border transition-shadow
                       focus-within:ring-2 focus-within:ring-accent/40
                       {subPrefixValid ? 'border-border' : 'border-status-crashed/70'}"
              >
                <input
                  id="dom-host"
                  value={d.subPrefix}
                  oninput={(e) =>
                    (d.subPrefix = e.currentTarget.value
                      .toLowerCase()
                      .replace(/\s+/g, ""))}
                  placeholder="subdomain"
                  spellcheck="false"
                  autocapitalize="off"
                  autocomplete="off"
                  class="min-w-0 flex-1 h-9 px-3 rounded-l-lg bg-transparent font-mono
                         text-[13px] text-fg placeholder:text-fg-subtle
                         focus:outline-none"
                />
                {#if suffixOptions.length > 1}
                  <select
                    bind:value={d.suffix}
                    aria-label="Domain suffix"
                    class="h-9 shrink-0 pl-2 pr-7 rounded-r-lg border-l border-border
                           bg-surface-2/60 font-mono text-[13px] text-fg-muted
                           focus:outline-none"
                  >
                    {#each suffixOptions as s (s)}
                      <option value={s}>.{s}</option>
                    {/each}
                  </select>
                {:else}
                  <span
                    class="inline-flex items-center h-9 shrink-0 px-3 rounded-r-lg
                           border-l border-border bg-surface-2/50 font-mono
                           text-[13px] text-fg-muted select-none"
                  >
                    .{d.suffix}
                  </span>
                {/if}
              </div>
              {#if !subPrefixValid}
                <p class="text-[11px] text-status-crashed leading-relaxed">
                  Use lowercase letters, digits, and hyphens (e.g.
                  <code class="font-mono">cloud</code> or
                  <code class="font-mono">api.staging</code>). Leave empty for the
                  root domain.
                </p>
              {:else}
                <p class="text-[11px] text-fg-subtle leading-relaxed">
                  Resolves at <code class="font-mono">{composedHostname}</code>.
                </p>
              {/if}
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

            <div class="space-y-1.5">
              <label for="dom-ssl-mode" class="text-[12px] font-medium text-fg-muted">
                SSL mode
              </label>
              <select
                id="dom-ssl-mode"
                bind:value={d.sslMode}
                class="w-full h-9 px-3 rounded-lg bg-bg border border-border text-[13px]
                       text-fg focus:outline-none focus:ring-2 focus:ring-accent/40"
              >
                {#each sslModeOptions as o (o.value)}
                  <option value={o.value} disabled={o.disabled}>{o.label}</option>
                {/each}
              </select>
              <p class="text-[11px] text-fg-subtle leading-relaxed">
                {#if d.sslMode === "automatic_local"}
                  PortBay issues a locally trusted mkcert certificate and keeps
                  its SANs aligned with this hostname.
                {:else if d.sslMode === "custom_certificate"}
                  Use a company or hand-issued certificate. The cert and key must
                  match and cover this hostname.
                {:else if d.sslMode === "self_signed"}
                  Intended only as a fallback. Browsers will show warnings until
                  you switch back to a trusted mode.
                {:else}
                  Reserved for public domains only; local <code class="font-mono">.test</code>
                  names are not eligible for public ACME.
                {/if}
              </p>
            </div>

            {#if d.sslMode === "custom_certificate"}
              <div class="grid grid-cols-1 gap-3">
                <div class="space-y-1.5">
                  <label for="dom-custom-cert" class="text-[12px] font-medium text-fg-muted">
                    Certificate path
                  </label>
                  <input
                    id="dom-custom-cert"
                    bind:value={d.customCertPath}
                    placeholder="/absolute/path/cert.pem"
                    class="w-full h-9 px-3 rounded-lg bg-bg border border-border
                           text-[13px] text-fg font-mono placeholder:text-fg-subtle
                           focus:outline-none focus:ring-2 focus:ring-accent/40"
                  />
                </div>
                <div class="space-y-1.5">
                  <label for="dom-custom-key" class="text-[12px] font-medium text-fg-muted">
                    Private key path
                  </label>
                  <input
                    id="dom-custom-key"
                    bind:value={d.customKeyPath}
                    placeholder="/absolute/path/key.pem"
                    class="w-full h-9 px-3 rounded-lg bg-bg border border-border
                           text-[13px] text-fg font-mono placeholder:text-fg-subtle
                           focus:outline-none focus:ring-2 focus:ring-accent/40"
                  />
                </div>
              </div>
            {/if}

            {#if d.sslMode === "public_acme"}
              <div class="grid grid-cols-1 md:grid-cols-2 gap-3">
                <div class="space-y-1.5">
                  <label for="dom-acme-issuer" class="text-[12px] font-medium text-fg-muted">
                    Issuer
                  </label>
                  <select
                    id="dom-acme-issuer"
                    bind:value={d.acmeIssuer}
                    class="w-full h-9 px-3 rounded-lg bg-bg border border-border text-[13px]
                           text-fg focus:outline-none focus:ring-2 focus:ring-accent/40"
                  >
                    {#each acmeIssuerOptions as o (o.value)}
                      <option value={o.value}>{o.label}</option>
                    {/each}
                  </select>
                </div>
                <div class="space-y-1.5">
                  <label for="dom-acme-env" class="text-[12px] font-medium text-fg-muted">
                    Environment
                  </label>
                  <select
                    id="dom-acme-env"
                    bind:value={d.acmeEnvironment}
                    class="w-full h-9 px-3 rounded-lg bg-bg border border-border text-[13px]
                           text-fg focus:outline-none focus:ring-2 focus:ring-accent/40"
                  >
                    {#each acmeEnvironmentOptions as o (o.value)}
                      <option value={o.value}>{o.label}</option>
                    {/each}
                  </select>
                </div>
                <div class="space-y-1.5">
                  <label for="dom-acme-email" class="text-[12px] font-medium text-fg-muted">
                    Account email
                  </label>
                  <input
                    id="dom-acme-email"
                    type="email"
                    bind:value={d.acmeEmail}
                    placeholder="admin@example.com"
                    class="w-full h-9 px-3 rounded-lg bg-bg border border-border
                           text-[13px] text-fg placeholder:text-fg-subtle
                           focus:outline-none focus:ring-2 focus:ring-accent/40"
                  />
                </div>
                <div class="space-y-1.5">
                  <label for="dom-acme-key" class="text-[12px] font-medium text-fg-muted">
                    Algorithm
                  </label>
                  <select
                    id="dom-acme-key"
                    bind:value={d.acmeKeyType}
                    class="w-full h-9 px-3 rounded-lg bg-bg border border-border text-[13px]
                           text-fg focus:outline-none focus:ring-2 focus:ring-accent/40"
                  >
                    {#each acmeKeyTypeOptions as o (o.value)}
                      <option value={o.value}>{o.label}</option>
                    {/each}
                  </select>
                </div>
                <div class="space-y-1.5">
                  <label for="dom-acme-dns-provider" class="text-[12px] font-medium text-fg-muted">
                    DNS API provider
                  </label>
                  <select
                    id="dom-acme-dns-provider"
                    bind:value={d.acmeDnsProvider}
                    class="w-full h-9 px-3 rounded-lg bg-bg border border-border text-[13px]
                           text-fg focus:outline-none focus:ring-2 focus:ring-accent/40"
                  >
                    {#each dnsProviderOptions as o (o.value)}
                      <option value={o.value}>{o.label}</option>
                    {/each}
                  </select>
                </div>
                <div class="flex items-end gap-4 pb-1">
                  {@render toggleRow(
                    "Enable debug",
                    "Emit extra ACME diagnostics from Caddy.",
                    d.acmeDebug,
                    () => (d.acmeDebug = !d.acmeDebug),
                  )}
                  {@render toggleRow(
                    "Force request",
                    "Force Caddy to attempt issuance again on the next reload.",
                    d.acmeForceRequest,
                    () => (d.acmeForceRequest = !d.acmeForceRequest),
                  )}
                </div>
              </div>

              {#if d.acmeIssuer === "zero_ssl"}
                <div class="grid grid-cols-1 md:grid-cols-2 gap-3">
                  <input
                    aria-label="ZeroSSL API key"
                    bind:value={d.acmeZerosslApiKey}
                    placeholder="ZeroSSL API key"
                    class="w-full h-9 px-3 rounded-lg bg-bg border border-border
                           text-[13px] text-fg font-mono placeholder:text-fg-subtle
                           focus:outline-none focus:ring-2 focus:ring-accent/40"
                  />
                  <p class="text-[11px] text-fg-subtle leading-relaxed">
                    ZeroSSL can use its Caddy issuer API key, or EAB key id and
                    HMAC below.
                  </p>
                </div>
              {/if}

              {#if d.acmeIssuer === "zero_ssl" || d.acmeIssuer === "google_trust_services"}
                <div class="grid grid-cols-1 md:grid-cols-2 gap-3">
                  <input
                    aria-label="ACME EAB key id"
                    bind:value={d.acmeEabKeyId}
                    placeholder="EAB key id"
                    class="w-full h-9 px-3 rounded-lg bg-bg border border-border
                           text-[13px] text-fg font-mono placeholder:text-fg-subtle
                           focus:outline-none focus:ring-2 focus:ring-accent/40"
                  />
                  <input
                    aria-label="ACME EAB HMAC key"
                    type="password"
                    bind:value={d.acmeEabHmacKey}
                    placeholder="EAB HMAC key"
                    class="w-full h-9 px-3 rounded-lg bg-bg border border-border
                           text-[13px] text-fg font-mono placeholder:text-fg-subtle
                           focus:outline-none focus:ring-2 focus:ring-accent/40"
                  />
                </div>
              {/if}

              {#if d.acmeDnsProvider === "cloudflare"}
                <textarea
                  aria-label="Cloudflare DNS API token"
                  bind:value={d.acmeDnsApiToken}
                  rows="3"
                  placeholder="Cloudflare API token with Zone:DNS:Edit for this domain"
                  class="w-full px-3 py-2 rounded-lg bg-bg border border-border
                         text-[13px] text-fg font-mono placeholder:text-fg-subtle
                         focus:outline-none focus:ring-2 focus:ring-accent/40"
                ></textarea>
                <p class="text-[11px] text-fg-subtle leading-relaxed -mt-2">
                  Use a scoped token with Zone:Zone:Read and Zone:DNS:Edit for
                  this zone. Wildcard public certificates use Cloudflare DNS-01.
                </p>
              {/if}
            {/if}

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
                disabled={!dirty || saving || !subPrefixValid}
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
