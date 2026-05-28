/**
 * crashSurface — the proactive "send this crash in one click" surface.
 *
 * PortBay already captures crashes locally (Rust panics + JS errors, written to
 * disk by `telemetry.rs`), but until now the only way to send one was to dig
 * into Settings. This store puts a crash report in front of the user the moment
 * it matters, so reports actually reach us even with automatic diagnostics off
 * (the send is an explicit click — see `send_crash_report` on the Rust side).
 *
 * Two entry points, two presentations (rendered by `CrashReportCard.svelte`):
 *
 *   - `presentLatestPending()` — called once on boot. If a crash from a previous
 *     session is sitting on disk (e.g. a Rust panic that took the app down), it
 *     surfaces as a centred modal. This is the Bun-style "it crashed, here's the
 *     one-click report" flow.
 *
 *   - `noteLiveError(id, signature)` — called by the crash reporter when a JS
 *     error/rejection is caught mid-session (the app is still alive). It surfaces
 *     as a quiet bottom-right card. To avoid nagging, each distinct signature is
 *     shown at most once, ever (persisted), and only one card shows at a time.
 *
 * Dismissals are remembered so a declined crash never re-prompts on the next
 * launch — it stays available in Settings → Crash reporting either way.
 */

import { browser } from "$app/environment";

import { safeInvoke } from "$lib/ipc";
import type { CrashReport, CrashReportSummary } from "$lib/types/telemetry";

/** localStorage keys: signatures already surfaced, and crash ids snoozed. */
const KEY_SEEN_SIGS = "portbay.crash.seenSignatures";
const KEY_SNOOZED_IDS = "portbay.crash.snoozedIds";

export type CrashSurfaceMode = "crash" | "live";
export type CrashSurfaceState = "idle" | "sending" | "sent" | "error";

function readSet(key: string): Set<string> {
  if (!browser) return new Set();
  try {
    const raw = localStorage.getItem(key);
    if (!raw) return new Set();
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? new Set(parsed.map(String)) : new Set();
  } catch {
    return new Set();
  }
}

function addToSet(key: string, value: string): void {
  if (!browser) return;
  const set = readSet(key);
  set.add(value);
  // Cap to avoid unbounded growth — only the most recent matter for de-dup.
  const trimmed = [...set].slice(-200);
  try {
    localStorage.setItem(key, JSON.stringify(trimmed));
  } catch {
    /* storage full / disabled — de-dup just won't persist across launches */
  }
}

function createCrashSurfaceStore() {
  let mode = $state<CrashSurfaceMode>("crash");
  let phase = $state<CrashSurfaceState>("idle");
  let report = $state<CrashReport | null>(null);

  const isOpen = $derived(report !== null);

  async function fetchReport(id: string): Promise<CrashReport | null> {
    try {
      return await safeInvoke<CrashReport>("read_crash_report", { id });
    } catch {
      return null;
    }
  }

  /**
   * Boot-time check for a crash left over from a previous session. Shows the
   * newest pending report that hasn't been snoozed, as a centred modal.
   */
  async function presentLatestPending(): Promise<void> {
    if (!browser || report !== null) return;
    let summaries: CrashReportSummary[];
    try {
      summaries = await safeInvoke<CrashReportSummary[]>("list_crash_reports");
    } catch {
      return;
    }
    const snoozed = readSet(KEY_SNOOZED_IDS);
    // `list_crash_reports` is already newest-first.
    const next = summaries.find((s) => !snoozed.has(s.id));
    if (!next) return;
    const full = await fetchReport(next.id);
    if (!full || report !== null) return;
    mode = "crash";
    phase = "idle";
    report = full;
  }

  /**
   * A live error was just captured. Surface it once per signature as a quiet
   * bottom-right card; never interrupt if a card is already showing.
   */
  async function noteLiveError(id: string, signature: string): Promise<void> {
    if (!browser || report !== null) return;
    const seen = readSet(KEY_SEEN_SIGS);
    if (seen.has(signature)) return;
    addToSet(KEY_SEEN_SIGS, signature);
    const full = await fetchReport(id);
    // Re-check after the await — a concurrent error may have opened a card.
    if (!full || report !== null) return;
    mode = "live";
    phase = "idle";
    report = full;
  }

  /** Upload the presented report (explicit consent), then show confirmation. */
  async function send(): Promise<void> {
    if (!report || phase === "sending") return;
    phase = "sending";
    try {
      await safeInvoke("send_crash_report", { id: report.id });
      phase = "sent";
    } catch {
      // safeInvoke already toasted the failure; let the user retry or dismiss.
      phase = "error";
    }
  }

  /** Delete the report without sending, then close. */
  async function discard(): Promise<void> {
    if (!report) return;
    const id = report.id;
    try {
      await safeInvoke("discard_crash_report", { id });
    } catch {
      /* benign — it stays in Settings, but the card still closes */
    }
    close();
  }

  /**
   * Close without deciding. The report stays on disk (still listed in
   * Settings) but is snoozed so it won't re-prompt on the next launch.
   */
  function dismiss(): void {
    if (report && mode === "crash") addToSet(KEY_SNOOZED_IDS, report.id);
    close();
  }

  function close(): void {
    report = null;
    phase = "idle";
  }

  return {
    get mode() {
      return mode;
    },
    get phase() {
      return phase;
    },
    get report() {
      return report;
    },
    get isOpen() {
      return isOpen;
    },
    presentLatestPending,
    noteLiveError,
    send,
    discard,
    dismiss,
  };
}

export const crashSurface = createCrashSurfaceStore();
