//! Sidecar health commands.
//!
//! Feeds the top-of-Dashboard sidecar health row (card #5) and the sidebar
//! footer pills. Polled by the frontend on a 3s cadence; also pushed
//! via `portbay://status` events when the reconcile loop notices a change.

use tauri::State;

use crate::commands::dto::{SidecarHealth, SidecarState, SidecarStatus};
use crate::error::AppResult;
use crate::hosts::HostsManager;
use crate::state::AppState;

#[tauri::command]
pub async fn sidecar_status(state: State<'_, AppState>) -> AppResult<SidecarHealth> {
    let process_compose = pc_status(&state).await;
    let caddy = caddy_status();
    let mkcert_ca = mkcert_status();
    let hosts_helper = hosts_status();

    Ok(SidecarHealth {
        process_compose,
        caddy,
        mkcert_ca,
        hosts_helper,
    })
}

/// `pc_alive()` — minimal liveness check. Cheaper than `sidecar_status`.
#[tauri::command]
pub async fn pc_alive(state: State<'_, AppState>) -> AppResult<bool> {
    let Ok(client) = state.pc_client() else {
        return Ok(false);
    };
    Ok(client.live().await?)
}

async fn pc_status(state: &AppState) -> SidecarStatus {
    let client = state.pc_client.lock().expect("pc_client mutex poisoned").clone();
    let (status, detail) = match client {
        None => (SidecarState::Stopped, Some("not started".into())),
        Some(c) => match c.live().await {
            Ok(true) => (SidecarState::Running, Some("alive".into())),
            Ok(false) => (SidecarState::Unreachable, Some("unreachable".into())),
            Err(e) => (SidecarState::Unreachable, Some(e.to_string())),
        },
    };
    SidecarStatus {
        name: "process-compose",
        status,
        detail,
        last_error: None,
    }
}

fn caddy_status() -> SidecarStatus {
    // Caddy sidecar isn't wired into the GUI's setup() yet — Phase 2
    // sidecar wiring lands alongside card #5. For now we report `stopped`
    // honestly rather than pretending it's running.
    let installed = which::which("caddy").is_ok();
    SidecarStatus {
        name: "caddy",
        status: if installed {
            SidecarState::Stopped
        } else {
            SidecarState::NotInstalled
        },
        detail: if installed {
            Some("not started yet (wired in card #5)".into())
        } else {
            Some("not found on PATH".into())
        },
        last_error: None,
    }
}

fn mkcert_status() -> SidecarStatus {
    let installed = which::which("mkcert").is_ok();
    SidecarStatus {
        name: "mkcert",
        status: if installed {
            SidecarState::Running
        } else {
            SidecarState::NotInstalled
        },
        detail: if installed {
            Some("found on PATH".into())
        } else {
            Some("not found — install with `brew install mkcert`".into())
        },
        last_error: None,
    }
}

fn hosts_status() -> SidecarStatus {
    match HostsManager::system().list_managed() {
        Ok(entries) => SidecarStatus {
            name: "hosts",
            status: SidecarState::Running,
            detail: Some(format!("{} managed entries", entries.len())),
            last_error: None,
        },
        Err(e) => SidecarStatus {
            name: "hosts",
            status: SidecarState::Unreachable,
            detail: None,
            last_error: Some(e.to_string()),
        },
    }
}
