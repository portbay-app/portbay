/**
 * Unit tests for the optimistic lifecycle core (card: P3 — Speed as a feature).
 *
 * This is the CI guard for the interaction-latency budget. The "feedback
 * within 100 ms" promise rests on two things being true:
 *
 *   1. Issuing an action yields its optimistic display *synchronously* — there
 *      is no await between the click and the row flipping. `beginTransition`
 *      being a pure function makes that structural, and the first test pins it.
 *   2. A stale, wrong-direction poll reading does NOT clear a fresh overlay,
 *      so the row doesn't flicker back during the ~1s the backend takes to
 *      catch up. That's the subtle resolution logic, exhaustively tested here.
 *
 * The end-to-end "click Play → row paints 'starting' under budget" assertion
 * belongs to the shared WebDriver harness owned by the P4 perf-CI card; this
 * guards the logic that makes the budget achievable.
 */
import { describe, expect, it } from "vitest";

import {
  beginTransition,
  clearTransition,
  displayFor,
  onStatusEvent,
  optimisticDisplay,
  resolvesTransition,
  type TransitionMap,
} from "$lib/lifecycle/optimistic";
import type { PortbayStatus } from "$lib/types/status";

const ALL: PortbayStatus[] = [
  "stopped",
  "starting",
  "running",
  "unhealthy",
  "crashed",
  "port_conflict",
];

describe("displayFor", () => {
  it("maps a start to 'starting' and a stop to 'stopping'", () => {
    expect(displayFor("start")).toBe("starting");
    expect(displayFor("stop")).toBe("stopping");
  });
});

describe("beginTransition (synchronous optimistic flip)", () => {
  it("flips the display immediately, with no async step", () => {
    const empty: TransitionMap = {};
    const after = beginTransition(empty, "p1", "start", 1000);
    // The flip is observable on the very next line — this is the latency
    // budget guarantee in microcosm.
    expect(optimisticDisplay(after, "p1", "stopped")).toBe("starting");
    expect(after.p1).toEqual({
      kind: "start",
      display: "starting",
      startedAt: 1000,
    });
  });

  it("does not mutate the input map (immutability)", () => {
    const empty: TransitionMap = {};
    beginTransition(empty, "p1", "stop");
    expect(empty).toEqual({});
  });

  it("overwrites a prior transition for the same id", () => {
    let m = beginTransition({}, "p1", "start", 1);
    m = beginTransition(m, "p1", "stop", 2);
    expect(m.p1.display).toBe("stopping");
    expect(m.p1.startedAt).toBe(2);
  });
});

describe("resolvesTransition — stale-event suppression", () => {
  it("a start resolves on any status except 'stopped'", () => {
    for (const s of ALL) {
      expect(resolvesTransition("start", s)).toBe(s !== "stopped");
    }
  });

  it("a stop resolves only once at rest (not running/starting)", () => {
    for (const s of ALL) {
      const expected = s !== "running" && s !== "starting";
      expect(resolvesTransition("stop", s)).toBe(expected);
    }
  });
});

describe("onStatusEvent", () => {
  it("keeps a start overlay through a stale 'stopped' tick", () => {
    const m = beginTransition({}, "p1", "start");
    const after = onStatusEvent(m, "p1", "stopped");
    // Stale poll: the process hasn't booted yet. Overlay must survive.
    expect(optimisticDisplay(after, "p1", "stopped")).toBe("starting");
    expect(after).toBe(m); // unchanged reference — no needless re-render
  });

  it("clears a start overlay once the project is starting/running", () => {
    const m = beginTransition({}, "p1", "start");
    // Real "running" event: overlay cleared, row shows canonical "running".
    expect(
      optimisticDisplay(onStatusEvent(m, "p1", "running"), "p1", "running"),
    ).toBe("running");
    // Real "starting" event: overlay cleared; canonical is now "starting"
    // too, so the row still reads "starting" — but from real state, not the
    // overlay (proven by the cleared map below).
    const afterStarting = onStatusEvent(m, "p1", "starting");
    expect("p1" in afterStarting).toBe(false);
    expect(optimisticDisplay(afterStarting, "p1", "starting")).toBe("starting");
  });

  it("keeps a stop overlay through a stale 'running' tick", () => {
    const m = beginTransition({}, "p1", "stop");
    const after = onStatusEvent(m, "p1", "running");
    expect(optimisticDisplay(after, "p1", "running")).toBe("stopping");
  });

  it("clears a stop overlay once the project is stopped", () => {
    const m = beginTransition({}, "p1", "stop");
    const after = onStatusEvent(m, "p1", "stopped");
    expect(optimisticDisplay(after, "p1", "stopped")).toBe("stopped");
  });

  it("clears a stop overlay if the project crashes while stopping", () => {
    const m = beginTransition({}, "p1", "stop");
    const after = onStatusEvent(m, "p1", "crashed");
    expect(optimisticDisplay(after, "p1", "crashed")).toBe("crashed");
  });

  it("is a no-op for an id with no overlay", () => {
    const m: TransitionMap = {};
    expect(onStatusEvent(m, "ghost", "running")).toBe(m);
  });
});

describe("clearTransition (rollback)", () => {
  it("drops the overlay so the row falls back to canonical status", () => {
    const m = beginTransition({}, "p1", "start");
    const after = clearTransition(m, "p1");
    expect(optimisticDisplay(after, "p1", "stopped")).toBe("stopped");
  });

  it("returns the same reference when there is nothing to clear", () => {
    const m: TransitionMap = {};
    expect(clearTransition(m, "p1")).toBe(m);
  });
});

describe("optimisticDisplay", () => {
  it("returns canonical status when no overlay is present", () => {
    for (const s of ALL) {
      expect(optimisticDisplay({}, "p1", s)).toBe(s);
    }
  });
});
