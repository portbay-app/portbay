/**
 * Command palette store — open/close + query + recency tracking.
 *
 * Recents are keyed by command id and persisted to localStorage so
 * "the action you ran yesterday" stays at the top of the empty-query
 * list across launches. We keep the last 12; FIFO eviction.
 */
import { browser } from "$app/environment";

const RECENT_KEY = "portbay.palette.recent";
const RECENT_CAP = 12;

function loadRecents(): string[] {
  if (!browser) return [];
  try {
    const raw = localStorage.getItem(RECENT_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed)
      ? parsed.filter((v): v is string => typeof v === "string")
      : [];
  } catch {
    return [];
  }
}

function persistRecents(ids: string[]) {
  if (!browser) return;
  try {
    localStorage.setItem(RECENT_KEY, JSON.stringify(ids));
  } catch {
    /* localStorage may be locked in private modes — ignore */
  }
}

function createPaletteStore() {
  let open = $state<boolean>(false);
  let query = $state<string>("");
  let recents = $state<string[]>(loadRecents());

  function show() {
    query = "";
    open = true;
  }

  function hide() {
    open = false;
  }

  function setQuery(next: string) {
    query = next;
  }

  /** Mark a command id as the most recently used. Idempotent — the id
   *  bubbles to the head of the list either way. */
  function markUsed(id: string) {
    const filtered = recents.filter((r) => r !== id);
    recents = [id, ...filtered].slice(0, RECENT_CAP);
    persistRecents(recents);
  }

  return {
    get isOpen() {
      return open;
    },
    get query() {
      return query;
    },
    get recents() {
      return recents;
    },
    show,
    hide,
    setQuery,
    markUsed,
  };
}

export const palette = createPaletteStore();
