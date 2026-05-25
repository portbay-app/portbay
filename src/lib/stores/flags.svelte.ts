/**
 * flags — the early-access feature-flag system.
 *
 * Each feature has a stage: `"ga"` (everyone) or `"early"` (only a Pro account
 * with the `early_access` entitlement that has opted into early access). Register
 * a feature here and gate its UI with `flags.enabled("my-feature")`; flip it to
 * `"ga"` to launch it to everyone with no code change at the call sites. Keep
 * this table in lockstep with the Rust `flags` module.
 *
 * Reads the entitlement store and the early-access opt-in preference, so it
 * tracks sign-in / upgrade / opt-in live.
 */

import { entitlements } from "./entitlements.svelte";
import { preferences } from "./preferences.svelte";

type Stage = "ga" | "early";

/** Feature → rollout stage. Add early-access features here. */
const REGISTRY: Record<string, Stage> = {
  // "experimental-tunnels": "early",
};

export const flags = {
  /** Whether a feature is currently enabled for this user. Unknown = GA. */
  enabled(feature: string): boolean {
    const stage = REGISTRY[feature] ?? "ga";
    if (stage === "ga") return true;
    return entitlements.allows("early_access") && preferences.value.earlyAccessOptIn;
  },
  /** Whether a feature exists and is gated behind early access. */
  isEarlyAccess(feature: string): boolean {
    return REGISTRY[feature] === "early";
  },
};
