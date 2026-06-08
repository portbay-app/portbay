<!--
  SshHostTable — the SSH Access host workbench (left pane). A searchable,
  filterable table of saved hosts showing environment (stage), provider/region,
  live health + latency, auth method, and last-used. Selecting a row opens the
  detail panel (the parent owns selection via the `?host=` query param). Health
  comes from the `sshProbe` store; the footer refresh re-probes every host.
-->
<script lang="ts">
  import HostMark from "$lib/components/atoms/HostMark.svelte";
  import Icon from "$lib/components/atoms/Icon.svelte";
  import { providerLabel } from "$lib/ssh/providers";
  import { sshProbe } from "$lib/stores/sshProbe.svelte";
  import {
    STAGES,
    absoluteTime,
    authIcon,
    authSummary,
    destination,
    healthMeta,
    relativeTime,
    stageMeta,
  } from "$lib/ssh/hostFormat";
  import type { SshConnectionView } from "$lib/types/sshConnections";

  interface Props {
    connections: SshConnectionView[];
    selectedId: string | null;
    onselect: (id: string) => void;
    onadd: () => void;
    onimport: () => void;
    onmanageIdentities: () => void;
    onrefresh: () => void;
    onedit: (id: string) => void;
    ondetectOs: (id: string) => void;
    onremove: (id: string) => void;
  }
  let {
    connections,
    selectedId,
    onselect,
    onadd,
    onimport,
    onmanageIdentities,
    onrefresh,
    onedit,
    ondetectOs,
    onremove,
  }: Props = $props();

  // Per-row quick-actions menu (the three-dots in the last column). Position is
  // captured from the trigger button and the menu is rendered fixed, so it
  // escapes the table's scroll clipping.
  let menu = $state<{ id: string; x: number; y: number } | null>(null);
  function openRowMenu(e: MouseEvent, id: string) {
    e.preventDefault();
    e.stopPropagation();
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    menu = { id, x: r.right, y: r.bottom };
  }
  function runAction(fn: (id: string) => void, id: string) {
    menu = null;
    fn(id);
  }

  let query = $state("");
  let stageFilter = $state<"all" | (typeof STAGES)[number]>("all");
  let filtersOpen = $state(false);
  let providerFilter = $state<Set<string>>(new Set());
  let healthFilter = $state<Set<string>>(new Set());
  let searchEl = $state<HTMLInputElement | null>(null);

  // Distinct providers present, for the Filters popover.
  const presentProviders = $derived.by(() => {
    const seen = new Map<string, string>();
    for (const c of connections) {
      const id = (c.environment ?? "").toLowerCase();
      if (id && !seen.has(id)) seen.set(id, providerLabel(id) ?? id);
    }
    return [...seen.entries()].sort((a, b) => a[1].localeCompare(b[1]));
  });

  const activeFilterCount = $derived(providerFilter.size + healthFilter.size);

  const filtered = $derived.by(() => {
    const q = query.trim().toLowerCase();
    return connections
      .filter((c) => {
        if (stageFilter !== "all" && (c.stage ?? "").toLowerCase() !== stageFilter) {
          return false;
        }
        if (providerFilter.size && !providerFilter.has((c.environment ?? "").toLowerCase())) {
          return false;
        }
        if (healthFilter.size) {
          const h = sshProbe.get(c.id)?.health ?? "unknown";
          if (!healthFilter.has(h)) return false;
        }
        if (!q) return true;
        const haystack = [
          c.name,
          c.sshHost,
          c.sshUser,
          c.detectedOs ?? "",
          c.region ?? "",
          providerLabel(c.environment) ?? "",
          c.stage ?? "",
          ...(c.tags ?? []),
        ]
          .join(" ")
          .toLowerCase();
        return haystack.includes(q);
      })
      .sort((a, b) => (b.lastUsed ?? 0) - (a.lastUsed ?? 0));
  });

  function toggle(set: Set<string>, value: string): Set<string> {
    const next = new Set(set);
    if (next.has(value)) next.delete(value);
    else next.add(value);
    return next;
  }

  function clearFilters() {
    providerFilter = new Set();
    healthFilter = new Set();
    stageFilter = "all";
  }

  function focusHostRow(index: number) {
    const max = filtered.length - 1;
    if (max < 0) return;
    const next = Math.max(0, Math.min(max, index));
    document.querySelector<HTMLElement>(`[data-host-row="${next}"]`)?.focus();
  }

  function onHostRowKeydown(e: KeyboardEvent, id: string, index: number) {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      focusHostRow(index + 1);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      focusHostRow(index - 1);
    } else if (e.key === "Home") {
      e.preventDefault();
      focusHostRow(0);
    } else if (e.key === "End") {
      e.preventDefault();
      focusHostRow(filtered.length - 1);
    } else if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      onselect(id);
    }
  }

  // ⌘K / Ctrl+K focuses the search box.
  function onWindowKey(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
      e.preventDefault();
      searchEl?.focus();
    }
  }

  const HEALTH_FILTERS = [
    { id: "healthy", label: "Healthy" },
    { id: "degraded", label: "Degraded" },
    { id: "down", label: "Down" },
    { id: "unknown", label: "Unknown" },
  ];
