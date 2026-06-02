<!--
  IdeStatusBar — the slim VS Code-style status strip across the bottom of the
  host workspace: connection state on the left, port-forward count + a panel
  toggle on the right.
-->
<script lang="ts">
  import Icon from "$lib/components/atoms/Icon.svelte";

  interface Props {
    hostName: string;
    dest: string;
    port: number;
    connected: boolean;
    healthLabel: string;
    healthDotClass: string;
    tunnelCount: number;
    panelVisible: boolean;
    onTogglePanel: () => void;
  }
  let {
    hostName,
    dest,
    port,
    connected,
    healthLabel,
    healthDotClass,
    tunnelCount,
    panelVisible,
    onTogglePanel,
  }: Props = $props();
</script>

<footer
  class="flex h-6 shrink-0 items-center gap-3 border-t border-border/60 bg-surface/40 px-3
         text-[11px] text-fg-subtle"
>
  <span class="inline-flex items-center gap-1.5">
    {#if connected}
      <span class="h-1.5 w-1.5 rounded-full bg-status-running"></span>
      <span class="text-status-running">Connected</span>
    {:else}
      <span class="h-1.5 w-1.5 rounded-full {healthDotClass}"></span>
      <span>{healthLabel}</span>
    {/if}
  </span>

  <span class="truncate font-mono">{hostName}</span>
  <span class="truncate font-mono text-fg-subtle">{dest}:{port}</span>

  <div class="ml-auto flex items-center gap-3">
    {#if tunnelCount > 0}
      <span class="inline-flex items-center gap-1">
        <Icon name="link" size={12} />
        {tunnelCount}
      </span>
    {/if}
    <button
      type="button"
      onclick={onTogglePanel}
      title="Toggle panel (Ctrl+`)"
      aria-label="Toggle panel"
      class="inline-flex items-center gap-1 rounded px-1.5 py-0.5 hover:bg-surface-2 hover:text-fg
        {panelVisible ? 'text-fg' : ''}"
    >
      <Icon name="terminal" size={12} />
    </button>
  </div>
</footer>
