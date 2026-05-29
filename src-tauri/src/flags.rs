//! Early-access feature-flag system — core half.
//!
//! Mirrors `src/lib/stores/flags.svelte.ts`: every feature has a rollout
//! [`Stage`] — `Ga` (everyone) or `Early` (only a Pro account that has opted
//! into early access). Register a feature in [`stage`] and gate it with
//! [`enabled`]; flip it to `Ga` to launch it to everyone with no change at the
//! call sites. Keep this table in lockstep with the client store.
//!
//! Source of truth is ship-time (the table below). Backend-driven overrides
//! (toggling an early flag without a release) are intentionally deferred — see
//! the card's note; the matrix doesn't promise them on day one.

use crate::entitlements::Entitlements;

/// How widely a feature is rolled out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Generally available — on for everyone.
    Ga,
    /// Early access — on only for a Pro account that has opted in.
    Early,
}

/// The ship-time flag table. Empty today: Pro ships with the machinery, no
/// early features yet. Add `"feature-id" => Stage::Early` here (and in the
/// client store) to gate a feature behind early access.
fn stage(feature: &str) -> Stage {
    // Early-access feature table. Empty today — add ids here (and in the
    // client store) to gate a feature, e.g. `"experimental-tunnels"`.
    const EARLY: &[&str] = &[];
    if EARLY.contains(&feature) {
        Stage::Early
    } else {
        Stage::Ga
    }
}

/// Pure resolution, unit-tested independently of the (currently empty) table.
fn resolve(stage: Stage, ent: &Entitlements, opted_in: bool) -> bool {
    match stage {
        Stage::Ga => true,
        Stage::Early => ent.early_access && opted_in,
    }
}

/// Whether `feature` is enabled for an account with `ent`, given the user's
/// early-access `opted_in` preference. Unknown features are GA.
pub fn enabled(feature: &str, ent: &Entitlements, opted_in: bool) -> bool {
    resolve(stage(feature), ent, opted_in)
}

/// Whether `feature` is gated behind early access at all.
pub fn is_early_access(feature: &str) -> bool {
    matches!(stage(feature), Stage::Early)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entitlements::{anonymous_entitlements, free_entitlements};

    fn pro_ent() -> Entitlements {
        Entitlements {
            max_projects: None,
            max_devices: Some(2),
            sync: true,
            custom_port_cors: true,
            mail: "full".into(),
            early_access: true,
            priority_support: true,
        }
    }

    #[test]
    fn unknown_feature_is_ga_for_everyone() {
        assert!(enabled("does-not-exist", &anonymous_entitlements(), false));
        assert!(enabled("does-not-exist", &free_entitlements(), false));
        assert!(enabled("does-not-exist", &pro_ent(), false));
        assert!(!is_early_access("does-not-exist"));
    }

    #[test]
    fn ga_is_on_regardless_of_tier_or_opt_in() {
        assert!(resolve(Stage::Ga, &anonymous_entitlements(), false));
        assert!(resolve(Stage::Ga, &pro_ent(), false));
    }

    #[test]
    fn early_requires_both_entitlement_and_opt_in() {
        // Pro + opted in → on.
        assert!(resolve(Stage::Early, &pro_ent(), true));
        // Pro but not opted in → off (the toggle is meaningful).
        assert!(!resolve(Stage::Early, &pro_ent(), false));
        // Opted in but not entitled (free/anon) → off.
        assert!(!resolve(Stage::Early, &free_entitlements(), true));
        assert!(!resolve(Stage::Early, &anonymous_entitlements(), true));
    }
}