</script>

<svelte:window onkeydown={onWindowKey} onclick={() => (menu = null)} />

<section class="flex h-full min-w-0 flex-1 flex-col overflow-hidden">
  <!-- Header -->
  <header class="px-8 pt-7 pb-4">
    <div class="flex items-start gap-3">
      <span class="grid place-items-center w-10 h-10 rounded-xl bg-accent/12 text-accent shrink-0">
        <Icon name="terminal" size={20} />
      </span>
      <div class="min-w-0 flex-1">
        <h1 class="text-[20px] font-semibold tracking-tight text-fg">SSH Access</h1>
        <p class="mt-0.5 text-[12.5px] text-fg-muted">
          Connect securely to remote hosts and boot environments.
        </p>
      </div>
      <button
        type="button"
        onclick={onmanageIdentities}
        class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md text-[12px]
               font-medium border border-border text-fg-muted hover:text-fg hover:bg-surface-2"
        title="Manage reusable SSH identities"
      >
        <Icon name="key" size={12} />
        Identities
      </button>
      <button
        type="button"
        onclick={onimport}
        class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md text-[12px]
               font-medium border border-border text-fg-muted hover:text-fg hover:bg-surface-2"
        title="Import hosts from ~/.ssh/config"
      >
        <Icon name="file-text" size={12} />
        Import
      </button>
      <button
        type="button"
        onclick={onadd}
        class="inline-flex items-center gap-1.5 h-8 px-3.5 rounded-md text-[12px]
               font-medium text-on-accent bg-accent shadow-sm hover:brightness-110
               active:brightness-95 transition"
      >
        <Icon name="plus" size={13} />
        Add host
      </button>
    </div>

    <!-- Toolbar: search · filters · stage tabs -->
    <div class="mt-5 flex flex-wrap items-center gap-3">
      <div class="relative w-72 max-w-full">
        <span class="absolute left-2.5 top-1/2 -translate-y-1/2 text-fg-subtle">
          <Icon name="search" size={14} />
        </span>
        <input
          bind:this={searchEl}
          bind:value={query}
          placeholder="Search hosts…"
          class="w-full h-9 rounded-lg border border-border bg-surface pl-8 pr-12 text-[12.5px] text-fg
                 placeholder:text-fg-subtle focus:border-accent/60 focus:outline-none"
        />
        <kbd class="absolute right-2 top-1/2 -translate-y-1/2 rounded border border-border
                    px-1.5 py-0.5 text-[10px] font-medium text-fg-subtle">⌘ K</kbd>
      </div>

      <div class="relative">
        <button
          type="button"
          onclick={() => (filtersOpen = !filtersOpen)}
          class="inline-flex items-center gap-1.5 h-9 px-3 rounded-lg text-[12.5px]
                 font-medium border text-fg-muted hover:text-fg hover:bg-surface-2
                 {activeFilterCount ? 'border-accent/50 text-fg' : 'border-border'}"
        >
          <Icon name="sliders-horizontal" size={13} />
          Filters
          {#if activeFilterCount}
            <span class="ml-0.5 inline-grid place-items-center h-4 min-w-4 px-1 rounded-full
                         bg-accent text-on-accent text-[10px] font-semibold tabular-nums">
              {activeFilterCount}
            </span>
          {/if}
        </button>

        {#if filtersOpen}
          <!-- Click-away backdrop -->
          <button
            type="button"
            class="fixed inset-0 z-10 cursor-default"
            aria-label="Close filters"
            onclick={() => (filtersOpen = false)}
          ></button>
          <div
            class="absolute left-0 z-20 mt-1.5 w-64 rounded-xl border border-border bg-surface p-3 shadow-xl"
          >
            <div class="flex items-center justify-between">
              <span class="text-[11px] font-semibold uppercase tracking-wide text-fg-subtle">Health</span>
              {#if activeFilterCount}
                <button type="button" onclick={clearFilters} class="text-[11px] text-accent hover:underline">
                  Clear all
                </button>
              {/if}
            </div>
            <div class="mt-2 flex flex-wrap gap-1.5">
              {#each HEALTH_FILTERS as h (h.id)}
                <button
                  type="button"
                  onclick={() => (healthFilter = toggle(healthFilter, h.id))}
                  class="inline-flex items-center gap-1.5 h-7 px-2 rounded-md border text-[11.5px]
                         {healthFilter.has(h.id)
                           ? 'border-accent/50 bg-accent/10 text-fg'
                           : 'border-border text-fg-muted hover:bg-surface-2'}"
                >
                  <span class="w-1.5 h-1.5 rounded-full {healthMeta(h.id as never).dotClass}"></span>
                  {h.label}
                </button>
              {/each}
            </div>

            {#if presentProviders.length}
              <div class="mt-3 text-[11px] font-semibold uppercase tracking-wide text-fg-subtle">
                Provider
              </div>
              <div class="mt-2 flex flex-wrap gap-1.5">
                {#each presentProviders as [id, label] (id)}
                  <button
                    type="button"
                    onclick={() => (providerFilter = toggle(providerFilter, id))}
                    class="inline-flex items-center gap-1.5 h-7 px-2 rounded-md border text-[11.5px]
                           {providerFilter.has(id)
                             ? 'border-accent/50 bg-accent/10 text-fg'
                             : 'border-border text-fg-muted hover:bg-surface-2'}"
                  >
                    <HostMark environment={id} size={13} />
                    {label}
                  </button>
                {/each}
              </div>
            {/if}
          </div>
        {/if}
      </div>

      <div class="ml-auto inline-flex items-center gap-0.5 rounded-lg border border-border bg-surface p-0.5">
        <button
          type="button"
          onclick={() => (stageFilter = "all")}
          class="h-7 px-3 rounded-md text-[12px] font-medium transition-colors
                 {stageFilter === 'all' ? 'bg-accent/15 text-accent' : 'text-fg-muted hover:text-fg'}"
        >
          All
        </button>
        {#each STAGES as s (s)}
          <button
            type="button"
            onclick={() => (stageFilter = s)}
            class="h-7 px-3 rounded-md text-[12px] font-medium capitalize transition-colors
                   {stageFilter === s ? 'bg-accent/15 text-accent' : 'text-fg-muted hover:text-fg'}"
          >
            {s}
          </button>
        {/each}
      </div>
    </div>
  </header>

  <!-- Table -->
  <div class="min-h-0 flex-1 overflow-auto px-8 pb-4">
    <div class="min-w-[680px] overflow-hidden rounded-xl border border-border/70">
      <!-- Column headers -->
      <div
        class="grid grid-cols-[minmax(160px,2fr)_104px_minmax(130px,1.3fr)_110px_minmax(120px,1fr)_130px_36px]
               items-center gap-2 border-b border-border/70 bg-surface-2/40 px-4 py-2.5
               text-[10.5px] font-semibold uppercase tracking-wide text-fg-subtle"
      >
        <span>Host</span>
        <span>Environment</span>
        <span>Provider / Region</span>
        <span>Health</span>
        <span>Auth method</span>
        <span>Last used</span>
        <span class="text-right"><Icon name="settings" size={13} /></span>
      </div>

      {#if filtered.length === 0}
        <p class="px-4 py-10 text-center text-[12.5px] text-fg-subtle">
          {query || activeFilterCount ? "No hosts match your filters." : "No hosts yet."}
        </p>
      {:else}
        {#each filtered as c, rowIndex (c.id)}
          {@const probe = sshProbe.get(c.id)}
          {@const stage = stageMeta(c.stage)}
          {@const health = healthMeta(probe?.health)}
          {@const auth = authSummary(c)}
          {@const prov = providerLabel(c.provider)}
          {@const envBrand = providerLabel(c.environment)}
          <div
            role="button"
            tabindex="0"
            data-host-row={rowIndex}
            onclick={() => onselect(c.id)}
            onkeydown={(e) => onHostRowKeydown(e, c.id, rowIndex)}
            class="grid w-full cursor-pointer grid-cols-[minmax(160px,2fr)_104px_minmax(130px,1.3fr)_110px_minmax(120px,1fr)_130px_36px]
                   items-center gap-2 border-b border-border/40 px-4 py-3 text-left transition-colors
                   last:border-b-0 hover:bg-surface-2/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40
                   {selectedId === c.id ? 'bg-accent/[0.06] ring-1 ring-inset ring-accent/40' : ''}"
          >
            <!-- Host -->
            <span class="flex min-w-0 items-center gap-2.5">
              <HostMark environment={c.environment} size={26} class="shrink-0" />
              <span class="min-w-0">
                <span class="block truncate text-[13px] font-semibold text-fg">{c.name}</span>
                <span class="block truncate font-mono text-[11px] text-fg-subtle">
                  {c.sshHost}{c.sshPort !== 22 ? `:${c.sshPort}` : ""}
                </span>
              </span>
            </span>

            <!-- Environment: deployment stage when set, else the detected
                 platform brand (cPanel / Ubuntu / AWS…). -->
            <span class="flex min-w-0 items-center">
              {#if stage}
                <span class="inline-flex items-center rounded-md px-1.5 py-0.5 text-[10.5px] font-medium {stage.chipClass}">
                  {stage.label}
                </span>
              {:else if envBrand}
                <span class="flex min-w-0 items-center gap-1.5">
                  <HostMark environment={c.environment} size={16} class="shrink-0" />
                  <span class="truncate text-[12px] text-fg">{envBrand}</span>
                </span>
              {:else}
                <span class="text-[11px] text-fg-subtle">—</span>
              {/if}
            </span>

            <!-- Provider / Region: the real cloud host + region (auto-detected),
                 separate from the control-panel/distro brand above. -->
            <span class="flex min-w-0 items-center gap-2">
              {#if prov}
                <HostMark environment={c.provider} size={18} class="shrink-0" />
                <span class="min-w-0">
                  <span class="block truncate text-[12px] text-fg">{prov}</span>
                  {#if c.region}
                    <span class="block truncate text-[11px] text-fg-subtle">{c.region}</span>
                  {/if}
                </span>
              {:else if c.region}
                <span class="truncate text-[12px] text-fg">{c.region}</span>
              {:else}
                <span class="text-[11px] text-fg-subtle">—</span>
              {/if}
            </span>

            <!-- Health -->
            <span class="flex items-center gap-1.5">
              <span class="w-1.5 h-1.5 rounded-full {health.dotClass}"></span>
              <span class="min-w-0">
                <span class="block text-[12px] text-fg">{health.label}</span>
                {#if probe?.latencyMs != null}
                  <span class="block text-[11px] tabular-nums text-fg-subtle">{probe.latencyMs}ms</span>
                {/if}
              </span>
            </span>

            <!-- Auth method -->
            <span class="flex min-w-0 items-center gap-1.5 text-fg-muted">
              <Icon name={authIcon(c)} size={13} class="shrink-0" />
              <span class="min-w-0">
                <span class="block truncate text-[12px] text-fg">{auth.label}</span>
                {#if auth.detail}
                  <span class="block truncate font-mono text-[11px] text-fg-subtle">{auth.detail}</span>
                {/if}
              </span>
            </span>

            <!-- Last used -->
            <span class="min-w-0">
              <span class="block truncate text-[12px] text-fg">{relativeTime(c.lastUsed)}</span>
              {#if c.lastUsed}
                <span class="block truncate text-[11px] text-fg-subtle">{absoluteTime(c.lastUsed)}</span>
              {/if}
            </span>

            <!-- Quick actions (three-dots), aligned under the Settings header. -->
            <span class="flex justify-end">
              <button
                type="button"
                onclick={(e) => openRowMenu(e, c.id)}
                class="grid h-7 w-7 place-items-center rounded-md text-fg-subtle hover:bg-surface-2 hover:text-fg
                       {menu?.id === c.id ? 'bg-surface-2 text-fg' : ''}"
                aria-label="Host actions"
                aria-haspopup="menu"
              >
                <Icon name="more-horizontal" size={16} />
              </button>
            </span>
          </div>
        {/each}
      {/if}
    </div>
  </div>

  <!-- Footer -->
  <footer class="flex items-center justify-between border-t border-border/60 px-8 py-2.5">
    <span class="text-[11.5px] text-fg-subtle">
      {filtered.length} host{filtered.length === 1 ? "" : "s"}
      {#if filtered.length !== connections.length}<span class="text-fg-subtle">of {connections.length}</span>{/if}
    </span>
    <button
      type="button"
      onclick={onrefresh}
      class="inline-flex items-center gap-1.5 h-7 px-2.5 rounded-md text-[11.5px]
             text-fg-muted hover:text-fg hover:bg-surface-2"
      title="Re-probe host health"
    >
      <Icon name="refresh-cw" size={12} />
      Refresh
    </button>
  </footer>
</section>

<!-- Per-host quick-actions menu. Every read is null-safe so setting `menu = null`
     mid-flush can't throw before the {#if} unmounts. -->
{#if menu}
  {@const m = menu}
  {@const conn = connections.find((c) => c.id === m?.id)}
  <div
    class="fixed z-50 w-48 rounded-lg border border-border bg-surface p-1 shadow-xl"
    style="left: {Math.max(8, (m?.x ?? 0) - 192)}px; top: {(m?.y ?? 0) + 4}px"
    role="menu"
    tabindex="-1"
  >
    <button type="button" role="menuitem" onclick={() => runAction(onselect, m?.id ?? "")} class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg">
      <Icon name="terminal" size={13} /> Open
    </button>
    <button type="button" role="menuitem" onclick={() => runAction(onedit, m?.id ?? "")} class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg">
      <Icon name="pencil" size={13} /> Edit host
    </button>
    <button type="button" role="menuitem" onclick={() => runAction(ondetectOs, m?.id ?? "")} class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg">
      <Icon name="server-cog" size={13} /> Detect OS
    </button>
    <div class="my-1 border-t border-border/60"></div>
    <button
      type="button"
      role="menuitem"
      disabled={conn?.inUse}
      title={conn?.inUse ? "Remove this host's tunnels first" : ""}
      onclick={() => runAction(onremove, m?.id ?? "")}
      class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-status-crashed hover:bg-status-crashed/10 disabled:opacity-50 disabled:hover:bg-transparent"
    >
      <Icon name="trash-2" size={13} /> Remove from PortBay
    </button>
  </div>
{/if}
