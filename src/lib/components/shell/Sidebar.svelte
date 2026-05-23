<!--
  Sidebar — left nav rail.

  Five entries: Projects (home), Services, Domains, Logs, Settings.
  Top padding (`pt-9`) reserves space for the macOS traffic lights when
  the window uses `titleBarStyle: "Overlay"` (set in tauri.conf.json).

  Footer holds a sidecar-status pill (reads from the sidecars store)
  and a refresh button that polls all sidecar status in one shot.
  A 4 px resize handle on the right edge lives in SidebarResizeHandle.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { page } from "$app/state";

  import SidebarItem from "./SidebarItem.svelte";
  import SidebarResizeHandle from "./SidebarResizeHandle.svelte";
  import Icon from "$lib/components/atoms/Icon.svelte";
  import StatusDot from "$lib/components/atoms/StatusDot.svelte";
  import { SidecarPill } from "$lib/components/sidecars";
  import { sidecars } from "$lib/stores/sidecars.svelte";
  import { density } from "$lib/stores/density.svelte";
  import { groups } from "$lib/stores/groups.svelte";
  import { groupEditor } from "$lib/stores/groupEditor.svelte";
  import { projects } from "$lib/stores/projects.svelte";

  /** Hide the resize handle when density is `compact` — the layout
   *  forces sidebar width to its own clamp in that mode, so manual
   *  resize would have no visible effect. */
  const showHandle = $derived(density.value !== "compact");

  let groupsOpen = $state<boolean>(true);

  onMount(() => {
    void groups.refresh();
  });

  /** Per-group derived state: how many members are currently running. */
  function runningCount(memberIds: string[]): number {
    const running = new Set(
      projects.value.filter((p) => p.status === "running").map((p) => p.id),
    );
    return memberIds.filter((id) => running.has(id)).length;
  }

  async function refresh() {
    await sidecars.refresh();
  }
</script>

<aside
  class="relative h-full flex flex-col bg-surface border-r border-border"
  aria-label="Primary navigation"
>
  <!-- Brand row — pt-9 leaves room for macOS traffic lights -->
  <div
    data-tauri-drag-region
    class="shrink-0 pt-9 pb-3 px-4 select-none cursor-default"
  >
    <div class="flex items-baseline gap-1.5">
      <span class="text-base font-semibold tracking-tight">PortBay</span>
      <span class="text-[10px] font-mono text-fg-subtle">v0.1.0</span>
    </div>
  </div>

  <!-- Nav -->
  <nav class="flex-1 min-h-0 px-2 py-1 overflow-y-auto space-y-0.5">
    <SidebarItem href="/" icon="home" label="Projects" />

    <!-- Groups submenu — collapsible cluster list. -->
    <div class="pt-1">
      <div class="flex items-center justify-between gap-1 px-2 py-1">
        <button
          type="button"
          onclick={() => (groupsOpen = !groupsOpen)}
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

      {#if groupsOpen}
        {#if groups.value.length === 0}
          <p class="px-2 py-1 text-[11px] text-fg-subtle">
            No groups yet. Cluster projects together for one-click batch
            actions.
          </p>
        {:else}
          {#each groups.value as g (g.id)}
            {@const live = runningCount(g.knownIds)}
            {@const isActive = page.url.pathname === `/groups/${g.id}`}
            <a
              href="/groups/{g.id}"
              class="group flex items-center gap-2 px-2 py-1.5 rounded-md text-sm
                     transition-colors"
              class:bg-accent={isActive}
              class:text-on-accent={isActive}
              class:text-fg-muted={!isActive}
              class:hover:bg-surface-2={!isActive}
              class:hover:text-fg={!isActive}
              title="{g.memberCount} member{g.memberCount === 1 ? '' : 's'}"
            >
              <StatusDot
                status={live > 0 ? "running" : "stopped"}
                size="sm"
              />
              <span class="flex-1 min-w-0 truncate">{g.name}</span>
              <span
                class="text-[10px] tabular-nums"
                class:text-on-accent={isActive}
                class:text-fg-subtle={!isActive}
              >
                {live}/{g.memberCount}
              </span>
            </a>
          {/each}
        {/if}
      {/if}
    </div>

    <div class="pt-1 space-y-0.5">
      <SidebarItem href="/services" icon="server" label="Services" matchPrefix />
      <SidebarItem href="/domains" icon="link" label="Domains" matchPrefix />
      <SidebarItem href="/logs" icon="file-text" label="Logs" matchPrefix />
      <SidebarItem
        href="/settings"
        icon="settings"
        label="Settings"
        matchPrefix
      />
    </div>
  </nav>

  <!-- Footer: refresh + (future) overall daemon health -->
  <div
    class="shrink-0 border-t border-border px-3 py-2.5 flex items-center justify-between"
  >
    <SidecarPill />
    <button
      type="button"
      onclick={refresh}
      title="Refresh sidecar status"
      aria-label="Refresh sidecar status"
      class="p-1 rounded-md text-fg-subtle hover:text-fg hover:bg-surface-2 transition-colors"
      class:animate-spin={sidecars.loading}
    >
      <Icon name="refresh-cw" size={14} />
    </button>
  </div>

  {#if showHandle}
    <SidebarResizeHandle />
  {/if}
</aside>
