/**
 * Toast bus — the bottom-right surface shown by `ToastHost`.
 *
 * Notification routing: every push is recorded in the bell (`notifications`),
 * which is the home for all notifications. The bottom-right toast is reserved
 * for high-priority items that need the user's attention right now — actual
 * errors, or anything offering an action the user must decide on (see
 * `isHighPriority`). Success / info / plain-warning notifications go to the
 * bell only and never toast.
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

  /**
   * Whether an envelope earns a bottom-right toast (vs. living in the bell
   * only). High priority = a genuine error, or anything carrying an action the
   * user must decide on. Severity falls back to the same `whoCausedIt` mapping
   * the renderer uses (user → warning, system → error) when unset, so a plain
   * informational/success notification stays out of the corner.
   */
  function isHighPriority(e: CommandError): boolean {
    const severity = e.severity ?? (e.whoCausedIt === "user" ? "warning" : "error");
    return severity === "error" || e.actions.length > 0;
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
    if (dup) {
      return toasts.find((t) => fingerprint(t.envelope) === fp)?.id ?? "";
    }

    // The bell is the home for every notification; record it unconditionally.
    notifications.push(envelope);

    // The bottom-right toast is reserved for high-priority items — actual
    // errors or action-required envelopes. Everything else stays in the bell.
    if (!isHighPriority(envelope)) return "";

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
