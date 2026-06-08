<!--
  SidebarNavList — the reorderable middle block of the sidebar (AI → Settings).

  Drag-to-reorder built on Svelte's native FLIP so the motion reads like a
  real-world reorderable list (Linear / iOS): the row you grab lifts into a
  floating clone that tracks the cursor, its original spot collapses into a
  soft drop-slot, and every other row *slips* into place with a spring-ish
  ease as the target index changes. Nothing reorders the data until you drop,
  and during the drag we only reorder a local working copy — so the keyed
  `#each` stays consistent with Svelte's renderer (no DOM tug-of-war).

  A short movement threshold separates a click (navigate) from a drag
  (reorder), so the rows stay ordinary nav links when you just tap them.
-->
<script lang="ts">
  import { flip } from "svelte/animate";
  import { cubicOut } from "svelte/easing";
  import SidebarItem from "./SidebarItem.svelte";
  import type { NavItem } from "$lib/stores/navOrder.svelte";

  interface Props {
    items: NavItem[];
    collapsed: boolean;
    runningDbCount: number;
    /** Commit the new order once a drag settles. */
    oncommit: (items: NavItem[]) => void;
  }
  let { items, collapsed, runningDbCount, oncommit }: Props = $props();

  // Pointer travel (px) before a press becomes a drag rather than a click.
  const DRAG_THRESHOLD = 4;
  // FLIP timing for the slip. Short + easeOut keeps it snappy, not floaty.
  const FLIP_MS = 200;
  // Distance from the scroll edge (px) where auto-scroll kicks in.
  const EDGE = 48;

  /** Working copy we reorder live during a drag. While idle it mirrors the
   *  store; we only resync from props when no drag is in flight so an
   *  in-progress reorder is never clobbered by an incoming prop update. */
  // Seed from the prop; the $effect below keeps it synced while idle. The
  // initial-capture warning is exactly the behaviour we want here.
  // svelte-ignore state_referenced_locally
  let work = $state<NavItem[]>([...items]);
  let dragId = $state<string | null>(null);
  $effect(() => {
    // Touch `items` so this re-runs on prop change; bail mid-drag.
    const next = items;
    if (dragId === null) work = [...next];
  });

  const draggedItem = $derived(work.find((i) => i.id === dragId) ?? null);

  // --- Floating clone geometry (viewport-fixed) -------------------------
  let active = $state(false); // past the threshold → clone is shown
  let cloneLeft = $state(0);
  let cloneWidth = $state(0);
  let cloneTop = $state(0);

  // --- Transient drag bookkeeping (not reactive) ------------------------
  let listEl: HTMLDivElement | null = null;
  let scroller: HTMLElement | null = null;
  let startX = 0;
  let startY = 0;
  let grabOffsetY = 0; // pointerY − rowTop, keeps the clone under the grab point
  let pendingItem: NavItem | null = null;
  let pendingRow: HTMLElement | null = null;
  let justDragged = false; // swallow the click that ends a drag
  let lastClientY = 0;
  let autoRaf = 0;

  function findScroller(el: HTMLElement | null): HTMLElement | null {
    let n = el?.parentElement ?? null;
    while (n) {
      const s = getComputedStyle(n);
      if (/(auto|scroll)/.test(s.overflowY) && n.scrollHeight > n.clientHeight) {
        return n;
      }
      n = n.parentElement;
    }
    return null;
  }

  function onPointerDown(e: PointerEvent, item: NavItem) {
    if (e.button !== 0) return; // primary button only
    pendingItem = item;
    pendingRow = (e.currentTarget as HTMLElement) ?? null;
    startX = e.clientX;
    startY = e.clientY;
    window.addEventListener("pointermove", onPointerMove);
    window.addEventListener("pointerup", onPointerUp, { once: true });
  }

  function beginDrag(e: PointerEvent) {
    if (!pendingItem || !pendingRow) return;
    const rect = pendingRow.getBoundingClientRect();
    cloneLeft = rect.left;
    cloneWidth = rect.width;
    grabOffsetY = startY - rect.top;
    cloneTop = e.clientY - grabOffsetY;
    scroller = findScroller(listEl);
    dragId = pendingItem.id;
    active = true;
    justDragged = true;
    document.body.style.userSelect = "none";
    document.body.style.cursor = "grabbing";
  }

  function onPointerMove(e: PointerEvent) {
    if (!pendingItem) return;
    if (!active) {
      if (Math.hypot(e.clientX - startX, e.clientY - startY) < DRAG_THRESHOLD) {
        return;
      }
      beginDrag(e);
    }
    e.preventDefault();
    lastClientY = e.clientY;
    cloneTop = e.clientY - grabOffsetY;
    reorderToPointer(e.clientY);
    tickAutoScroll();
  }

  /** Move the dragged id to whichever slot the pointer is over. The reorder
   *  mutates `work`, and the keyed `#each` + `flip` animate every other row
   *  into its new position — that sliding is the "slip". */
  function reorderToPointer(clientY: number) {
    if (!listEl || dragId === null) return;
    const rows = Array.from(
      listEl.querySelectorAll<HTMLElement>("[data-reorder-row]"),
    );
    let target = rows.length - 1;
    for (let i = 0; i < rows.length; i++) {
      const r = rows[i].getBoundingClientRect();
      if (clientY < r.top + r.height / 2) {
        target = i;
        break;
      }
    }
    const from = work.findIndex((i) => i.id === dragId);
    if (from === -1 || from === target) return;
    const next = [...work];
    const [moved] = next.splice(from, 1);
    next.splice(target, 0, moved);
    work = next;
  }

  // Auto-scroll the nav when the clone nears the top/bottom edge, so long
  // lists can be reordered without letting go.
  function tickAutoScroll() {
    cancelAutoScroll();
    if (!scroller) return;
    const box = scroller.getBoundingClientRect();
    const topGap = lastClientY - box.top;
    const botGap = box.bottom - lastClientY;
    let dy = 0;
    if (topGap < EDGE) dy = -Math.ceil((EDGE - topGap) / 6);
    else if (botGap < EDGE) dy = Math.ceil((EDGE - botGap) / 6);
    if (dy === 0) return;
    const step = () => {
      if (!scroller || dragId === null) return;
      scroller.scrollTop += dy;
      reorderToPointer(lastClientY);
      autoRaf = requestAnimationFrame(step);
    };
    autoRaf = requestAnimationFrame(step);
  }
  function cancelAutoScroll() {
    if (autoRaf) cancelAnimationFrame(autoRaf);
    autoRaf = 0;
  }

  function onPointerUp() {
    window.removeEventListener("pointermove", onPointerMove);
    cancelAutoScroll();
    const didReorder = active;
    if (active) {
      active = false;
      document.body.style.userSelect = "";
      document.body.style.cursor = "";
      // Commit the settled order; the clone disappears and the dropped row
      // fades back in at its slot.
      oncommit([...work]);
    }
    dragId = null;
    pendingItem = null;
    pendingRow = null;
    // A drag ends with a synthetic click on the anchor — swallow it once so
    // we don't navigate to wherever we dropped.
    if (didReorder) {
      setTimeout(() => (justDragged = false), 0);
    } else {
      justDragged = false;
    }
  }

  function onRowClickCapture(e: MouseEvent) {
    if (justDragged) {
      e.preventDefault();
      e.stopPropagation();
    }
  }

  // --- Keyboard reorder (WCAG 2.1.1: a non-pointer path for the drag) -----
  // Alt+Arrow moves the focused row one slot; plain arrows are left to the
  // browser so ordinary link/landmark navigation is untouched. Focus rides the
  // moved row, and the change commits immediately (no separate "drop").
  let announce = $state("");

  function moveByKeyboard(item: NavItem, delta: number) {
    if (dragId !== null) return; // don't fight an in-flight pointer drag
    const from = work.findIndex((i) => i.id === item.id);
    if (from === -1) return;
    const to = from + delta;
    if (to < 0 || to >= work.length) return;
    const next = [...work];
    const [moved] = next.splice(from, 1);
    next.splice(to, 0, moved);
    work = next; // drives the FLIP slide
    oncommit([...next]);
    announce = `${item.label} moved to position ${to + 1} of ${next.length}`;
    // Keep focus on the row that moved once the DOM settles.
    queueMicrotask(() => {
      listEl
        ?.querySelector<HTMLElement>(
          `[data-reorder-row="${CSS.escape(item.id)}"] a`,
        )
        ?.focus();
    });
  }

  function onRowKeydown(e: KeyboardEvent, item: NavItem) {
    if (!e.altKey) return;
    if (e.key === "ArrowUp") {
      e.preventDefault();
      moveByKeyboard(item, -1);
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      moveByKeyboard(item, 1);
    }
  }
