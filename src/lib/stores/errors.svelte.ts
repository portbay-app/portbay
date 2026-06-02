/**
 * Toast bus — the bottom-right surface shown by `ToastHost`.
 *
 * Notification routing is preference-driven. Each envelope carries a category
 * and severity; the user's Settings → Notifications matrix decides whether it
 * lands in the bell, bottom-right toast, or sound channel.
 *
 * Auto-dismiss policy (for the toasts that do show):
 *   - Toasts with `actions.length === 0` auto-dismiss after 8 s.
 *   - Toasts with at least one action stay until the user clicks an
 *     action or the dismiss (×) button. Actions imply something the user
 *     should consciously decide; we don't decide for them.
 *
 * Deduplication: identical envelopes (same `code` + `whatHappened`) pushed
 * within a 2 s window are coalesced — checked against the bell history so it
 * covers toasted and bell-only notifications alike (a flaky operation
 * retrying internally won't stack).
 */
import type { CommandError } from "$lib/types/error";
import { severityForEnvelope, shouldDeliver } from "$lib/notifications/prefs";
import { notificationPrefs } from "./notificationPrefs.svelte";
import { notifications } from "./notifications.svelte";

const AUTO_DISMISS_MS = 8_000;
const DEDUP_WINDOW_MS = 2_000;

export interface ToastEntry {
  id: string;
  envelope: CommandError;
  /** Unix millis when this toast first appeared. */
  pushedAt: number;
  /** Set when auto-dismiss is armed; cleared on user dismiss. */
  timer?: ReturnType<typeof setTimeout>;
}

function createErrorBus() {
  let toasts = $state<ToastEntry[]>([]);

  function fingerprint(e: CommandError): string {
    return `${e.code}::${e.whatHappened}`;
  }

  function dismiss(id: string) {
    const idx = toasts.findIndex((t) => t.id === id);
    if (idx === -1) return;
    if (toasts[idx].timer) clearTimeout(toasts[idx].timer);
    toasts = toasts.filter((t) => t.id !== id);
  }

  function push(envelope: CommandError): string {
    const now = Date.now();
    const fp = fingerprint(envelope);

    // Dedup against the bell history (which records every push), so coalescing
    // applies to toasted and bell-only notifications alike. When a duplicate is
    // suppressed, hand back the live toast id if one is still showing for it.
    const dup = notifications.value.find(
      (n) => fingerprint(n.envelope) === fp && now - n.receivedAt < DEDUP_WINDOW_MS,
    );
    const toastDup = toasts.find((t) => fingerprint(t.envelope) === fp && now - t.pushedAt < DEDUP_WINDOW_MS);
    if (dup || toastDup) {
      return toasts.find((t) => fingerprint(t.envelope) === fp)?.id ?? "";
    }

    const severity = severityForEnvelope(envelope);
    const prefs = notificationPrefs.value;
    const when = new Date(now);

    if (shouldDeliver(prefs, envelope.category, severity, "bell", when)) {
      notifications.push(envelope);
    }

    // No audible cue here. Sound is reserved for the task board: agent activity
    // (a completed card, a comment, an execution error) plays through the
    // activity store's `playAgentEventCue`. Command-error envelopes — including
    // a failed save that the backend tags `agent-board` — must stay silent.
    if (!shouldDeliver(prefs, envelope.category, severity, "toast", when)) return "";

    const id = crypto.randomUUID();
    const entry: ToastEntry = {
      id,
      envelope,
      pushedAt: now,
    };

    // Auto-dismiss only when there are no action buttons.
    if (envelope.actions.length === 0) {
      entry.timer = setTimeout(() => dismiss(id), AUTO_DISMISS_MS);
    }

    toasts = [...toasts, entry];
    return id;
  }

  function clear() {
    for (const t of toasts) {
      if (t.timer) clearTimeout(t.timer);
    }
    toasts = [];
  }

  return {
    get value() {
      return toasts;
    },
    push,
    dismiss,
    clear,
  };
}

export const errorBus = createErrorBus();
