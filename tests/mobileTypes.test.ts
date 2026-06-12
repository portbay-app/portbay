import { describe, expect, it } from "vitest";

import {
  deviceShortLabel,
  groupTargets,
  isMobileType,
  mobilePhaseDisplay,
  mobilePhaseLabel,
  type MobileRunTarget,
} from "../src/lib/types/mobile";

const t = (over: Partial<MobileRunTarget>): MobileRunTarget => ({
  id: "id",
  name: "Device",
  platform: "ios",
  kind: "simulator",
  state: "shutdown",
  ...over,
});

describe("groupTargets", () => {
  const targets: MobileRunTarget[] = [
    t({ id: "a", name: "iPhone 16", state: "booted" }),
    t({ id: "b", name: "iPhone SE", state: "shutdown", osVersion: "iOS 18.2" }),
    t({
      id: "c",
      name: "Pixel 8",
      platform: "android",
      kind: "emulator",
      state: "shutdown",
    }),
    t({
      id: "d",
      name: "Nour's iPhone",
      kind: "physical",
      state: "connected",
      unsupportedReason: "signing flow pending",
    }),
  ];

  it("groups ready (booted/connected) first, unsupported stay in their kind group", () => {
    const groups = groupTargets(targets);
    expect(groups.map((g) => g.group)).toEqual([
      "Ready",
      "Simulators",
      "Emulators",
      "Physical devices",
    ]);
    // Booted sim is Ready; the connected-but-unsupported physical is NOT
    // promoted to Ready (it can't actually run).
    expect(groups[0].targets.map((x) => x.id)).toEqual(["a"]);
    expect(groups[3].targets.map((x) => x.id)).toEqual(["d"]);
  });

  it("filters by name and os version, case-insensitive", () => {
    expect(
      groupTargets(targets, "pixel").flatMap((g) => g.targets.map((x) => x.id)),
    ).toEqual(["c"]);
    expect(
      groupTargets(targets, "18.2").flatMap((g) => g.targets.map((x) => x.id)),
    ).toEqual(["b"]);
    expect(groupTargets(targets, "zzz")).toEqual([]);
  });
});

describe("deviceShortLabel", () => {
  it("labels the pin forms users actually see", () => {
    expect(deviceShortLabel(null)).toBe("Auto destination");
    expect(deviceShortLabel(undefined)).toBe("Auto destination");
    expect(deviceShortLabel("avd:Pixel_8_API_35")).toBe("Pixel 8 API 35");
    expect(deviceShortLabel("ios")).toBe("iOS Simulator");
    expect(deviceShortLabel("android")).toBe("Android Emulator");
    expect(deviceShortLabel("emulator-5554")).toBe("emulator-5554");
  });
});

describe("phase display", () => {
  it("maps phases onto the existing status tokens", () => {
    expect(mobilePhaseDisplay("connected")).toBe("running");
    expect(mobilePhaseDisplay("build_failed")).toBe("crashed");
    expect(mobilePhaseDisplay("building")).toBe("starting");
    expect(mobilePhaseDisplay("launching")).toBe("starting");
  });

  it("has a label for every phase", () => {
    for (const phase of [
      "resolving_device",
      "booting_device",
      "building",
      "installing",
      "launching",
      "connected",
      "build_failed",
    ] as const) {
      expect(mobilePhaseLabel[phase]).toBeTruthy();
    }
  });
});

describe("isMobileType", () => {
  it("covers exactly the four mobile kinds", () => {
    expect(isMobileType("flutter")).toBe(true);
    expect(isMobileType("xcode")).toBe(true);
    expect(isMobileType("android")).toBe(true);
    expect(isMobileType("expo")).toBe(true);
    expect(isMobileType("next")).toBe(false);
    expect(isMobileType("static")).toBe(false);
  });
});
