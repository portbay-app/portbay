<!--
  Sidebar — left nav rail (redesigned).

  Three regions:
    1. Brand header — lighthouse mark + "PortBay" wordmark, with a "Pro"
       pill alongside it for Pro entitlements. The pt-9 padding keeps the
       macOS traffic lights (titleBarStyle: Overlay) clear of the brand row.
    2. Nav — Projects, Groups (collapsible), Domains, Services, Logs,
       Settings. Languages is reachable via the palette / Settings, not the
       top-level nav.
    3. System footer — overall sidecar pill ("All Systems Operational"),
       CPU / Memory / Disk meters, and a thin version row with a GitHub
       link. The footer used to live in the right rail; in the redesign
       it slides under the sidebar so the rail is free for project
       inspection.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { openUrl } from "$lib/security/openUrl";
  import { getVersion } from "@tauri-apps/api/app";

  import SidebarItem from "./SidebarItem.svelte";
  import SidebarResizeHandle from "./SidebarResizeHandle.svelte";
  import Icon from "$lib/components/atoms/Icon.svelte";
  import StatusDot from "$lib/components/atoms/StatusDot.svelte";
  import LighthouseLogo from "$lib/components/atoms/LighthouseLogo.svelte";

  import { sidecars } from "$lib/stores/sidecars.svelte";
  import { metrics } from "$lib/stores/metrics.svelte";
  import { density } from "$lib/stores/density.svelte";
  import { groups } from "$lib/stores/groups.svelte";
  import { groupEditor } from "$lib/stores/groupEditor.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import { databases } from "$lib/stores/databases.svelte";
  import { entitlements } from "$lib/stores/entitlements.svelte";
  import { SIDECAR_ORDER } from "$lib/types/sidecars";
  import type { SidecarState } from "$lib/types/sidecars";
  import type { PortbayStatus } from "$lib/types/status";

  // Read the version from the Tauri app handle so the footer always
  // matches the running build — no hardcoded string to drift post-release.
  let appVersion = $state<string>("");

  /** Hide the resize handle in compact density — the layout forces a
   *  fixed sidebar width there, so a drag handle wouldn't do anything. */
  const showHandle = $derived(density.value !== "compact");

  /** Running database instances — surfaced as a badge on the Databases nav
   *  item (N running). Reflects the databases store, refreshed app-wide on
   *  boot and whenever the Databases page or its actions run. */
  const runningDbCount = $derived(
    databases.value.filter((d) => d.status === "running").length,
  );

  /** Compact density renders the rail as an icon-only strip. Driven by the
   *  same density preference the Settings page toggles, so "compact" means
   *  both a denser layout and a collapsed sidebar. */
  const collapsed = $derived(density.value === "compact");

  let groupsOpen = $state<boolean>(true);

  onMount(() => {
    void groups.refresh();
    // Metrics need to be live for the footer meters regardless of which
    // route the user is on. The store is idempotent — calling start
    // multiple times is fine.
    void metrics.start();
    void getVersion()
      .then((v) => (appVersion = v))
      .catch(() => {});
  });

  /** Per-group derived state: how many members are currently running. */
  function runningCount(memberIds: string[]): number {
    const running = new Set(
      projects.value.filter((p) => p.status === "running").map((p) => p.id),
    );
    return memberIds.filter((id) => running.has(id)).length;
  }

  // Aggregate sidecar state — worst-of-N picks the pill colour and copy.
  const SEVERITY: SidecarState[] = [
    "unreachable",
    "not_installed",
    "stopped",
    "running",
  ];

  const aggregate = $derived.by<SidecarState>(() => {
    const states = SIDECAR_ORDER.map((k) => sidecars.value[k].status);
    for (const c of SEVERITY) {
      if (states.includes(c)) return c;
    }
    return "running";
  });

  const pillStatus = $derived.by<PortbayStatus>(() => {
    switch (aggregate) {
      case "running":
        return "running";
      case "stopped":
        return "stopped";
      case "not_installed":
        return "port_conflict";
      case "unreachable":
        return "crashed";
    }
  });

  const pillTitle = $derived.by(() => {
    switch (aggregate) {
      case "running":
        return "All Systems Operational";
      case "stopped":
        return "Idle";
      case "not_installed":
        return "Setup Needed";
      case "unreachable":
        return "Daemon Down";
    }
  });

  const pillSubtitle = $derived.by(() => {
    switch (aggregate) {
      case "running":
        return "Everything looks good.";
      case "stopped":
        return "Some services aren't running.";
      case "not_installed":
        return "One or more tools need installing.";
      case "unreachable":
        return "A background daemon stopped responding.";
    }
  });

  // Meter values — null while metrics haven't ticked yet.
  const cpuPct = $derived.by<number | null>(() =>
    metrics.value ? Math.round(metrics.value.cpu.total) : null,
  );

  const memPct = $derived.by<number | null>(() => {
    if (!metrics.value) return null;
    const { usedBytes, totalBytes } = metrics.value.memory;
    if (totalBytes === 0) return null;
    return Math.round((usedBytes / totalBytes) * 100);
  });

  const memUsedGb = $derived.by<string | null>(() => {
    if (!metrics.value) return null;
    return (metrics.value.memory.usedBytes / 1024 ** 3).toFixed(1);
  });

  const diskPct = $derived.by<number | null>(() => {
    if (!metrics.value) return null;
    const { usedBytes, totalBytes } = metrics.value.disk;
    if (totalBytes === 0) return null;
    return Math.round((usedBytes / totalBytes) * 100);
  });

  const diskUsedGb = $derived.by<string | null>(() => {
    if (!metrics.value) return null;
    return Math.round(metrics.value.disk.usedBytes / 1024 ** 3).toString();
  });

  // The CPU / Memory / Disk meters are useful at a glance, so the footer
  // panel is expanded by default. Users can collapse it with the chevron
  // when they want a quieter sidebar.
  let footerOpen = $state<boolean>(true);

  function toggleFooter() {
    footerOpen = !footerOpen;
  }

  function openGithub() {
    void openUrl("https://github.com/portbay-app/portbay");
  }
