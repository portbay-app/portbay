/**
 * Density preference — `comfortable` (Vince default) or `compact` (Sam).
 *
 * Persisted to localStorage and pushed onto `<body data-density="…">` so
 * CSS can pick it up via attribute selectors. Components opt in by reading
 * `var(--density-card-py)` etc. — see `src/app.css`.
 *
 * Uses Svelte 5 runes; consumers import `density.value` to read and
 * `density.toggle()` or `density.set(...)` to mutate.
 */
import { browser } from "$app/environment";

export type Density = "comfortable" | "compact";

const STORAGE_KEY = "portbay.density";
const DEFAULT_DENSITY: Density = "comfortable";

function loadInitial(): Density {
  if (!browser) return DEFAULT_DENSITY;
  const v = localStorage.getItem(STORAGE_KEY);
  return v === "compact" || v === "comfortable" ? v : DEFAULT_DENSITY;
}

function apply(value: Density) {
  if (!browser) return;
  document.body.setAttribute("data-density", value);
  localStorage.setItem(STORAGE_KEY, value);
}

function createDensityStore() {
  let current = $state<Density>(loadInitial());
  if (browser) apply(current);

  return {
    get value() {
      return current;
    },
    set(next: Density) {
      current = next;
      apply(current);
    },
    toggle() {
      current = current === "comfortable" ? "compact" : "comfortable";
      apply(current);
    },
  };
}

export const density = createDensityStore();
