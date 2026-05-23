<!--
  Sidebar — left nav rail.

  Five entries: Projects (home), Services, Domains, Logs, Settings.
  Top padding (`pt-9`) reserves space for the macOS traffic lights when
  the window uses `titleBarStyle: "Overlay"` (set in tauri.conf.json).

  Footer holds a refresh button (no-op stub until the reconcile loop
  card lands) and a small status pill reading from the sidecars store
  (added in card #5).
-->
<script lang="ts">
  import SidebarItem from "./SidebarItem.svelte";
  import SidebarResizeHandle from "./SidebarResizeHandle.svelte";
  import Icon from "$lib/components/atoms/Icon.svelte";
  import { SidecarPill } from "$lib/components/sidecars";
  import { sidecars } from "$lib/stores/sidecars.svelte";
  import { density } from "$lib/stores/density.svelte";

  /** Hide the resize handle when density is `compact` — the layout
   *  forces sidebar width to its own clamp in that mode, so manual
   *  resize would have no visible effect. */
  const showHandle = $derived(density.value !== "compact");

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
    <SidebarItem href="/services" icon="server" label="Services" matchPrefix />
    <SidebarItem href="/domains" icon="link" label="Domains" matchPrefix />
    <SidebarItem href="/logs" icon="file-text" label="Logs" matchPrefix />
    <SidebarItem
      href="/settings"
      icon="settings"
      label="Settings"
      matchPrefix
    />
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
