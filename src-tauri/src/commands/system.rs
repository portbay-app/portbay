//! System-level commands — `doctor`, `tail_logs`.
//!
//! `doctor` mirrors the CLI's `cmd_doctor` JSON output shape so the GUI
//! and CLI report the same findings to the same support requests.

use tauri::State;

use crate::commands::dto::{DoctorFinding, DoctorReport, DoctorVerdict};
use crate::commands::projects::load_registry;
use crate::error::{AppError, AppResult};
use crate::hosts::HostsManager;
use crate::state::AppState;

#[tauri::command]
pub async fn doctor(state: State<'_, AppState>) -> AppResult<DoctorReport> {
    let mut findings = Vec::new();

    // Registry
    match load_registry(&state) {
        Ok(reg) => findings.push(DoctorFinding {
            check: "registry".into(),
            verdict: DoctorVerdict::Ok,
            detail: format!(
                "{} project(s), v{} schema, suffix .{}",
                reg.list_projects().len(),
                reg.version,
                reg.domain_suffix
            ),
        }),
        Err(e) => findings.push(DoctorFinding {
            check: "registry".into(),
            verdict: DoctorVerdict::Fail,
            detail: e.to_string(),
        }),
    }

    // PC daemon
    let pc_client = state.pc_client.lock().expect("pc_client mutex poisoned").clone();
    let pc_finding = match pc_client {
        None => DoctorFinding {
            check: "process-compose".into(),
            verdict: DoctorVerdict::Warn,
            detail: "not started yet".into(),
        },
        Some(c) => match c.live().await {
            Ok(true) => DoctorFinding {
                check: "process-compose".into(),
                verdict: DoctorVerdict::Ok,
                detail: "alive".into(),
            },
            Ok(false) => DoctorFinding {
                check: "process-compose".into(),
                verdict: DoctorVerdict::Warn,
                detail: "not reachable".into(),
            },
            Err(e) => DoctorFinding {
                check: "process-compose".into(),
                verdict: DoctorVerdict::Warn,
                detail: e.to_string(),
            },
        },
    };
    findings.push(pc_finding);

    // Tools on PATH
    for tool in ["mkcert", "caddy", "process-compose"] {
        match which::which(tool) {
            Ok(p) => findings.push(DoctorFinding {
                check: format!("tool: {tool}"),
                verdict: DoctorVerdict::Ok,
                detail: p.display().to_string(),
            }),
            Err(_) => findings.push(DoctorFinding {
                check: format!("tool: {tool}"),
                verdict: DoctorVerdict::Warn,
                detail: "not found on PATH (bundled .app uses its sidecar — this only matters for CLI standalone use)".into(),
            }),
        }
    }

    // /etc/hosts reconcile state
    match (HostsManager::system().list_managed(), load_registry(&state)) {
        (Ok(entries), Ok(reg)) => {
            use std::collections::HashSet;
            let expected: HashSet<String> =
                reg.list_projects().iter().map(|p| p.hostname.clone()).collect();
            let present: HashSet<String> = entries.iter().map(|e| e.hostname.clone()).collect();
            let missing = expected.difference(&present).count();
            let orphan = present.difference(&expected).count();
            let verdict = if missing == 0 && orphan == 0 {
                DoctorVerdict::Ok
            } else {
                DoctorVerdict::Warn
            };
            let detail = if missing == 0 && orphan == 0 {
                format!("{} entries, all match registry", entries.len())
            } else {
                format!(
                    "{} entries (missing: {missing}, orphan: {orphan}). Run `sudo portbay hosts reconcile` to fix.",
                    entries.len()
                )
            };
            findings.push(DoctorFinding {
                check: "/etc/hosts".into(),
                verdict,
                detail,
            });
        }
        (Err(e), _) => findings.push(DoctorFinding {
            check: "/etc/hosts".into(),
            verdict: DoctorVerdict::Warn,
            detail: e.to_string(),
        }),
        (_, Err(_)) => {
            // Registry load already errored above; nothing useful to add here.
        }
    }

    Ok(DoctorReport { findings })
}

/// `tail_logs(id, limit, offset)` — static log tail from PC's buffer.
///
/// For live streaming, see card #10's Channel<T>-based follow mode — this
/// command intentionally returns a snapshot.
#[tauri::command]
pub async fn tail_logs(
    state: State<'_, AppState>,
    id: String,
    #[allow(non_snake_case)] limit: Option<u32>,
    #[allow(non_snake_case)] offset: Option<u64>,
) -> AppResult<Vec<String>> {
    let client = state.pc_client()?;
    let lines = client
        .logs(&id, offset.unwrap_or(0), limit.unwrap_or(200))
        .await
        .map_err(AppError::Pc)?;
    Ok(lines)
}
