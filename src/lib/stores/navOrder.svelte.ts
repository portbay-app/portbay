/**
 * Nav order — the reorderable middle block of the sidebar (AI → Settings),
 * persisted to localStorage.
 *
 * Only this block is user-arrangeable. The anchors above it (Projects, Tasks,
 * Groups) and the system footer below stay fixed, so the sidebar's structure
 * is stable while the destinations a user reaches most can float to the top.
 *
 * Visibility (pin/hide) is controlled from the Integrations page only: the
 * sidebar renders `visibleItems`, the hub renders `allItems` with toggles.
 * All parsing/merge rules live in `navOrderCore.ts` (pure, unit-tested);
 * this store owns the runes and the localStorage I/O.
 */
import { browser } from "$app/environment";
import {
  NAV_ITEMS,
  mergeVisibleOrder,
  parsePersisted,
  reconcile,
  serialize,
  type NavItem,
} from "./navOrderCore";

export { NAV_ITEMS } from "./navOrderCore";
export type { NavItem } from "./navOrderCore";

const STORAGE_KEY = "portbay.nav-order";

function loadInitial(): { items: NavItem[]; hidden: Set<string> } {
  if (!browser) return reconcile(null);
  return reconcile(parsePersisted(localStorage.getItem(STORAGE_KEY)));
}

function persist(items: NavItem[], hidden: Set<string>) {
  if (!browser) return;
  localStorage.setItem(STORAGE_KEY, serialize(items, hidden));
}

function createNavOrderStore() {
  const initial = loadInitial();
  let items = $state<NavItem[]>(initial.items);
  // Reassigned (never mutated) on change — plain Sets aren't deeply reactive.
  let hidden = $state<Set<string>>(initial.hidden);

  return {
    /** Full ordered list, hidden included (canonical order authority). */
    get items() {
      return items;
    },
    /** What the sidebar renders. */
    get visibleItems() {
      return items.filter((it) => !hidden.has(it.id));
    },
    /** What the Integrations hub renders — every item plus its pin state. */
    get allItems(): (NavItem & { hidden: boolean })[] {
      return items.map((it) => ({ ...it, hidden: hidden.has(it.id) }));
    },
    /** Commit a drag/keyboard reorder of the visible subset. */
    commit(nextVisible: NavItem[]) {
      items = mergeVisibleOrder(items, nextVisible, hidden);
      persist(items, hidden);
    },
    /** Commit a reorder of the FULL list (the Settings sidebar manager works
     *  on all items, hidden included). Ids are validated against the current
     *  set so a buggy caller can't drop or invent destinations. */
    commitAll(next: NavItem[]) {
      const currentIds = new Set(items.map((it) => it.id));
      const nextIds = new Set(next.map((it) => it.id));
      if (nextIds.size !== currentIds.size || ![...currentIds].every((id) => nextIds.has(id))) {
        return;
      }
      items = next;
      persist(items, hidden);
    },
    /** Pin (hide=false) or unpin (hide=true) one item. No-op on non-hideable items. */
    setHidden(id: string, hide: boolean) {
      const item = items.find((it) => it.id === id);
      if (!item || item.hideable === false) return;
      const next = new Set(hidden);
      if (hide) next.add(id);
      else next.delete(id);
      hidden = next;
      persist(items, hidden);
    },
    /** Restore the out-of-the-box order and visibility. */
    reset() {
      const fresh = reconcile(null);
      items = fresh.items;
      hidden = fresh.hidden;
      persist(items, hidden);
    },
  };
}

export const navOrder = createNavOrderStore();
