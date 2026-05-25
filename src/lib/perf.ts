/**
 * Lightweight, dev-only interaction-latency instrument (card: P3 — Speed as a
 * feature). Records how long each IPC round-trip takes so the honest baseline
 * in `docs/PERFORMANCE.md` can be read from a real session rather than guessed.
 *
 * Zero cost in production: every entry point short-circuits unless
 * `import.meta.env.DEV` is set, so this compiles away in release builds.
 *
 * Read it from the dev console:
 *
 *   __portbayPerf.table()      // console.table of recent IPC timings
 *   __portbayPerf.summary()    // p50/p95/max per command
 *   __portbayPerf.samples      // raw ring buffer
 *   __portbayPerf.clear()
 */
import { browser } from "$app/environment";

const DEV = import.meta.env.DEV;

export interface PerfSample {
  /** e.g. "ipc:start_project". */
  label: string;
  /** Duration in milliseconds. */
  ms: number;
  /** Epoch ms when the sample was recorded. */
  at: number;
}

const MAX_SAMPLES = 300;
const samples: PerfSample[] = [];

/** Record a finished measurement. No-op outside dev. */
export function record(label: string, ms: number): void {
  if (!DEV) return;
  samples.push({ label, ms, at: Date.now() });
  if (samples.length > MAX_SAMPLES) samples.shift();
}

/** Start a manual timer; call the returned fn when the work finishes. */
export function startTimer(label: string): () => void {
  if (!DEV) return () => {};
  const t0 = performance.now();
  return () => record(label, performance.now() - t0);
}

function percentile(sorted: number[], p: number): number {
  if (sorted.length === 0) return 0;
  const idx = Math.min(sorted.length - 1, Math.floor((p / 100) * sorted.length));
  return sorted[idx];
}

/** Per-label p50 / p95 / max / count, for a quick read of where time goes. */
function summary(): Record<
  string,
  { count: number; p50: number; p95: number; max: number }
> {
  const byLabel = new Map<string, number[]>();
  for (const s of samples) {
    const arr = byLabel.get(s.label) ?? [];
    arr.push(s.ms);
    byLabel.set(s.label, arr);
  }
  const out: Record<
    string,
    { count: number; p50: number; p95: number; max: number }
  > = {};
  for (const [label, arr] of byLabel) {
    const sorted = [...arr].sort((a, b) => a - b);
    out[label] = {
      count: sorted.length,
      p50: Math.round(percentile(sorted, 50)),
      p95: Math.round(percentile(sorted, 95)),
      max: Math.round(sorted[sorted.length - 1]),
    };
  }
  return out;
}

// Expose a small console API in dev so the latency baseline can be captured
// from a live session without any UI surface.
if (DEV && browser) {
  (globalThis as Record<string, unknown>).__portbayPerf = {
    get samples() {
      return samples;
    },
    summary,
    table() {
      // eslint-disable-next-line no-console
      console.table(summary());
    },
    clear() {
      samples.length = 0;
    },
  };
}
