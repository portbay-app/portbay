//! Per-tick outcome envelopes.
//!
//! Each sub-reconciler returns a [`StepOutcome`]; a tick folds the four
//! per-step outcomes into a [`ReconcileReport`]. The report is logged
//! once per tick via `tracing` and is the primary diagnostic surface for
//! the reconcile loop — there's no separate metrics or event channel.
//!
//! `StepOutcome` deliberately uses owned `String` rather than typed
//! variants. The reconciler's job is to converge state and log progress,
//! not to expose a structured error catalogue to callers — those flow
//! through the existing `AppError`/`CommandError` envelope on the next
//! user-facing action.

use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub enum StepOutcome {
    /// The desired state already matched the live state. No I/O performed.
    Skipped { reason: String },

    /// A real change was applied. `detail` is a single-line human summary
    /// suitable for the log line and the future "last reconcile" UI hint.
    Applied { detail: String },

    /// The sub-step failed. Folded into the report rather than raised so
    /// independent sub-reconcilers stay independent — a hosts permission
    /// failure must not block the PC restart that's already in flight.
    Failed { error: String },
}

impl StepOutcome {
    pub fn skipped(reason: impl Into<String>) -> Self {
        Self::Skipped {
            reason: reason.into(),
        }
    }

    pub fn applied(detail: impl Into<String>) -> Self {
        Self::Applied {
            detail: detail.into(),
        }
    }

    pub fn failed(error: impl Into<String>) -> Self {
        Self::Failed {
            error: error.into(),
        }
    }

    pub fn is_failure(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }
}

/// The outcome of one full reconcile tick.
///
/// The four sub-step outcomes are in apply order — certs run before PC
/// because Caddy needs cert files on disk, PC before Caddy because Caddy
/// proxies to PC's upstreams, hosts last because DNS resolving without
/// a serving Caddy is worse than a missing host entry.
#[derive(Debug, Clone)]
pub struct ReconcileReport {
    pub started_at_ms: u64,
    pub duration_ms: u64,
    pub certs: StepOutcome,
    pub pc: StepOutcome,
    pub caddy: StepOutcome,
    pub hosts: StepOutcome,
}

impl ReconcileReport {
    pub(super) fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    pub fn any_failure(&self) -> bool {
        self.certs.is_failure()
            || self.pc.is_failure()
            || self.caddy.is_failure()
            || self.hosts.is_failure()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_constructors_round_trip() {
        let s = StepOutcome::skipped("unchanged");
        let a = StepOutcome::applied("3 projects");
        let f = StepOutcome::failed("502 from caddy");
        assert!(matches!(s, StepOutcome::Skipped { .. }));
        assert!(matches!(a, StepOutcome::Applied { .. }));
        assert!(matches!(f, StepOutcome::Failed { .. }));
        assert!(!s.is_failure());
        assert!(!a.is_failure());
        assert!(f.is_failure());
    }

    #[test]
    fn report_any_failure_flags_at_least_one_failed_step() {
        let r = ReconcileReport {
            started_at_ms: 0,
            duration_ms: 1,
            certs: StepOutcome::applied("issued 1"),
            pc: StepOutcome::skipped("unchanged"),
            caddy: StepOutcome::failed("post /load returned 500"),
            hosts: StepOutcome::skipped("unchanged"),
        };
        assert!(r.any_failure());
    }

    #[test]
    fn report_any_failure_is_false_when_all_green() {
        let r = ReconcileReport {
            started_at_ms: 0,
            duration_ms: 1,
            certs: StepOutcome::skipped("unchanged"),
            pc: StepOutcome::skipped("unchanged"),
            caddy: StepOutcome::skipped("unchanged"),
            hosts: StepOutcome::skipped("unchanged"),
        };
        assert!(!r.any_failure());
    }
}
