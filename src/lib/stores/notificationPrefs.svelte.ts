/**
 * Notification preferences — synchronous frontend mirror of the persisted
 * Rust `preferences.notifications` block.
 */
import { browser } from "$app/environment";

import { safeInvoke } from "$lib/ipc";
import {
  DEFAULT_NOTIFICATION_PREFS,
  normaliseNotificationPrefs,
  type NotificationPrefs,
} from "$lib/notifications/prefs";

const SAVE_DEBOUNCE_MS = 300;

function createNotificationPrefsStore() {
  let value = $state<NotificationPrefs>(normaliseNotificationPrefs(DEFAULT_NOTIFICATION_PREFS));
  let loaded = $state(false);
  let saving = $state(false);
  let timer: ReturnType<typeof setTimeout> | null = null;

  async function load(): Promise<void> {
    if (!browser) return;
    try {
      const prefs = await safeInvoke<NotificationPrefs>("get_notification_prefs");
      value = normaliseNotificationPrefs(prefs);
    } catch {
      /* safeInvoke already surfaced the error; defaults keep routing usable */
    } finally {
      loaded = true;
    }
  }

  function update(mutator: (draft: NotificationPrefs) => void): void {
    const next = normaliseNotificationPrefs(value);
    mutator(next);
    value = normaliseNotificationPrefs(next);
    scheduleSave();
  }

  async function resetToDefaults(): Promise<void> {
    value = normaliseNotificationPrefs(DEFAULT_NOTIFICATION_PREFS);
    await saveNow();
  }

  async function saveNow(): Promise<void> {
    if (!browser) return;
    if (timer) {
      clearTimeout(timer);
      timer = null;
    }
    saving = true;
    const snapshot = normaliseNotificationPrefs(value);
    try {
      value = normaliseNotificationPrefs(
        await safeInvoke<NotificationPrefs>("set_notification_prefs", { prefs: snapshot }),
      );
    } catch {
      /* safeInvoke already surfaced the error; keep optimistic value visible */
    } finally {
      saving = false;
    }
  }

  function scheduleSave(): void {
    if (!browser) return;
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => {
      void saveNow();
    }, SAVE_DEBOUNCE_MS);
  }

  return {
    get value() {
      return value;
    },
    get loaded() {
      return loaded;
    },
    get saving() {
      return saving;
    },
    load,
    update,
    resetToDefaults,
    saveNow,
  };
}

export const notificationPrefs = createNotificationPrefsStore();
