/**
 * Mobile run UX types — mirrors the Rust side:
 *  - `mobile_targets::RunTarget` / `PreflightCheck`
 *  - `mobile_phase::MobilePhase` / `MobilePhaseEvent`
 *
 * The phase sub-state is *additive* to {@link PortbayStatus}: the base
 * taxonomy is untouched; mobile rows/rails overlay the phase label while a
 * run is in flight so "Running" is only shown once the app is actually
 * attached (Connected).
 */
import type { DisplayStatus } from "./status";
import type { ProjectType, ProjectView } from "./projects";

/** One run destination, normalized across platforms. */
export interface MobileRunTarget {
  /** Pinned into `MobileRunConfig.device` (udid / serial / `avd:<name>` / `ios`|`android`). */
  id: string;
  name: string;
  platform: "ios" | "android" | "macos" | "web" | "unknown";
  osVersion?: string;
  kind: "simulator" | "emulator" | "physical" | "desktop" | "web";
  state: "booted" | "shutdown" | "connected";
  /** Present when listed but not runnable (e.g. physical iOS pre-signing). */
  unsupportedReason?: string;
}

/** One toolchain pre-flight check (rail Checks section, mobile kinds). */
export interface MobilePreflightCheck {
  label: string;
  ok: boolean;
  detail: string;
}

/** Run sub-state derived from the launch script's markers / CLI milestones. */
export type MobilePhase =
  | "resolving_device"
  | "booting_device"
  | "building"
  | "installing"
  | "launching"
  | "connected"
  | "build_failed";

/** Emitted on `portbay://mobile-phase`; `phase: null` clears the sub-state. */
export interface MobilePhaseEvent {
  id: string;
  phase: MobilePhase | null;
  detail?: string;
  ts: number;
}

/** Hydration shape from `get_mobile_phases`. */
export interface MobilePhaseInfo {
  phase: MobilePhase;
  detail?: string;
}

/** The word users see for each phase. */
export const mobilePhaseLabel: Record<MobilePhase, string> = {
  resolving_device: "Resolving device…",
  booting_device: "Booting device…",
  building: "Building…",
  installing: "Installing…",
  launching: "Launching…",
  connected: "Connected",
  build_failed: "Build failed",
};

/**
 * Dot/tint a phase maps onto, reusing the existing status tokens so the
 * mobile overlay needs no new colors: in-flight phases pulse like `starting`,
 * Connected is the green that previously lied during builds, a failed build
 * is `crashed` red.
 */
export function mobilePhaseDisplay(phase: MobilePhase): DisplayStatus {
  switch (phase) {
    case "connected":
      return "running";
    case "build_failed":
      return "crashed";
    default:
      return "starting";
  }
}

/** The four kinds whose Play is a simulator/emulator launch. */
export function isMobileType(t: ProjectType): boolean {
  return t === "flutter" || t === "xcode" || t === "android" || t === "expo";
}

export function isMobileProject(p: ProjectView): boolean {
  return isMobileType(p.type);
}

/** Picker group, in display order. */
export type TargetGroup =
  | "Ready"
  | "Simulators"
  | "Emulators"
  | "Physical devices"
  | "Other";

const GROUP_ORDER: TargetGroup[] = [
  "Ready",
  "Simulators",
  "Emulators",
  "Physical devices",
  "Other",
];

function groupOf(t: MobileRunTarget): TargetGroup {
  if (!t.unsupportedReason && (t.state === "booted" || t.state === "connected")) {
    return "Ready";
  }
  switch (t.kind) {
    case "simulator":
      return "Simulators";
    case "emulator":
      return "Emulators";
    case "physical":
      return "Physical devices";
    default:
      return "Other";
  }
}

/**
 * Group + order targets for the destination picker (Xcode's grouping:
 * ready-to-run first, then bootable simulators/emulators, physical last).
 * `query` filters by name/OS, case-insensitive. Pure — unit-tested.
 */
export function groupTargets(
  targets: MobileRunTarget[],
  query = "",
): { group: TargetGroup; targets: MobileRunTarget[] }[] {
  const q = query.trim().toLowerCase();
  const filtered = q
    ? targets.filter(
        (t) =>
          t.name.toLowerCase().includes(q) ||
          (t.osVersion ?? "").toLowerCase().includes(q),
      )
    : targets;
  const buckets = new Map<TargetGroup, MobileRunTarget[]>();
  for (const t of filtered) {
    const g = groupOf(t);
    const list = buckets.get(g) ?? [];
    list.push(t);
    buckets.set(g, list);
  }
  return GROUP_ORDER.filter((g) => buckets.has(g)).map((group) => ({
    group,
    targets: buckets.get(group)!,
  }));
}

/**
 * Human label for a pinned `MobileRunConfig.device` id when the enumerated
 * target list isn't at hand (row chips). `avd:Pixel_8` → `Pixel 8`;
 * Expo's `ios`/`android` get their picker names; anything else passes through.
 */
export function deviceShortLabel(device: string | null | undefined): string {
  if (!device) return "Auto destination";
  if (device.startsWith("avd:")) return device.slice(4).replace(/_/g, " ");
  if (device === "ios") return "iOS Simulator";
  if (device === "android") return "Android Emulator";
  return device;
}
