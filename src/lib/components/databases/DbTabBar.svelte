<script lang="ts">
  /**
   * DbTabBar — horizontal tab strip for the database IDE workspace.
   * Renders visibleTabs for the active instance; each tab has an icon, title,
   * and a close button (for closable tabs). Active tab shows an accent underline.
   * A "+" popover at the end lets the user open a New Query or ERD.
   */
  import Icon from "$lib/components/atoms/Icon.svelte";
  import type { IconName } from "$lib/components/atoms/Icon.svelte";
  import { dbWorkspace } from "$lib/stores/dbWorkspace.svelte";
  import type { DbTabKind } from "$lib/stores/dbWorkspace.svelte";

  const KIND_ICON: Record<DbTabKind, IconName> = {
    overview: "info",
    table: "database",
    query: "terminal",
    erd: "share",
    explain: "activity",
    build: "grid-2x2",
  };

  let plusOpen = $state(false);
  let menuEl = $state<HTMLDivElement | null>(null);

  // Close on any click that lands outside the +/menu container. We must NOT
  // rely on stopPropagation() in the button handler: Svelte 5 delegates `onclick`
  // to a root listener while `svelte:window` binds natively to `window` (above
  // the root), so stopPropagation there doesn't keep the native click from
  // reaching window — the menu would open and close on the same click. Checking
  // containment is robust regardless of delegation.
  function onWindowClick(e: MouseEvent) {
    if (!plusOpen) return;
    const target = e.target as Node | null;
    if (menuEl && target && menuEl.contains(target)) return;
    plusOpen = false;
  }
</script>

<svelte:window onclick={onWindowClick} />

<div class="shrink-0 flex items-stretch border-b border-border/60 bg-surface/60">
  <!-- Only the tab strip scrolls. The "+" menu lives outside it (below), so its
       dropdown popover isn't clipped by this container's overflow — setting
       overflow-x also clips the Y axis, which otherwise hid the menu. -->
  <div
    class="flex items-stretch overflow-x-auto scrollbar-none min-w-0"
    role="tablist"
    aria-label="Database workspace tabs"
  >
    {#each dbWorkspace.visibleTabs as tab (tab.id)}
    {@const isActive = tab.id === dbWorkspace.activeTabId}
    <div class="relative flex items-stretch shrink-0">
      <button
        type="button"
        role="tab"
        aria-selected={isActive}
        onclick={() => dbWorkspace.focus(tab.id)}
        class="relative flex items-center gap-1.5 h-10 pl-3
               {tab.closable ? 'pr-1.5' : 'pr-3'}
               text-[12px] transition-colors whitespace-nowrap
               {isActive
          ? 'text-fg font-medium'
          : 'text-fg-muted hover:text-fg hover:bg-surface-2/60'}"
      >
        <Icon name={KIND_ICON[tab.kind]} size={12} class="shrink-0" />
        <span>{tab.title}</span>
        {#if isActive}
          <span
            class="absolute left-2 right-2 -bottom-px h-[2px] rounded-full bg-accent"
          ></span>
        {/if}
      </button>
      {#if tab.closable}
        <button
          type="button"
          onclick={() => dbWorkspace.closeTab(tab.id)}
          title="Close tab"
          aria-label="Close {tab.title}"
          class="flex items-center justify-center w-5 h-10 px-0.5 text-fg-subtle/60
                 hover:text-fg hover:bg-surface-2/60 transition-colors"
        >
          <Icon name="x" size={10} />
        </button>
      {/if}
    </div>
    {/each}
  </div>

  <!-- Separator + "+" button — outside the scroll region so its dropdown
       popover renders below the strip without being clipped. -->
  <div
    bind:this={menuEl}
    class="flex items-center px-1 border-l border-border/40 ml-1 relative shrink-0"
  >
    <button
      type="button"
      onclick={() => (plusOpen = !plusOpen)}
      title="Open new tab"
      aria-label="Open new tab"
      aria-haspopup="true"
      aria-expanded={plusOpen}
      class="flex items-center justify-center w-7 h-7 rounded-md text-fg-subtle
             hover:text-fg hover:bg-surface-2 transition-colors"
    >
      <Icon name="plus" size={13} />
    </button>

    {#if plusOpen && dbWorkspace.activeInstanceId}
      {@const instanceId = dbWorkspace.activeInstanceId}
      <div
        role="presentation"
        onclick={(e) => e.stopPropagation()}
        onkeydown={(e) => e.stopPropagation()}
        class="absolute left-0 top-full mt-1 z-40 w-48 rounded-lg border border-border
               bg-surface shadow-2xl py-1"
      >
        <button
          type="button"
          onclick={() => {
            plusOpen = false;
            dbWorkspace.openQuery(instanceId);
          }}
          class="w-full flex items-center gap-2 px-3 py-2 text-[12px]
                 text-fg-muted hover:bg-surface-2 hover:text-fg transition-colors"
        >
          <Icon name="terminal" size={12} />
          New query
        </button>
        <button
          type="button"
          onclick={() => {
            plusOpen = false;
            dbWorkspace.openBuilder(instanceId);
          }}
          class="w-full flex items-center gap-2 px-3 py-2 text-[12px]
                 text-fg-muted hover:bg-surface-2 hover:text-fg transition-colors"
        >
          <Icon name="grid-2x2" size={12} />
          Visual query builder
        </button>
        <button
          type="button"
          onclick={() => {
            plusOpen = false;
            dbWorkspace.openErd(instanceId);
          }}
          class="w-full flex items-center gap-2 px-3 py-2 text-[12px]
                 text-fg-muted hover:bg-surface-2 hover:text-fg transition-colors"
        >
          <Icon name="share" size={12} />
          Schema diagram (ERD)
        </button>
      </div>
    {/if}
  </div>

  <!-- Right-side spacer fills remaining width -->
  <div class="flex-1"></div>
</div>
