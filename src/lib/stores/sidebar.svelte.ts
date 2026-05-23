/**
 * Sidebar width — user-resizable, persisted to localStorage.
 *
 * The root layout reads `sidebar.width` and substitutes it into the
 * grid's first column. Compact density mode bypasses this entirely
 * (the sidebar collapses to its own narrower clamp inside the layout
 * derivation), so user-resize and density are independent.
 */
import { browser } from "$app/environment";

const STORAGE_KEY = "portbay.sidebar.width";

/** Default width — matches the previous hardcoded comfortable value. */
export const SIDEBAR_DEFAULT = 220;

/** Minimum width — below this the sidebar's labels start truncating awkwardly. */
export const SIDEBAR_MIN = 160;

/** Maximum width — above this the main panel gets cramped. */
export const SIDEBAR_MAX = 360;

/** Keyboard nudge step in px when the resize handle has focus. */
export const SIDEBAR_KEY_STEP = 8;

function clamp(n: number): number {
  return Math.min(SIDEBAR_MAX, Math.max(SIDEBAR_MIN, Math.round(n)));
}

function loadInitial(): number {
  if (!browser) return SIDEBAR_DEFAULT;
  const raw = localStorage.getItem(STORAGE_KEY);
  if (!raw) return SIDEBAR_DEFAULT;
  const n = Number(raw);
  return Number.isFinite(n) ? clamp(n) : SIDEBAR_DEFAULT;
}

function persist(value: number) {
  if (!browser) return;
  localStorage.setItem(STORAGE_KEY, String(value));
}

function createSidebarStore() {
  let width = $state<number>(loadInitial());
  /** True while a drag is in progress; the layout uses it to disable
   *  transitions so the sidebar tracks the pointer 1:1. */
  let dragging = $state<boolean>(false);

  return {
    get width() {
      return width;
    },
    get dragging() {
      return dragging;
    },
    set(next: number) {
      width = clamp(next);
    },
    /** Persisted commit — call on `pointerup` so we don't write every frame. */
    commit() {
      persist(width);
    },
    nudge(delta: number) {
      width = clamp(width + delta);
      persist(width);
    },
    reset() {
      width = SIDEBAR_DEFAULT;
      persist(width);
    },
    beginDrag() {
      dragging = true;
    },
    endDrag() {
      dragging = false;
      persist(width);
    },
  };
}

export const sidebar = createSidebarStore();
