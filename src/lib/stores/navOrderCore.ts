/**
 * Nav-order core — the pure logic behind the sidebar's reorderable block:
 * the canonical item catalog, persisted-value parsing/migration, and the
 * order/visibility reconciliation rules.
 *
 * Deliberately rune- and SvelteKit-free so `pnpm test` (node environment,
 * no Svelte compiler) can exercise every rule directly; the
 * `navOrder.svelte.ts` store wraps this with `$state` + localStorage.
 *
 * Persistence shapes (one localStorage key, upgraded in place):
 *   v1 — `string[]`                      ordered item ids, everything visible
 *   v2 — `{ order: string[], hidden: string[] }`
 * A v1 value must keep loading forever: profiles in the wild hold it.
 */
import type { IconName } from "$lib/components/atoms/Icon.svelte";

export interface NavItem {
  /** Stable id — the persistence key and svelte-dnd-action item id. */
  id: string;
  href: string;
  icon: IconName;
  label: string;
  /** Light the item active when the path *starts with* href (sub-routes). */
  matchPrefix?: boolean;
  /**
   * False pins the item into the sidebar permanently. Settings (the escape
   * hatch) and Integrations (the only place an item can be re-pinned from)
   * must never be hideable — hiding either could strand the user.
   */
  hideable?: boolean;
  /**
   * Ship hub-only: start hidden until the user pins it from the Integrations
   * page. Only consulted when the item is missing from a saved order (i.e.
   * the save predates the item) or on a fresh profile — an explicit user
   * choice always wins.
   */
  defaultHidden?: boolean;
}

/**
 * Canonical, default-ordered list of the reorderable destinations. The order
 * here is the out-of-the-box order and the fallback whenever no (or a corrupt)
 * preference exists.
 */
export const NAV_ITEMS: NavItem[] = [
  { id: "ai", href: "/ai", icon: "bot", label: "AI", matchPrefix: true },
  { id: "domains", href: "/domains", icon: "link", label: "Domains", matchPrefix: true },
  { id: "dns", href: "/dns", icon: "globe", label: "DNS", matchPrefix: true },
  { id: "services", href: "/services", icon: "server", label: "Services", matchPrefix: true },
  { id: "web-servers", href: "/web-servers", icon: "server-cog", label: "Web Server", matchPrefix: true },
  { id: "certificates", href: "/certificates", icon: "shield", label: "Certificates", matchPrefix: true },
  { id: "sandbox", href: "/sandbox", icon: "package", label: "Sandbox", matchPrefix: true },
  { id: "logs", href: "/logs", icon: "file-text", label: "Logs", matchPrefix: true },
  { id: "inspector", href: "/inspector", icon: "activity", label: "Inspector", matchPrefix: true },
  { id: "languages", href: "/languages", icon: "file-code", label: "Languages", matchPrefix: true },
  { id: "databases", href: "/databases", icon: "database", label: "Databases", matchPrefix: true },
  { id: "ssh", href: "/ssh", icon: "terminal", label: "SSH", matchPrefix: true },
  { id: "tunnels", href: "/tunnels", icon: "cloud", label: "Tunnels", matchPrefix: true },
  { id: "integrations", href: "/integrations", icon: "grid-2x2", label: "Integrations", matchPrefix: true, hideable: false },
  { id: "settings", href: "/settings", icon: "settings", label: "Settings", matchPrefix: true, hideable: false },
];

export interface PersistedNav {
  order: string[];
  hidden: string[];
}

const stringsOnly = (v: unknown): string[] =>
  Array.isArray(v) ? v.filter((k): k is string => typeof k === "string") : [];

/** Parse a raw localStorage value into the v2 shape; null on anything unusable. */
export function parsePersisted(raw: string | null): PersistedNav | null {
  if (!raw) return null;
  try {
    const parsed: unknown = JSON.parse(raw);
    if (Array.isArray(parsed)) {
      // v1 — order only; visibility didn't exist yet, so nothing was hidden.
      return { order: stringsOnly(parsed), hidden: [] };
    }
    if (parsed && typeof parsed === "object" && Array.isArray((parsed as PersistedNav).order)) {
      const o = parsed as Record<string, unknown>;
      return { order: stringsOnly(o.order), hidden: stringsOnly(o.hidden) };
    }
  } catch {
    // Corrupt value — fall through to the defaults.
  }
  return null;
}

/**
 * Merge a saved value with the canonical catalog:
 *  - unknown / duplicate ids in the saved order are dropped;
 *  - newly-shipped canonical items are appended in canonical relative order
 *    rather than silently vanishing;
 *  - hidden ids are honored only for known, hideable items;
 *  - a canonical item the save has never seen (or a fresh profile) inherits
 *    its `defaultHidden`.
 */
export function reconcile(
  saved: PersistedNav | null,
  catalog: NavItem[] = NAV_ITEMS,
): { items: NavItem[]; hidden: Set<string> } {
  const byId = new Map(catalog.map((it) => [it.id, it]));
  const seen = new Set<string>();
  const items: NavItem[] = [];
  for (const id of saved?.order ?? []) {
    const it = byId.get(id);
    if (it && !seen.has(id)) {
      items.push(it);
      seen.add(id);
    }
  }
  for (const it of catalog) {
    if (!seen.has(it.id)) {
      items.push(it);
      seen.add(it.id);
    }
  }

  const savedOrder = new Set(saved?.order ?? []);
  const hidden = new Set<string>();
  for (const it of catalog) {
    if (it.hideable === false) continue;
    if (saved?.hidden.includes(it.id)) hidden.add(it.id);
    // The save predates this item (or there is no save): apply its default.
    else if (it.defaultHidden && !savedOrder.has(it.id)) hidden.add(it.id);
  }
  return { items, hidden };
}

/**
 * Fold a reorder of the *visible* subset back into the full order: hidden
 * items keep their slots, visible slots are refilled in the new relative
 * order. This is what lets the drag list operate on visible items only while
 * a later re-pin still surfaces the item where the user last had it.
 */
export function mergeVisibleOrder(
  current: NavItem[],
  nextVisible: NavItem[],
  hidden: Set<string>,
): NavItem[] {
  const known = new Set(current.map((it) => it.id));
  const queue = nextVisible.filter((it) => known.has(it.id) && !hidden.has(it.id));
  return current.map((it) => {
    if (hidden.has(it.id)) return it;
    return queue.shift() ?? it;
  });
}

/** v2 persistence payload for the current state. */
export function serialize(items: NavItem[], hidden: Set<string>): string {
  return JSON.stringify({
    order: items.map((it) => it.id),
    hidden: items.filter((it) => hidden.has(it.id)).map((it) => it.id),
  } satisfies PersistedNav);
}
