//! Reconcile loop — drives the live system (PC YAML, Caddy admin, hosts,
//! certs) toward the on-disk Registry.
//!
//! Ticks fire on three triggers:
//!
//! 1. **CRUD commands** call [`Reconciler::mark_dirty`] after every
//!    `save_registry`. The tick runs in the background so the user's
//!    command toast returns immediately.
//! 2. **Cold boot** — `lib::run::setup` kicks one tick after both
//!    sidecars are up so the bootstrap state applies the registry-
//!    derived Caddy config + hosts + certs.
//! 3. **Periodic safety tick** every 30 s catches drift from the CLI
//!    (which writes the same registry file) or any external `/etc/hosts`
//!    edits.
//!
//! Multiple `mark_dirty` calls during a single tick coalesce: tokio's
//! `Notify` stores at most one permit, so a burst of CRUD calls produces
//! one extra tick after the in-flight one completes.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager};
use tokio::sync::{Mutex, Notify};

use crate::registry::{store, Registry};
use crate::state::AppState;

pub mod caddy;
pub mod certs;
pub mod hosts;
pub mod pc;
pub mod report;

pub use pc::{build_initial_yaml, default_yaml_path};
pub use report::{ReconcileReport, StepOutcome};

/// Reconciler state.
///
/// Construct once at app boot via [`Reconciler::new`]. Inputs the
/// reconciler needs that don't change at runtime (logs dir, mkcert
/// wrapper) live on the struct; per-tick state (cached hashes for each
/// sub-step) lives behind a tokio mutex so it can be held across the
/// async PC restart and Caddy load.
pub struct Reconciler {
    inner: Mutex<Inner>,
    notify: Arc<Notify>,
}

struct Inner {
    logs_dir: PathBuf,
    yaml_path: PathBuf,
    pc_cache: pc::PcCache,
    caddy_cache: caddy::CaddyCache,
    hosts_cache: hosts::HostsCache,
    certs_cache: certs::CertsCache,
}

impl Reconciler {
    pub fn new(logs_dir: PathBuf, yaml_path: PathBuf) -> Self {
        Self {
            inner: Mutex::new(Inner {
                logs_dir,
                yaml_path,
                pc_cache: pc::PcCache::default(),
                caddy_cache: caddy::CaddyCache::default(),
                hosts_cache: hosts::HostsCache::default(),
                certs_cache: certs::CertsCache::default(),
            }),
            notify: Arc::new(Notify::new()),
        }
    }

    /// Signal the background loop to run a tick at the next opportunity.
    /// Cheap, non-blocking. Multiple marks within one tick coalesce.
    pub fn mark_dirty(&self) {
        self.notify.notify_one();
    }

    /// Pre-populate the PC sub-step's cache with the hash of the YAML
    /// `lib::run::setup` already wrote + booted against. Without this,
    /// the first tick's PC sub-reconciler sees an empty cache, decides
    /// to "apply", and triggers a redundant restart of the PC daemon
    /// that boot_pc just spawned a few hundred milliseconds earlier.
    pub async fn prime_pc_cache_from_yaml(&self, yaml: &str) {
        let mut inner = self.inner.lock().await;
        inner.pc_cache.prime(pc::hash_yaml(yaml));
    }

    /// Drop the Caddy sub-step's cached hash so the next tick re-POSTs
    /// `/load`. Used by `reissue_cert` — the cert files on disk changed
    /// but the config-JSON hash didn't (same paths), so Caddy wouldn't
    /// otherwise re-read the certs.
    pub async fn invalidate_caddy_cache(&self) {
        let mut inner = self.inner.lock().await;
        inner.caddy_cache.invalidate();
    }

