<!--
  SidebarCustomizer — the sidebar pin manager (Settings → General).

  One ordered list mirroring the sidebar's reorderable block, all destinations
  included: drag the grip (svelte-dnd-action, same engine as the task board)
  to arrange, click the eye to pin/unpin. Hidden rows stay in the list, dimmed,
  holding their slot — so a later re-pin surfaces the item exactly where the
  user last kept it, and the list doubles as the recovery path.

  Everything applies instantly to the live sidebar (same store, no Save), which
  is the whole feedback loop: glance left, see the change. Settings and
  Integrations rows are locked visible — Settings is the escape hatch, this
  manager is the only un-hide surface, so neither may vanish.

  Keyboard path (WCAG 2.1.1): Alt+ArrowUp/Down on a row moves it one slot,
  with an aria-live announcement — mirrors the sidebar's own shortcut.
-->
<script lang="ts">
  import { flip } from "svelte/animate";
  import { cubicOut } from "svelte/easing";
  import { dndzone, type DndEvent } from "svelte-dnd-action";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import { navOrder } from "$lib/stores/navOrder.svelte";
  import type { NavItem } from "$lib/stores/navOrderCore";

  const FLIP_MS = 200;

  type Row = NavItem & { hidden: boolean };

  /** Working copy dndzone mutates during a drag; resyncs from the store while
   *  idle so pin toggles (which bypass the drag) are reflected immediately. */
  let rows = $state<Row[]>([...navOrder.allItems]);
  let dragging = $state(false);
  $effect(() => {
    const next = navOrder.allItems;
    if (!dragging) rows = [...next];
  });

  const shownCount = $derived(rows.filter((r) => !r.hidden).length);

  function onConsider(e: CustomEvent<DndEvent<Row>>) {
    dragging = true;
    rows = e.detail.items;
  }

  function onFinalize(e: CustomEvent<DndEvent<Row>>) {
    rows = e.detail.items;
    dragging = false;
    navOrder.commitAll(rows.map(({ hidden: _hidden, ...item }) => item));
  }

  // Screen-reader announcement for keyboard moves and pin flips.
  let announce = $state("");

  function moveByKeyboard(row: Row, delta: number) {
    if (dragging) return;
    const from = rows.findIndex((r) => r.id === row.id);
    const to = from + delta;
    if (from === -1 || to < 0 || to >= rows.length) return;
    const next = [...rows];
    const [moved] = next.splice(from, 1);
    next.splice(to, 0, moved);
    rows = next;
    navOrder.commitAll(next.map(({ hidden: _hidden, ...item }) => item));
    announce = `${row.label} moved to position ${to + 1} of ${next.length}`;
    queueMicrotask(() => {
      document
        .querySelector<HTMLElement>(`[data-sidebar-row="${CSS.escape(row.id)}"]`)
        ?.focus();
    });
  }

  function onRowKeydown(e: KeyboardEvent, row: Row) {
    if (!e.altKey) return;
    if (e.key === "ArrowUp") {
      e.preventDefault();
      moveByKeyboard(row, -1);
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      moveByKeyboard(row, 1);
    }
  }

  function togglePin(row: Row) {
    const next = !row.hidden;
    navOrder.setHidden(row.id, next);
    announce = `${row.label} ${next ? "hidden from" : "shown in"} sidebar`;
  }
</script>

<div class="space-y-2.5">
  <div class="flex items-center justify-between gap-3">
    <span class="text-[11.5px] text-fg-subtle tabular-nums">
      {shownCount} of {rows.length} in the sidebar · changes apply instantly
    </span>
    <button
      type="button"
      onclick={() => navOrder.reset()}
      class="text-[11.5px] text-fg-subtle hover:text-fg underline-offset-2 hover:underline transition-colors"
    >
      Reset to defaults
    </button>
  </div>

  <div
    class="rounded-lg border border-border bg-surface overflow-hidden"
    use:dndzone={{
      items: rows,
      flipDurationMs: FLIP_MS,
      dropTargetStyle: {},
      dragDisabled: false,
    }}
    onconsider={onConsider}
    onfinalize={onFinalize}
  >
    {#each rows as row (row.id)}
      <!-- The row itself is the keyboard-reorder focus target (Alt+Arrow,
           WCAG 2.1.1 — the non-pointer path for the drag), so the tabindex
           and keydown on this wrapper are deliberate; same pattern as the
           sidebar's own SidebarNavList. -->
      <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
      <div
        data-sidebar-row={row.id}
        role="listitem"
        tabindex="0"
        aria-keyshortcuts="Alt+ArrowUp Alt+ArrowDown"
        aria-label="{row.label}{row.hidden ? ', hidden' : ''}"
        onkeydown={(e) => onRowKeydown(e, row)}
        animate:flip={{ duration: FLIP_MS, easing: cubicOut }}
        class="group flex items-center gap-2.5 px-3 py-2 border-b border-border/60 last:border-b-0
               bg-surface focus-visible:outline-none focus-visible:bg-surface-2/60
               transition-opacity {row.hidden ? 'opacity-45' : ''}"
      >
        <span
          class="shrink-0 text-fg-subtle/60 group-hover:text-fg-subtle cursor-grab
                 active:cursor-grabbing transition-colors"
          title="Drag to reorder"
        >
          <Icon name="grip-vertical" size={13} />
        </span>
        <Icon name={row.icon} size={15} class="text-fg-muted shrink-0" />
        <span class="flex-1 min-w-0 text-[12.5px] text-fg truncate">{row.label}</span>
        {#if row.hideable === false}
          <span
            class="shrink-0 inline-flex items-center gap-1 text-[10.5px] text-fg-subtle"
            title="This destination can't be hidden"
          >
            <Icon name="lock" size={10} />
            Always shown
          </span>
        {:else}
          <button
            type="button"
            role="switch"
            aria-checked={!row.hidden}
            aria-label="Show {row.label} in sidebar"
            title={row.hidden ? "Show in sidebar" : "Hide from sidebar"}
            onclick={() => togglePin(row)}
            class="shrink-0 p-1.5 rounded-md transition-colors
                   {row.hidden
              ? 'text-fg-subtle hover:text-fg hover:bg-surface-2'
              : 'text-fg-muted hover:text-fg hover:bg-surface-2'}"
          >
            <Icon name={row.hidden ? "eye-off" : "eye"} size={14} />
          </button>
        {/if}
      </div>
    {/each}
  </div>
</div>

<!-- Live region for keyboard moves and pin flips. -->
<div class="sr-only" aria-live="polite" role="status">{announce}</div>
