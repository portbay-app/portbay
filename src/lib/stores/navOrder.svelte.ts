/**
 * Nav order — the reorderable middle block of the sidebar (AI → Settings),
 * persisted to localStorage.
 *
 * Only this block is user-arrangeable. The anchors above it (Projects, Tasks,
 * Groups) and the system footer below stay fixed, so the sidebar's structure
 * is stable while the destinations a user reaches most can float to the top.
 *
 * Persistence stores just the ordered list of item ids. On load we reconcile
 * that against the canonical `NAV_ITEMS` so a saved order survives a release
 * that adds, removes, or renames a destination: unknown ids are dropped, and
 * any newly-shipped item that isn't in the saved order is appended in its
 * canonical position rather than silently vanishing.
 */
import { browser } from "$app/environment";
import type { IconName } from "$lib/components/atoms/Icon.svelte";

const STORAGE_KEY = "portbay.nav-order";

export interface NavItem {
  /** Stable id — the persistence key and svelte-dnd-action item id. */
  id: string;
  href: string;
  icon: IconName;
  label: string;
  /** Light the item active when the path *starts with* href (sub-routes). */
  matchPrefix?: boolean;
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
  { id: "settings", href: "/settings", icon: "settings", label: "Settings", matchPrefix: true },
];

/** Merge a saved id order with the canonical list (see file header). */
function reconcile(savedIds: string[]): NavItem[] {
  const byId = new Map(NAV_ITEMS.map((it) => [it.id, it]));
  const seen = new Set<string>();
  const out: NavItem[] = [];
  for (const id of savedIds) {
    const it = byId.get(id);
    if (it && !seen.has(id)) {
      out.push(it);
      seen.add(id);
    }
  }
  for (const it of NAV_ITEMS) {
    if (!seen.has(it.id)) {
      out.push(it);
      seen.add(it.id);
    }
  }
  return out;
}

function loadInitial(): NavItem[] {
  if (!browser) return [...NAV_ITEMS];
  const raw = localStorage.getItem(STORAGE_KEY);
  if (!raw) return [...NAV_ITEMS];
  try {
    const parsed = JSON.parse(raw);
    if (Array.isArray(parsed)) {
      return reconcile(parsed.filter((k): k is string => typeof k === "string"));
    }
  } catch {
    // Corrupt value — fall through to the default order.
  }
  return [...NAV_ITEMS];
}

function persist(items: NavItem[]) {
  if (!browser) return;
  localStorage.setItem(STORAGE_KEY, JSON.stringify(items.map((it) => it.id)));
}

function createNavOrderStore() {
  let items = $state<NavItem[]>(loadInitial());

  return {
    get items() {
      return items;
    },
    /** Commit the order on drop. */
    commit(next: NavItem[]) {
      items = next;
      persist(items);
    },
    /** Restore the out-of-the-box order. */
    reset() {
      items = [...NAV_ITEMS];
      persist(items);
    },
  };
}

export const navOrder = createNavOrderStore();
