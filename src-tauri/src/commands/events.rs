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
/// tick; events are emitted only on transitions, not every tick. 750 ms
/// keeps the perceived UI lag against tools like ServBay (which polls
/// roughly every second) competitive without saturating PC's REST API
/// (each tick costs one /processes round-trip, sub-100 ms).
const POLL_INTERVAL: Duration = Duration::from_millis(750);

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
                let mut observed = ObservedState::from_process(p);

                // If the user just asked PortBay to stop this project,
                // a non-zero exit code is almost certainly the wrapper
                // tool (npm, turbo, concurrently) translating SIGTERM
                // into `exit(1)`. Downgrade to Stopped so the row
                // doesn't paint red mid-shutdown.
                if observed.status == ProjectStatus::Crashed
                    && state.recently_stop_requested(&p.name)
                {
                    observed.status = ProjectStatus::Stopped;
                }

                let changed = match last.get(&p.name) {
                    Some(prev) => prev != &observed,
                    None => true, // first observation == emit
                };
                if changed {
                    let last_error = match observed.status {
                        ProjectStatus::Crashed => Some(crashed_summary(&state, &p.name, p.exit_code)),
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

/// Build the human-readable Crashed summary the frontend renders inline
/// under the failed project row. We tail the last few lines of the
/// project's log file and translate known error patterns into actionable
/// messages — generic "exit code N" tells the user nothing; "Port 3010
/// is already in use" tells them exactly what to fix.
///
/// Best-effort: any I/O failure falls back to the bare exit-code line.
fn crashed_summary(state: &AppState, project_id: &str, exit_code: i32) -> String {
    let log_path = state.logs_dir.join(format!("{project_id}.log"));
    let tail = tail_last_lines(&log_path, 60).unwrap_or_default();

    // Pattern → human summary. Ordered: most specific first.
    if let Some(port) = parse_eaddrinuse(&tail) {
        return format!(
            "Port {port} is already in use by another local-dev server. \
             Stop the process holding it (or change this project's port in \
             its detail panel) and try again.",
        );
    }
    if tail.iter().any(|l| {
        let lc = l.to_ascii_lowercase();
        lc.contains("command not found")
            || lc.contains("no such file or directory")
            || lc.contains("eaccess")
    }) {
        let line = tail
            .iter()
            .rev()
            .find(|l| {
                let lc = l.to_ascii_lowercase();
                lc.contains("command not found") || lc.contains("no such file")
            })
            .cloned()
            .unwrap_or_default();
        return format!(
            "Process couldn't be launched — {} \
             Check the project's start command and the runtime is installed.",
            line.trim()
        );
    }
    if tail
        .iter()
        .any(|l| l.to_ascii_lowercase().contains("permission denied"))
    {
        return "Permission denied — the start command needs access \
             to a file or port it isn't allowed to use."
            .into();
    }

    // No recognised pattern — fall back to the bare PC report plus the
    // last useful-looking line of stderr so the user has at least a
    // breadcrumb to chase.
    let last_meaningful = tail
        .iter()
        .rev()
        .find(|l| {
            let t = l.trim();
            !t.is_empty()
                && !t.starts_with('{') // JSON envelope noise from PC's logger
                && !t.contains("ELIFECYCLE")
        })
        .cloned();
    match last_meaningful {
        Some(line) if !line.trim().is_empty() => {
            format!("Process exited with code {exit_code}. {}", line.trim())
        }
        _ => format!("Process exited with code {exit_code}."),
    }
}

/// Read the last `n` text lines of a file without loading the whole
/// thing. Returns the lines newest-last; an I/O error returns None.
fn tail_last_lines(path: &std::path::Path, n: usize) -> Option<Vec<String>> {
    let contents = std::fs::read_to_string(path).ok()?;
    Some(
        contents
            .lines()
            .rev()
            .take(n)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|s| {
                // PC's log lines are JSON envelopes; strip to the
                // inner `message` field when present so the user sees
                // the actual stderr text, not envelope noise.
                extract_pc_message(s).unwrap_or_else(|| s.to_string())
            })
            .collect(),
    )
}

/// Process Compose's log writer wraps each child stderr/stdout line
/// in a JSON envelope:
/// `{"level":"error","process":"x","replica":0,"message":"..."}`.
/// Pull the `message` out so pattern matching works on the raw stderr.
fn extract_pc_message(line: &str) -> Option<String> {
    if !line.starts_with('{') {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    v.get("message")
        .and_then(|m| m.as_str())
        .map(|s| s.to_string())
}

/// Extract a port number from an EADDRINUSE-style stderr line.
/// Handles the two common Node/Next formats:
///   "Error: listen EADDRINUSE: address already in use :::3010"
///   "port: 3010"
fn parse_eaddrinuse(lines: &[String]) -> Option<u16> {
    for line in lines {
        let lc = line.to_ascii_lowercase();
        if !lc.contains("eaddrinuse") && !lc.contains("address already in use") {
            continue;
        }
        // First try the inline ":::PORT" form.
        if let Some(idx) = line.rfind(":::") {
            let tail = &line[idx + 3..];
            let port: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(p) = port.parse::<u16>() {
                return Some(p);
            }
        }
        // Then the "port: N" form.
        if let Some(idx) = lc.find("port:") {
            let tail = &line[idx + 5..];
            let port: String = tail
                .chars()
                .skip_while(|c| !c.is_ascii_digit())
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if let Ok(p) = port.parse::<u16>() {
                return Some(p);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_eaddrinuse_handles_node_format() {
        let lines = vec![
            "Error: listen EADDRINUSE: address already in use :::3010".to_string(),
        ];
        assert_eq!(parse_eaddrinuse(&lines), Some(3010));
    }

    #[test]
    fn parse_eaddrinuse_handles_kv_format() {
        let lines = vec!["  port: 5432".to_string(), "  errno: -48".to_string()];
        // Without the EADDRINUSE marker on the same line, we shouldn't
        // match — keeps innocent "port: N" hits from project banners
        // (e.g. "Listening on port: 3000") out of the heuristic.
        assert_eq!(parse_eaddrinuse(&lines), None);
    }

    #[test]
    fn parse_eaddrinuse_returns_none_without_marker() {
        // Bare "port: N" lines (banners, prompts, "Listening on port: 3000")
        // should not be misread as a conflict — the EADDRINUSE marker is
        // the load-bearing signal.
        let lines = vec!["Server listening on port: 3000".to_string()];
        assert_eq!(parse_eaddrinuse(&lines), None);
    }

    #[test]
    fn extract_pc_message_pulls_out_inner_text() {
        let json = r#"{"level":"error","process":"x","message":"boom"}"#;
        assert_eq!(extract_pc_message(json).as_deref(), Some("boom"));
    }

    #[test]
    fn extract_pc_message_passes_plain_lines() {
        assert_eq!(extract_pc_message("not json"), None);
    }
}
