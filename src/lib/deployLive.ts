/**
 * Pure state-fold for a streaming deploy run: the UI seeds one `LiveStep` per
 * command, folds `portbay://deploy` events into them as the run executes, then
 * reconciles against the command's returned `StepResult[]` (which stays the
 * source of truth — events are display-only and may be lost if a listener
 * attaches late).
 *
 * All functions mutate the passed array in place (Svelte 5 `$state` proxies
 * make that reactive); they're plain data transforms so the whole lifecycle is
 * unit-testable without a DOM or a backend.
 */
import type { DeployEvent, StepResult } from "$lib/types/sshTunnels";

export type LiveStepStatus = "queued" | "running" | "ok" | "failed" | "skipped" | "cancelled";

export interface LiveStep {
  command: string;
  status: LiveStepStatus;
  /** Raw output (ANSI codes intact), stdout + stderr interleaved as received. */
  output: string;
  exitCode: number | null;
  durationMs: number | null;
  /** Epoch ms when the step started running, for live elapsed display. */
  startedAt: number | null;
}

/** Retained output ceiling per step; older output is trimmed from the front. */
export const OUTPUT_CAP = 256 * 1024;
/** Post-trim size — trimming in slabs keeps the ANSI re-render occasional. */
const TRIM_TO = 192 * 1024;

/** Seed the live model: every command queued, no output. */
export function initLiveSteps(commands: string[]): LiveStep[] {
  return commands.map((command) => ({
    command,
    status: "queued",
    output: "",
    exitCode: null,
    durationMs: null,
    startedAt: null,
  }));
}

/** Drop the oldest output once over the cap, preferring a newline boundary so
    the visible window starts on a whole line. */
function capOutput(output: string): string {
  if (output.length <= OUTPUT_CAP) return output;
  let start = output.length - TRIM_TO;
  const nl = output.indexOf("\n", start);
  if (nl !== -1 && nl < output.length - 1) start = nl + 1;
  return output.slice(start);
}

/**
 * Fold one streaming event into the live model. `sync` events are ignored
 * here (the upload leg is tracked separately by the project-deploy UI).
 * `now` is injectable for tests.
 */
export function applyDeployEvent(steps: LiveStep[], ev: DeployEvent, now: number = Date.now()): void {
  if (ev.kind === "sync") return;
  const step = steps[ev.index];
  if (!step) return;
  switch (ev.kind) {
    case "stepStarted":
      step.status = "running";
      step.startedAt = now;
      break;
    case "output":
      step.output = capOutput(step.output + ev.chunk);
      break;
    case "stepDone":
      step.exitCode = ev.exitCode;
      step.durationMs = ev.durationMs;
      step.status = ev.exitCode === 0 ? "ok" : "failed";
      break;
  }
}

/**
 * Fold the command's returned results back in once the run resolves: exit
 * codes/durations become authoritative, a step that never streamed output
 * gets its captured stdout+stderr as a fallback, steps the backend never
 * reached become `skipped` (failure short-circuit) or `cancelled`. A final
 * step killed mid-flight (exit -1 on a cancelled run) also reads `cancelled`.
 */
export function reconcileResults(
  steps: LiveStep[],
  results: StepResult[],
  cancelled: boolean,
): void {
  steps.forEach((step, i) => {
    const r = results[i];
    if (r) {
      step.exitCode = r.exitCode;
      if (step.output === "") step.output = capOutput(r.stdout + r.stderr);
      step.status =
        r.exitCode === 0 ? "ok" : cancelled && i === results.length - 1 ? "cancelled" : "failed";
    } else {
      step.status = cancelled ? "cancelled" : "skipped";
    }
  });
}

/**
 * Settle the model after the run promise *rejects* (connection/auth failure):
 * an in-flight step reads `failed`, anything still queued reads `skipped`.
 * Returns true when any step had started — callers clear the model entirely
 * when the run died before doing anything (the error toast says why).
 */
export function finalizeError(steps: LiveStep[]): boolean {
  let started = false;
  for (const step of steps) {
    if (step.status === "running") {
      step.status = "failed";
      started = true;
    } else if (step.status === "queued") {
      step.status = "skipped";
    } else {
      started = true;
    }
  }
  return started;
}

/** Aggregate view of a settled (or in-flight) run for summary lines. */
export function summarize(steps: LiveStep[]): {
  failedAt: number;
  allOk: boolean;
  cancelled: boolean;
  running: boolean;
  totalMs: number;
} {
  return {
    failedAt: steps.findIndex((s) => s.status === "failed"),
    allOk: steps.length > 0 && steps.every((s) => s.status === "ok"),
    cancelled: steps.some((s) => s.status === "cancelled"),
    running: steps.some((s) => s.status === "running" || s.status === "queued"),
    totalMs: steps.reduce((acc, s) => acc + (s.durationMs ?? 0), 0),
  };
}

/** "850 ms" under a second, "12.4 s" under a minute, then "2m 05s". */
export function formatDuration(ms: number): string {
  if (ms < 1000) return `${Math.round(ms)} ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)} s`;
  const m = Math.floor(ms / 60_000);
  const s = Math.round((ms % 60_000) / 1000);
  return `${m}m ${String(s).padStart(2, "0")}s`;
}