</script>

<div bind:this={listEl} class="pt-2 space-y-0.5" role="list">
  {#each work as item (item.id)}
    <!-- The keydown handler is the deliberate keyboard-reorder path (WCAG
         2.1.1); the row's actual focus target is the interactive <a> inside
         SidebarItem, so the listener on this wrapper is intentional. -->
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <div
      data-reorder-row={item.id}
      role="listitem"
      aria-keyshortcuts="Alt+ArrowUp Alt+ArrowDown"
      class="reorder-row {dragId === item.id ? 'is-source' : ''}"
      animate:flip={{ duration: FLIP_MS, easing: cubicOut }}
      onpointerdown={(e) => onPointerDown(e, item)}
      onclickcapture={onRowClickCapture}
      onkeydown={(e) => onRowKeydown(e, item)}
    >
      <SidebarItem
        href={item.href}
        icon={item.icon}
        label={item.label}
        matchPrefix={item.matchPrefix}
        badge={item.id === "databases" ? runningDbCount : null}
        {collapsed}
      />
    </div>
  {/each}
</div>

<!-- Screen-reader announcement for keyboard reorder moves. -->
<div class="sr-only" aria-live="polite" role="status">{announce}</div>

<!-- Lifted clone — viewport-fixed, follows the cursor, ignores pointer events. -->
{#if active && draggedItem}
  <div
    class="reorder-clone {collapsed ? 'is-collapsed' : ''}"
    style:left="{cloneLeft}px"
    style:top="{cloneTop}px"
    style:width="{cloneWidth}px"
    aria-hidden="true"
  >
    <SidebarItem
      href={draggedItem.href}
      icon={draggedItem.icon}
      label={draggedItem.label}
      matchPrefix={draggedItem.matchPrefix}
      badge={draggedItem.id === "databases" ? runningDbCount : null}
      {collapsed}
    />
  </div>
{/if}

<style>
  .reorder-row {
    cursor: grab;
    touch-action: pan-y;
  }

  /* The grabbed row's original slot: keep its height (so the layout doesn't
     jump) but hollow it into a soft, inset drop-target the other rows slip
     around. */
  .reorder-row.is-source {
    background: color-mix(in srgb, var(--color-accent, #2563eb) 10%, transparent);
    border-radius: 0.5rem;
    box-shadow: inset 0 0 0 1px
      color-mix(in srgb, var(--color-accent, #2563eb) 22%, transparent);
  }
  .reorder-row.is-source :global(a) {
    opacity: 0;
  }

  /* The carried copy — elevated with a shadow + a hair of scale so it reads as
     physically picked up. */
  .reorder-clone {
    position: fixed;
    z-index: 70;
    pointer-events: none;
    border-radius: 0.5rem;
    background: var(--color-surface-2, rgba(127, 127, 127, 0.12));
    box-shadow:
      0 1px 2px rgba(0, 0, 0, 0.12),
      0 12px 28px rgba(0, 0, 0, 0.22);
    transform: scale(1.03);
    transform-origin: center left;
    /* The clone tracks the pointer via top/left writes every move; a tiny
       transition on transform alone keeps the lift smooth without lagging the
       follow. */
    transition: transform 120ms cubic-bezier(0.2, 0.8, 0.2, 1);
  }
  .reorder-clone.is-collapsed {
    transform-origin: center;
  }
</style>