    /// Run one tick to completion. Folds per-step outcomes into a
    /// report; never panics. Callers usually want [`mark_dirty`] and let
    /// the loop run this in the background — direct invocation exists
    /// for the `reconcile_hosts` command and tests.
    pub async fn tick(&self, app: &AppHandle) -> ReconcileReport {
        let state: tauri::State<'_, AppState> = app.state();
        let started_at_ms = ReconcileReport::now();
        let started = Instant::now();

        // Load the registry fresh — the on-disk store is the source of
        // truth and may have drifted (CLI writes go straight to disk).
        let reg = match store::load_or_default(&state.registry_path, &state.domain_suffix) {
            Ok(r) => r,
            Err(e) => {
                let err = StepOutcome::failed(format!("registry load: {e}"));
                return ReconcileReport {
                    started_at_ms,
                    duration_ms: started.elapsed().as_millis() as u64,
                    certs: err.clone(),
                    pc: err.clone(),
                    caddy: err.clone(),
                    hosts: err,
                };
            }
        };

        let mut inner = self.inner.lock().await;

        // Order matters — see report.rs module-docs. Sub-reconcilers
        // borrow disjoint fields of `inner`; we split via a destructure
        // so the borrow checker doesn't conflate them.
        let Inner {
            logs_dir,
            yaml_path,
            pc_cache,
            caddy_cache,
            hosts_cache,
            certs_cache,
        } = &mut *inner;

        let certs_result = certs::reconcile(&reg, state.mkcert.as_ref(), certs_cache);

        let pc_outcome = pc::reconcile(&reg, logs_dir, yaml_path, &state, app, pc_cache).await;

        let caddy_outcome = caddy::reconcile(&reg, &certs_result.lookup, &state, caddy_cache).await;

        // When dnsmasq's `/etc/resolver/<suffix>` is in place and
        // points at the running daemon, hostname → loopback routing
        // is handled by DNS and `/etc/hosts` becomes redundant. Let
        // the hosts sub-step Skip in that case.
        let dns_routing_active = {
            let port = state.dnsmasq.lock().expect("dnsmasq mutex poisoned").port();
            crate::dnsmasq::resolver::is_installed(&state.domain_suffix, port)
        };
        let hosts_outcome = hosts::reconcile(&reg, hosts_cache, dns_routing_active);

        let report = ReconcileReport {
            started_at_ms,
            duration_ms: started.elapsed().as_millis() as u64,
            certs: certs_result.outcome,
            pc: pc_outcome,
            caddy: caddy_outcome,
            hosts: hosts_outcome,
        };

        log_report(&report, reg_summary(&reg));
        report
    }
}

fn reg_summary(reg: &Registry) -> String {
    format!(
        "{} project(s), suffix .{}",
        reg.list_projects().len(),
        reg.domain_suffix
    )
}

fn log_report(r: &ReconcileReport, reg_summary: String) {
    if r.any_failure() {
        tracing::warn!(
            duration_ms = r.duration_ms,
            registry = %reg_summary,
            certs = ?r.certs,
            pc = ?r.pc,
            caddy = ?r.caddy,
            hosts = ?r.hosts,
            "reconcile tick (with failures)"
        );
    } else {
        tracing::info!(
            duration_ms = r.duration_ms,
            registry = %reg_summary,
            certs = ?r.certs,
            pc = ?r.pc,
            caddy = ?r.caddy,
            hosts = ?r.hosts,
            "reconcile tick"
        );
    }
}

/// Spawn the periodic reconcile loop. Returns immediately; the loop runs
/// for the lifetime of the app. One tick fires on each `mark_dirty()`
/// notification or every `period`, whichever comes first.
///
/// Call once from `lib::run::setup` after both sidecars are up. To
/// kick off an immediate first tick (apply the registry to the freshly
/// booted bootstrap state), call `state.reconciler.mark_dirty()` before
/// or just after spawning.
pub fn spawn_reconcile_loop(app: AppHandle, period: Duration) {
    tauri::async_runtime::spawn(async move {
        let notify = {
            let state: tauri::State<'_, AppState> = app.state();
            state.reconciler.notify.clone()
        };

        loop {
            tokio::select! {
                _ = notify.notified() => {},
                _ = tokio::time::sleep(period) => {},
            }
            let state: tauri::State<'_, AppState> = app.state();
            let _ = state.reconciler.tick(&app).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_dirty_does_not_panic_on_construction() {
        let r = Reconciler::new(PathBuf::from("/tmp"), PathBuf::from("/tmp/x.yaml"));
        r.mark_dirty();
        r.mark_dirty();
        r.mark_dirty();
    }

    #[tokio::test]
    async fn notify_coalesces_multiple_marks_into_one_wake() {
        let r = Reconciler::new(PathBuf::from("/tmp"), PathBuf::from("/tmp/x.yaml"));
        let n = r.notify.clone();

        // Fire three marks before any waiter exists.
        r.mark_dirty();
        r.mark_dirty();
        r.mark_dirty();

        // First notified() consumes the single stored permit.
        n.notified().await;

        // A second notified() now has no permit; verify it would block
        // by racing it against a 50 ms timeout — should time out.
        let timed_out = tokio::time::timeout(Duration::from_millis(50), n.notified())
            .await
            .is_err();
        assert!(timed_out, "second notified() should have no permit waiting");
    }
}
