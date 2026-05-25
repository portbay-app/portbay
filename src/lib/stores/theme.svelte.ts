/**
 * Theme preference — `dark` (default) or `light`.
 *
 * Persisted to localStorage and pushed onto `<body data-theme="…">` so
 * CSS tokens can switch without component-level theme branches.
 */
import { browser } from "$app/environment";

export type Theme = "dark" | "light";

const STORAGE_KEY = "portbay.theme";
const DEFAULT_THEME: Theme = "dark";

function loadInitial(): Theme {
  if (!browser) return DEFAULT_THEME;
  const v = localStorage.getItem(STORAGE_KEY);
  return v === "light" || v === "dark" ? v : DEFAULT_THEME;
}

function apply(value: Theme) {
  if (!browser) return;
  document.body.setAttribute("data-theme", value);
  document.documentElement.classList.toggle("light", value === "light");
  document.documentElement.classList.toggle("dark", value === "dark");
  localStorage.setItem(STORAGE_KEY, value);
}

function createThemeStore() {
  const initial = loadInitial();
  let current = $state<Theme>(initial);
  if (browser) apply(initial);

  return {
    get value() {
      return current;
    },
    set(next: Theme) {
      current = next;
      apply(current);
    },
    toggle() {
      current = current === "dark" ? "light" : "dark";
      apply(current);
    },
  };
}

export const theme = createThemeStore();
