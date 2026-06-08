/**
 * deployLive — the event fold behind the streaming deploy UI. Pins the step
 * lifecycle (queued → running → ok/failed), output accumulation + the front
 * trim cap, reconciliation against the authoritative StepResult[] (fallback
 * output, skipped vs cancelled tails), error finalization, and the summary /
 * duration helpers the panes render from.
 */
import { describe, expect, it } from "vitest";

import {
  applyDeployEvent,
  finalizeError,
  formatDuration,
  initLiveSteps,
  OUTPUT_CAP,
  reconcileResults,
  summarize,
  type LiveStep,
} from "$lib/deployLive";
import type { DeployEvent, StepResult } from "$lib/types/sshTunnels";

const RUN = "run-1";

const started = (index: number): DeployEvent => ({
  kind: "stepStarted",
  runId: RUN,
  index,
  command: `cmd ${index}`,
});
const output = (index: number, chunk: string, stderr = false): DeployEvent => ({
  kind: "output",
  runId: RUN,
  index,
  stderr,
  chunk,
});
const done = (index: number, exitCode: number, durationMs = 1200): DeployEvent => ({
  kind: "stepDone",
  runId: RUN,
  index,
  exitCode,
  durationMs,
});
const result = (command: string, exitCode: number, stdout = "", stderr = ""): StepResult => ({
  command,
  stdout,
  stderr,
  exitCode,
});

describe("initLiveSteps", () => {
  it("seeds every command as queued with empty output", () => {
    const steps = initLiveSteps(["npm ci", "npm run build"]);
    expect(steps).toHaveLength(2);
    expect(steps.every((s) => s.status === "queued" && s.output === "")).toBe(true);
    expect(steps[1].command).toBe("npm run build");
  });
});

describe("applyDeployEvent", () => {
  it("walks a step through running → ok with accumulated output", () => {
    const steps = initLiveSteps(["npm ci"]);
    applyDeployEvent(steps, started(0), 1000);
    expect(steps[0].status).toBe("running");
    expect(steps[0].startedAt).toBe(1000);

    applyDeployEvent(steps, output(0, "added 100 packages\n"));
    applyDeployEvent(steps, output(0, "audited 100 packages\n"));
    expect(steps[0].output).toBe("added 100 packages\naudited 100 packages\n");

    applyDeployEvent(steps, done(0, 0, 4321));
    expect(steps[0].status).toBe("ok");
    expect(steps[0].exitCode).toBe(0);
    expect(steps[0].durationMs).toBe(4321);
  });

  it("marks a non-zero exit failed", () => {
    const steps = initLiveSteps(["false"]);
    applyDeployEvent(steps, started(0));
    applyDeployEvent(steps, done(0, 1));
    expect(steps[0].status).toBe("failed");
  });

  it("interleaves stderr chunks into the same output stream", () => {
    const steps = initLiveSteps(["build"]);
    applyDeployEvent(steps, output(0, "out "));
    applyDeployEvent(steps, output(0, "err ", true));
    applyDeployEvent(steps, output(0, "out2"));
    expect(steps[0].output).toBe("out err out2");
  });

  it("ignores sync events and out-of-range indexes", () => {
    const steps = initLiveSteps(["a"]);
    applyDeployEvent(steps, { kind: "sync", runId: RUN, uploaded: 1, total: 2, bytes: 10 });
    applyDeployEvent(steps, output(5, "lost"));
    expect(steps[0].output).toBe("");
  });

  it("trims oldest output past the cap, starting at a line boundary", () => {
    const steps = initLiveSteps(["chatty"]);
    const line = `${"x".repeat(63)}\n`; // 64 bytes per line
    const big = line.repeat(Math.ceil(OUTPUT_CAP / line.length) + 16);
    applyDeployEvent(steps, output(0, big));
    applyDeployEvent(steps, output(0, "tail-marker"));
    const out = steps[0].output;
    expect(out.length).toBeLessThanOrEqual(OUTPUT_CAP);
    expect(out.endsWith("tail-marker")).toBe(true);
    // Trimmed window starts on a whole line, not mid-line.
    expect(out.startsWith("x".repeat(63))).toBe(true);
  });
});

describe("reconcileResults", () => {
  it("settles statuses and marks unreached steps skipped after a failure", () => {
    const steps = initLiveSteps(["ok-step", "boom", "never-ran"]);
    applyDeployEvent(steps, started(0));
    applyDeployEvent(steps, done(0, 0));
    applyDeployEvent(steps, started(1));
    applyDeployEvent(steps, done(1, 2));
    reconcileResults(
      steps,
      [result("ok-step", 0), result("boom", 2)],
      false,
    );
    expect(steps.map((s) => s.status)).toEqual(["ok", "failed", "skipped"]);
  });

  it("fills output from the captured result when no events streamed", () => {
    const steps = initLiveSteps(["quiet"]);
    reconcileResults(steps, [result("quiet", 0, "stdout text", "stderr text")], false);
    expect(steps[0].output).toBe("stdout textstderr text");
    expect(steps[0].status).toBe("ok");
  });

  it("keeps streamed output over the captured fallback", () => {
    const steps = initLiveSteps(["loud"]);
    applyDeployEvent(steps, output(0, "streamed"));
    reconcileResults(steps, [result("loud", 0, "captured")], false);
    expect(steps[0].output).toBe("streamed");
  });

  it("marks a cancelled run's killed step and unreached tail cancelled", () => {
    const steps = initLiveSteps(["long-build", "restart"]);
    applyDeployEvent(steps, started(0));
    reconcileResults(steps, [result("long-build", -1)], true);
    expect(steps.map((s) => s.status)).toEqual(["cancelled", "cancelled"]);
  });
});

describe("finalizeError", () => {
  it("fails the in-flight step and skips the queued tail", () => {
    const steps = initLiveSteps(["a", "b", "c"]);
    applyDeployEvent(steps, started(0));
    applyDeployEvent(steps, done(0, 0));
    applyDeployEvent(steps, started(1));
    expect(finalizeError(steps)).toBe(true);
    expect(steps.map((s) => s.status)).toEqual(["ok", "failed", "skipped"]);
  });

  it("reports false when nothing ever started (caller clears the model)", () => {
    const steps = initLiveSteps(["a", "b"]);
    expect(finalizeError(steps)).toBe(false);
    expect(steps.every((s) => s.status === "skipped")).toBe(true);
  });
});

describe("summarize", () => {
  const settle = (statuses: LiveStep["status"][], durations: (number | null)[] = []) =>
    statuses.map((status, i) => ({
      command: `s${i}`,
      status,
      output: "",
      exitCode: null,
      durationMs: durations[i] ?? null,
      startedAt: null,
    }));

  it("flags an all-ok run with summed duration", () => {
    const s = summarize(settle(["ok", "ok"], [1000, 500]));
    expect(s).toMatchObject({ allOk: true, failedAt: -1, cancelled: false, running: false });
    expect(s.totalMs).toBe(1500);
  });

  it("locates the first failure and reports in-flight runs", () => {
    expect(summarize(settle(["ok", "failed", "skipped"])).failedAt).toBe(1);
    expect(summarize(settle(["ok", "running", "queued"])).running).toBe(true);
    expect(summarize(settle(["ok", "cancelled"])).cancelled).toBe(true);
  });
});

describe("formatDuration", () => {
  it("scales units with magnitude", () => {
    expect(formatDuration(850)).toBe("850 ms");
    expect(formatDuration(12_400)).toBe("12.4 s");
    expect(formatDuration(125_000)).toBe("2m 05s");
  });
});
