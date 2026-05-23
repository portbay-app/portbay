/**
 * App preferences — behavioural toggles backed by the Rust
 * `preferences.json` file. Single source of truth: the Rust core. The
 * frontend mirrors a snapshot in memory so the Settings UI reads
 * synchronously after the initial `load()`.
 *
 * Pairs with `commands::preferences::{get_preferences, set_preferences,
 * mark_close_toast_seen}` on the Rust side.
 */
import { browser } from "$app/environment";

import { safeInvoke } from "$lib/ipc";

export interface Preferences {
  /** Install the menu-bar tray icon at launch. */
  showTrayIcon: boolean;
  /** When true, closing the window hides instead of quits the app. */
  closeToMenuBar: boolean;
  /** Internal: the one-time "still running" toast has been shown. */
  closeToMenuBarToastSeen: boolean;
}

const DEFAULTS: Preferences = {
  showTrayIcon: true,
  closeToMenuBar: true,
  closeToMenuBarToastSeen: false,
};

function createPreferencesStore() {
  let value = $state<Preferences>({ ...DEFAULTS });
  let loaded = $state<boolean>(false);

  async function load(): Promise<void> {
    if (!browser) return;
    try {
      value = await safeInvoke<Preferences>("get_preferences");
    } catch {
      // safeInvoke already showed the toast; keep defaults so the UI
      // stays interactive rather than blocked behind an opaque error.
    } finally {
      loaded = true;
    }
  }

  async function update(patch: Partial<Preferences>): Promise<void> {
    const next: Preferences = { ...value, ...patch };
    try {
      // The backend returns the persisted snapshot; trust it over the
      // optimistic patch in case server-side normalisation kicks in.
      value = await safeInvoke<Preferences>("set_preferences", { prefs: next });
    } catch {
      // safeInvoke already showed the toast; leave `value` untouched
      // so the UI rolls back automatically.
    }
  }

  async function markCloseToastSeen(): Promise<void> {
    try {
      await safeInvoke<void>("mark_close_toast_seen");
      value = { ...value, closeToMenuBarToastSeen: true };
    } catch {
      /* benign — the toast won't suppress itself, but the app keeps working */
    }
  }

  return {
    get value() {
      return value;
    },
    get loaded() {
      return loaded;
    },
    load,
    update,
    markCloseToastSeen,
  };
}

export const preferences = createPreferencesStore();
