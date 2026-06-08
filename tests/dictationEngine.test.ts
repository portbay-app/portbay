import { describe, expect, it } from "vitest";

import { resolveMicEngine, resolveToggleAction } from "$lib/dictation/engine";

// The "is the local engine actually enabled?" routing. Mirrors the user-facing
// contract: pick the local sidecar only when the engine is set to local AND a
// model has been chosen; otherwise fall back to macOS dictation so the mic
// button never goes dead.
describe("resolveMicEngine", () => {
  it("uses local only when the engine is local AND a model is chosen", () => {
    expect(resolveMicEngine("local", "parakeet-v3")).toBe("local");
  });

  it("falls back to macOS when the engine is local but no model is chosen", () => {
    expect(resolveMicEngine("local", "")).toBe("macos");
  });

  it("uses macOS whenever the engine is not local, regardless of model", () => {
    expect(resolveMicEngine("macos", "parakeet-v3")).toBe("macos");
    expect(resolveMicEngine("macos", "")).toBe("macos");
    // Unknown / unset engine strings also resolve to the safe default.
    expect(resolveMicEngine("", "parakeet-v3")).toBe("macos");
    expect(resolveMicEngine("whisper", "large-v3")).toBe("macos");
  });
});

// The mic-click decision: stop when you already hold it, hand off when another
// surface holds it, start when nothing is active.
describe("resolveToggleAction", () => {
  it("starts when idle, ignoring any stale owner", () => {
    expect(resolveToggleAction("idle", null, "ssh")).toBe("start");
    expect(resolveToggleAction("idle", "card", "ssh")).toBe("start");
  });

  it("stops when the requester already holds the session (either phase)", () => {
    expect(resolveToggleAction("arming", "ssh", "ssh")).toBe("stop");
    expect(resolveToggleAction("live", "ssh", "ssh")).toBe("stop");
  });

  it("hands off when another surface holds the session", () => {
    expect(resolveToggleAction("arming", "card", "ssh")).toBe("handoff");
    expect(resolveToggleAction("live", "card", "ssh")).toBe("handoff");
  });
});
