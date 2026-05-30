/**
 * Theme preference — `system` (default), `dark`, or `light`.
 *
 * `system` follows the OS appearance (and live-switches when the user flips
 * macOS between Light/Dark or has it on Auto), exactly like the app icon.
 * `dark`/`light` pin an explicit appearance.
 *
 * The chosen *preference* is persisted; the *resolved* appearance (always
 * `dark` or `light`) is what gets pushed onto `<html class="dark|light">` and
 * `<body data-theme="…">` so CSS tokens switch without component-level
 * branches. The no-flash inline script in `app.html` applies the same
 * resolution before hydration.
 */
import { browser } from "$app/environment";

export type ThemePreference = "system" | "dark" | "light";
export type ResolvedTheme = "dark" | "light";

const PREF_KEY = "portbay.theme";
/** Legacy key written by the old settings-page "system" hack. */
const LEGACY_CHOICE_KEY = "portbay.themeChoice";
const DEFAULT_PREFERENCE: ThemePreference = "system";

function isPreference(v: unknown): v is ThemePreference {
  return v === "system" || v === "dark" || v === "light";
}

function systemTheme(): ResolvedTheme {
  if (!browser) return "dark";
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

function loadPreference(): ThemePreference {
  if (!browser) return DEFAULT_PREFERENCE;
  // Prefer the legacy choice key first: when present it holds the true
  // preference (incl. "system"), whereas the old PREF_KEY only ever stored a
  // resolved dark|light.
  const legacy = localStorage.getItem(LEGACY_CHOICE_KEY);
  if (isPreference(legacy)) return legacy;
  const stored = localStorage.getItem(PREF_KEY);
  if (isPreference(stored)) return stored;
  return DEFAULT_PREFERENCE;
}

function resolve(pref: ThemePreference): ResolvedTheme {
  return pref === "system" ? systemTheme() : pref;
}

function apply(resolved: ResolvedTheme) {
  if (!browser) return;
  document.body.setAttribute("data-theme", resolved);
  document.documentElement.classList.toggle("light", resolved === "light");
  document.documentElement.classList.toggle("dark", resolved === "dark");
}

function persist(pref: ThemePreference) {
  if (!browser) return;
  try {
    localStorage.setItem(PREF_KEY, pref);
    localStorage.removeItem(LEGACY_CHOICE_KEY); // consolidated into PREF_KEY
  } catch {
    /* private mode — preference is session-only */
  }
}

function createThemeStore() {
  // Compute the initial values as plain locals first, then seed the runes from
  // them. Initializing one `$state` by reading another (`resolve(preference)`)
  // — or passing a rune to `apply()` at setup — trips Svelte's
  // `state_referenced_locally` warning, since those top-level reads capture
  // only the initial value. The runes below are only ever read through the
  // getters and mutated in the handlers, which is the reactive path.
  const initialPreference = loadPreference();
  const initialResolved = resolve(initialPreference);
  let preference = $state<ThemePreference>(initialPreference);
  let resolved = $state<ResolvedTheme>(initialResolved);

  if (browser) {
    apply(initialResolved);
    // Follow the OS when (and only when) the preference is "system".
    window.matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => {
      if (preference !== "system") return;
      resolved = systemTheme();
      apply(resolved);
    });
  }

  function set(next: ThemePreference) {
    preference = next;
    resolved = resolve(next);
    persist(next);
    apply(resolved);
  }

  return {
    /** User's chosen preference: system | dark | light. */
    get preference() {
      return preference;
    },
    /** Effective appearance actually applied: dark | light. */
    get resolved() {
      return resolved;
    },
    /** Back-compat alias for `resolved` (the applied dark|light). */
    get value() {
      return resolved;
    },
    set,
    /** Toggle pins an explicit appearance (drops out of "system"). */
    toggle() {
      set(resolved === "dark" ? "light" : "dark");
    },
  };
}

export const theme = createThemeStore();
