<!--
  IdeActivityBar — the narrow VS Code-style icon rail on the far left of the host
  workspace. The Home icon at the top surfaces the Welcome/host-overview in the
  editor area. Below it, each icon selects a primary-sidebar view (Explorer /
  Deploy / Tunnels / SFTP); the active one shows a left accent bar. Terminal
  (bottom panel) and Agent (right aux panel) sit directly under SFTP. A settings
  gear is pinned to the bottom. Clicking the active view icon toggles the sidebar.
-->
<script lang="ts">
  import Icon from "$lib/components/atoms/Icon.svelte";
  import type { IconName } from "$lib/components/atoms/Icon.svelte";
  import type { ActivityView } from "$lib/stores/ideLayout.svelte";

  interface Props {
    activeView: ActivityView;
    sidebarVisible: boolean;
    agentVisible: boolean;
    /** The editor area is showing the Welcome tab (no file open). */
    homeActive: boolean;
    /** The bottom panel is open on the Terminal tab. */
    terminalActive: boolean;
    tunnelCount?: number;
    onSelect: (view: ActivityView) => void;
    onHome: () => void;
    onToggleTerminal: () => void;
    onToggleAgent: () => void;
    onSettings: () => void;
  }
  let {
    activeView,
    sidebarVisible,
    agentVisible,
    homeActive,
    terminalActive,
    tunnelCount = 0,
    onSelect,
    onHome,
    onToggleTerminal,
    onToggleAgent,
    onSettings,
  }: Props = $props();

  const ITEMS: { id: ActivityView; label: string; icon: IconName }[] = [
    { id: "explorer", label: "Explorer", icon: "folder" },
    { id: "deploy", label: "Deploy", icon: "rocket" },
    { id: "tunnels", label: "Tunnels", icon: "link" },
    { id: "sftp", label: "SFTP transfers", icon: "inbox" },
  ];
</script>

<nav
  class="flex w-12 shrink-0 flex-col items-center border-r border-border/60 bg-surface/40 py-2"
  aria-label="Workspace views"
>
  <button
    type="button"
    onclick={onHome}
    title="Home"
    aria-label="Home"
    aria-current={homeActive ? "page" : undefined}
    class="relative grid h-11 w-11 place-items-center rounded-md transition-colors
      {homeActive ? 'text-fg' : 'text-fg-subtle hover:text-fg hover:bg-surface-2'}"
  >
    {#if homeActive}
      <span class="absolute left-0 top-1/2 h-5 w-0.5 -translate-y-1/2 rounded-full bg-accent"></span>
    {/if}
    <Icon name="home" size={20} />
  </button>

  {#each ITEMS as item (item.id)}
    {@const active = activeView === item.id && sidebarVisible}
    <button
      type="button"
      onclick={() => onSelect(item.id)}
      title={item.label}
      aria-label={item.label}
      aria-current={active ? "page" : undefined}
      class="relative grid h-11 w-11 place-items-center rounded-md transition-colors
        {active ? 'text-fg' : 'text-fg-subtle hover:text-fg hover:bg-surface-2'}"
    >
      {#if active}
        <span class="absolute left-0 top-1/2 h-5 w-0.5 -translate-y-1/2 rounded-full bg-accent"></span>
      {/if}
      <Icon name={item.icon} size={20} />
      {#if item.id === "tunnels" && tunnelCount > 0}
        <span
          class="absolute right-1 top-1 grid min-h-4 min-w-4 place-items-center rounded-full
                 bg-accent px-1 text-[9px] font-semibold tabular-nums text-on-accent"
        >
          {tunnelCount}
        </span>
      {/if}
    </button>
  {/each}

  <!-- Terminal + Agent sit directly under the view icons. -->
  <button
    type="button"
    onclick={onToggleTerminal}
    title="Terminal (bottom panel)"
    aria-label="Terminal"
    aria-current={terminalActive ? "page" : undefined}
    class="relative grid h-11 w-11 place-items-center rounded-md transition-colors
      {terminalActive ? 'text-fg' : 'text-fg-subtle hover:text-fg hover:bg-surface-2'}"
  >
    {#if terminalActive}
      <span class="absolute left-0 top-1/2 h-5 w-0.5 -translate-y-1/2 rounded-full bg-accent"></span>
    {/if}
    <Icon name="terminal" size={20} />
  </button>
  <button
    type="button"
    onclick={onToggleAgent}
    title="Agent (right panel)"
    aria-label="Agent"
    aria-current={agentVisible ? "page" : undefined}
    class="relative grid h-11 w-11 place-items-center rounded-md transition-colors
      {agentVisible ? 'text-fg' : 'text-fg-subtle hover:text-fg hover:bg-surface-2'}"
  >
    {#if agentVisible}
      <span class="absolute right-0 top-1/2 h-5 w-0.5 -translate-y-1/2 rounded-full bg-accent"></span>
    {/if}
    <Icon name="bot" size={20} />
  </button>

  <div class="mt-auto flex flex-col items-center">
    <button
      type="button"
      onclick={onSettings}
      title="Host settings"
      aria-label="Host settings"
      class="grid h-11 w-11 place-items-center rounded-md text-fg-subtle hover:bg-surface-2 hover:text-fg"
    >
      <Icon name="settings" size={20} />
    </button>
  </div>
</nav>