</script>

<aside
  class="relative h-full min-h-0 flex flex-col bg-sidebar border-r border-border"
  aria-label="Primary navigation"
>
  <!-- Brand row — pt-9 reserves space for the macOS traffic lights -->
  <div
    data-tauri-drag-region
    class="shrink-0 pt-9 pb-4 select-none cursor-default flex items-center gap-3
           {collapsed ? 'px-0 flex-col gap-2' : 'px-4'}"
  >
    <span class="text-fg shrink-0">
      <LighthouseLogo size={collapsed ? 30 : 36} />
    </span>
    {#if !collapsed}
      <div class="min-w-0 leading-tight flex items-center gap-1.5">
        <div class="text-[15px] font-semibold tracking-tight">PortBay</div>
        {#if entitlements.isPro}
          <span
            class="shrink-0 px-1.5 py-0.5 rounded-md bg-accent text-on-accent
                   text-[10px] font-semibold leading-none tracking-wide"
          >
            Pro
          </span>
        {/if}
      </div>
    {/if}
    <!-- Collapse / expand toggle — flips the density preference, so it stays
         in sync with the Settings density control. -->
    <button
      type="button"
      onclick={() => density.toggle()}
      title={collapsed ? "Expand sidebar" : "Collapse sidebar"}
      aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
      aria-pressed={collapsed}
      class="shrink-0 p-1.5 rounded-md text-fg-subtle hover:text-fg hover:bg-surface-2
             transition-colors {collapsed ? '' : 'ml-auto'}"
    >
      <Icon name={collapsed ? "panel-left-open" : "panel-left-close"} size={15} />
    </button>
  </div>

  <!-- Nav -->
  <nav
    class="flex-1 min-h-0 overflow-y-auto space-y-0.5
           {collapsed ? 'px-1.5 py-1' : 'px-2 py-1'}"
  >
    <SidebarItem href="/" icon="home" label="Projects" {collapsed} />

    <!-- Groups submenu — collapsible cluster list. In compact density the
         header is hidden and each group renders as a centered status dot. -->
    <div class="pt-2">
      {#if !collapsed}
        <div class="flex items-center justify-between gap-1 px-2 py-1">
          <button
            type="button"
            onclick={() => (groupsOpen = !groupsOpen)}
            aria-expanded={groupsOpen}
            aria-controls="sidebar-groups-list"
            class="flex items-center gap-1.5 text-[11px] uppercase tracking-wide
                   text-fg-subtle hover:text-fg-muted transition-colors"
          >
            <Icon
              name={groupsOpen ? "chevron-down" : "chevron-right"}
              size={10}
            />
            Groups
            {#if groups.value.length > 0}
              <span class="text-fg-subtle font-mono">{groups.value.length}</span>
            {/if}
          </button>
          <button
            type="button"
            onclick={() => groupEditor.create()}
            title="New group"
            aria-label="New group"
            class="p-0.5 rounded text-fg-subtle hover:text-accent hover:bg-surface-2 transition-colors"
          >
            <Icon name="plus" size={11} />
          </button>
        </div>
      {/if}

      {#if collapsed || groupsOpen}
        <div id="sidebar-groups-list" class="space-y-0.5">
          {#if groups.value.length === 0}
            {#if !collapsed}
              <p class="px-2 py-1 text-[11px] text-fg-subtle">
                No groups yet. Cluster projects for one-click batch actions.
              </p>
            {/if}
          {:else}
            {#each groups.value as g (g.id)}
              {@const live = runningCount(g.knownIds)}
              <a
                href="/groups/{g.id}"
                title={collapsed
                  ? g.name
                  : `${g.memberCount} member${g.memberCount === 1 ? "" : "s"}`}
                aria-label={collapsed ? g.name : undefined}
                class="group flex items-center rounded-md text-sm
                       text-fg-muted hover:bg-surface-2 hover:text-fg transition-colors
                       {collapsed ? 'justify-center px-0 py-2' : 'gap-2 px-2 py-1.5'}"
              >
                <StatusDot
                  status={live > 0 ? "running" : "stopped"}
                  size="sm"
                />
                {#if !collapsed}
                  <span class="flex-1 min-w-0 truncate">{g.name}</span>
                  <span class="text-[10px] tabular-nums text-fg-subtle">
                    {live}/{g.memberCount}
                  </span>
                {/if}
              </a>
            {/each}
          {/if}
        </div>
      {/if}
    </div>

    <div class="pt-2 space-y-0.5">
      <SidebarItem href="/domains" icon="link" label="Domains" matchPrefix {collapsed} />
      <SidebarItem href="/dns" icon="globe" label="DNS" matchPrefix {collapsed} />
      <SidebarItem href="/services" icon="server" label="Services" matchPrefix {collapsed} />
      <SidebarItem
        href="/web-servers"
        icon="server-cog"
        label="Web Server"
        matchPrefix
        {collapsed}
      />
      <SidebarItem
        href="/certificates"
        icon="shield"
        label="Certificates"
        matchPrefix
        {collapsed}
      />
      <SidebarItem
        href="/sandbox"
        icon="package"
        label="Sandbox"
        matchPrefix
        {collapsed}
      />
      <SidebarItem href="/logs" icon="file-text" label="Logs" matchPrefix {collapsed} />
      <SidebarItem
        href="/inspector"
        icon="activity"
        label="Inspector"
        matchPrefix
        {collapsed}
      />
      <SidebarItem
        href="/languages"
        icon="file-code"
        label="Languages"
        matchPrefix
        {collapsed}
      />
      <SidebarItem
        href="/databases"
        icon="database"
        label="Databases"
        matchPrefix
        badge={runningDbCount}
        {collapsed}
      />
      <SidebarItem href="/tunnels" icon="cloud" label="Tunnels" matchPrefix {collapsed} />
      <SidebarItem
        href="/settings"
        icon="settings"
        label="Settings"
        matchPrefix
        {collapsed}
      />
    </div>
  </nav>

  <!-- System footer — health pill, meters, version row -->
  <div class="shrink-0 border-t border-border">
    <!-- Health pill / expander. Collapsed density shows only the status dot
         (the title + meters need horizontal room the strip doesn't have). -->
    {#if collapsed}
      <div
        class="flex items-center justify-center px-1.5 py-3"
        title="{pillTitle} — {pillSubtitle}"
      >
        <StatusDot status={pillStatus} size="md" />
      </div>
    {:else}
      <button
        type="button"
        onclick={toggleFooter}
        class="w-full flex items-center justify-between gap-2 px-3 py-2.5
               text-left hover:bg-surface-2 transition-colors"
        aria-expanded={footerOpen}
        aria-controls="sidebar-system-meters"
      >
        <span class="flex items-center gap-2 min-w-0">
          <StatusDot status={pillStatus} size="md" />
          <span class="min-w-0 leading-tight">
            <span class="block text-[12px] font-medium text-fg truncate">
              {pillTitle}
            </span>
            <span class="block text-[10.5px] text-fg-subtle truncate">
              {pillSubtitle}
            </span>
          </span>
        </span>
        <Icon
          name={footerOpen ? "chevron-down" : "chevron-up"}
          size={12}
          class="text-fg-subtle shrink-0"
        />
      </button>

      {#if footerOpen}
        <div
          id="sidebar-system-meters"
          class="px-3 pb-3 pt-1 space-y-2 border-t border-border/60"
        >
        <!-- CPU -->
        <div class="space-y-1">
          <div class="flex items-baseline justify-between text-[11px]">
            <span class="text-fg-muted">CPU</span>
            <span class="font-mono tabular-nums text-fg">
              {cpuPct ?? "—"}{cpuPct !== null ? "%" : ""}
            </span>
          </div>
          <div class="h-1 rounded-full bg-surface-2 overflow-hidden">
            <div
              class="h-full bg-accent transition-[width] duration-500"
              style:width="{cpuPct ?? 0}%"
            ></div>
          </div>
        </div>

        <!-- Memory -->
        <div class="space-y-1">
          <div class="flex items-baseline justify-between text-[11px]">
            <span class="text-fg-muted">Memory</span>
            <span class="font-mono tabular-nums text-fg">
              {memUsedGb ?? "—"}{memUsedGb !== null ? " GB" : ""}
            </span>
          </div>
          <div class="h-1 rounded-full bg-surface-2 overflow-hidden">
            <div
              class="h-full bg-accent transition-[width] duration-500"
              style:width="{memPct ?? 0}%"
            ></div>
          </div>
        </div>

        <!-- Disk -->
        <div class="space-y-1">
          <div class="flex items-baseline justify-between text-[11px]">
            <span class="text-fg-muted">Disk</span>
            <span class="font-mono tabular-nums text-fg">
              {diskUsedGb ?? "—"}{diskUsedGb !== null ? " GB" : ""}
            </span>
          </div>
          <div class="h-1 rounded-full bg-surface-2 overflow-hidden">
            <div
              class="h-full bg-accent transition-[width] duration-500"
              style:width="{diskPct ?? 0}%"
            ></div>
          </div>
          </div>
        </div>
      {/if}
    {/if}

    <!-- Version + GitHub. Collapsed density drops the version text and
         centers the GitHub mark. -->
    <div
      class="flex items-center gap-2 px-3 py-2 border-t border-border/60
             {collapsed ? 'justify-center' : 'justify-between'}"
    >
      {#if !collapsed}
        <span class="text-[10.5px] font-mono text-fg-subtle">
          PortBay{appVersion ? ` ${appVersion}` : ""}
        </span>
      {/if}
      <button
        type="button"
        onclick={openGithub}
        title="Open on GitHub"
        aria-label="Open PortBay on GitHub"
        class="p-1 rounded-md text-fg-subtle hover:text-fg hover:bg-surface-2 transition-colors"
      >
        <!--
          Inline GitHub mark — lucide doesn't ship brand logos, so the
          octocat-style silhouette lives directly in the component.
        -->
        <svg
          width="14"
          height="14"
          viewBox="0 0 16 16"
          fill="currentColor"
          aria-hidden="true"
        >
          <path
            d="M8 0C3.58 0 0 3.58 0 8a8 8 0 0 0 5.47 7.59c.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.01 8.01 0 0 0 16 8c0-4.42-3.58-8-8-8z"
          />
        </svg>
      </button>
    </div>
  </div>

  {#if showHandle}
    <SidebarResizeHandle />
  {/if}
</aside>
