//! Live status events.
//!
//! Spawns a background poller that diffs PC's `/processes` snapshot
//! against its last observation and emits one `ProjectStatusEvent` per
//! changed project on the `portbay://status` channel.
//!
//! Channel name `portbay://status` follows the URI-style convention used
//! across the Tauri ecosystem and namespaces cleanly against plugin events.
//!
//! Scope deliberately small: PC status only. Caddy reconcile and
//! registry-drift events land in a separate follow-up card once the
//! reconcile loop is fleshed out.

use std::collections::HashMap;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};

use crate::commands::dto::{ProjectStatusEvent, RuntimeInfo};
use crate::process_compose::{Process, ProjectStatus};
use crate::registry::store;
use crate::state::AppState;
use crate::tray;

pub const STATUS_CHANNEL: &str = "portbay://status";

/// Cadence at which the poller wakes to check PC. Diffs are computed every
/// tick; events are emitted only on transitions, not every tick.
const POLL_INTERVAL: Duration = Duration::from_millis(1500);

#[derive(Debug, Clone, PartialEq)]
struct ObservedState {
    status: ProjectStatus,
    pid: u32,
    restarts: u32,
}

impl ObservedState {
    fn from_process(p: &Process) -> Self {
        Self {
            status: p.portbay_status(),
            pid: p.pid,
            restarts: p.restarts,
        }
    }
}

/// Spawn the status poller. Returns immediately; the task runs for the
/// lifetime of the app handle.
pub fn spawn_status_poller(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut last: HashMap<String, ObservedState> = HashMap::new();
        let mut tick = tokio::time::interval(POLL_INTERVAL);
        // First tick fires immediately; that's the right shape for a poller
        // — emit a first snapshot as soon as the daemon is reachable.
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            tick.tick().await;

            let state: tauri::State<AppState> = app.state();
            let client = {
                let g = state.pc_client.lock().expect("pc_client mutex poisoned");
                match g.clone() {
                    Some(c) => c,
                    None => continue, // daemon not up yet
                }
            };

            let Ok(processes) = client.processes().await else {
                continue; // unreachable — try again next tick
            };

            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);

            // Emit on transition; track for the next pass.
            let mut next: HashMap<String, ObservedState> = HashMap::with_capacity(processes.len());
            for p in &processes {
                let observed = ObservedState::from_process(p);
                let changed = match last.get(&p.name) {
                    Some(prev) => prev != &observed,
                    None => true, // first observation == emit
                };
                if changed {
                    let last_error = match observed.status {
                        ProjectStatus::Crashed if p.exit_code != 0 => {
                            Some(format!("Process exited with code {}.", p.exit_code))
                        }
                        ProjectStatus::Crashed => Some("Process crashed unexpectedly.".into()),
                        ProjectStatus::PortConflict => Some(
                            "Port conflict — another process is using the assigned port.".into(),
                        ),
                        ProjectStatus::Unhealthy => {
                            Some("Process is running but not passing its readiness probe.".into())
                        }
                        _ => None,
                    };
                    let event = ProjectStatusEvent {
                        id: p.name.clone(),
                        status: observed.status,
                        runtime: Some(RuntimeInfo::from_process(p)),
                        last_error,
                        ts: now,
                    };
                    let _ = app.emit(STATUS_CHANNEL, event);
                }
                next.insert(p.name.clone(), observed);
            }

            // Projects that disappeared from PC since last tick → emit stopped.
            for (id, prev) in &last {
                if !next.contains_key(id) && prev.status != ProjectStatus::Stopped {
                    let event = ProjectStatusEvent {
                        id: id.clone(),
                        status: ProjectStatus::Stopped,
                        runtime: None,
                        last_error: None,
                        ts: now,
                    };
                    let _ = app.emit(STATUS_CHANNEL, event);
                }
            }

            // Drive the menu-bar tray off the same observation that
            // feeds the UI. Pulls the latest registry order (cheap —
            // typically a single sub-10 KB JSON read) so the tray's
            // project list survives add/remove/rename without needing
            // a dedicated event channel.
            let aggregate_input: HashMap<String, ProjectStatus> = next
                .iter()
                .map(|(id, observed)| (id.clone(), observed.status))
                .collect();
            let registry = store::load_or_default(
                &state.registry_path,
                state.domain_suffix.as_str(),
            )
            .ok();
            if let Some(reg) = registry {
                tray::refresh(&app, reg.list_projects(), &aggregate_input);
            }

            last = next;
        }
    });
}
