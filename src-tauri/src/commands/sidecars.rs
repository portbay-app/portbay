//! Sidecar health commands.
//!
//! Feeds the top-of-Dashboard sidecar health row (card #5) and the sidebar
//! footer pills. Polled by the frontend on a 3s cadence; also pushed
//! via `portbay://status` events when the reconcile loop notices a change.

use std::net::Ipv4Addr;

use tauri::{AppHandle, State};

use crate::commands::dto::{SidecarHealth, SidecarState, SidecarStatus};
use crate::commands::projects::load_registry;
use crate::error::{AppError, AppResult};
use crate::hosts::HostsManager;
use crate::state::AppState;

#[tauri::command]
pub async fn sidecar_status(state: State<'_, AppState>) -> AppResult<SidecarHealth> {
    let process_compose = pc_status(&state).await;
    let caddy = caddy_status(&state).await;
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

/// `restart_pc()` — stop the bundled process-compose sidecar and start
/// a fresh one against the current bootstrap config. The action button on
/// the process-compose sidecar card maps to this command.
#[tauri::command]
pub async fn restart_pc(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    state.shutdown_pc();
    state.boot_pc(&app)
}

/// `restart_caddy()` — stop the bundled Caddy sidecar and start a fresh
/// one against the bootstrap admin-only config. The action button on the
/// Caddy sidecar card maps to this command. Waits for the admin endpoint
/// to come back up before returning (same readiness gate as boot).
#[tauri::command]
pub async fn restart_caddy(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    state.shutdown_caddy();
    state.boot_caddy(&app).await
}

/// `reconcile_hosts()` — overwrite the PortBay-managed block in
/// `/etc/hosts` with one entry per registered project. The hosts
/// sidecar card's action button maps to this command. Requires sudo;
/// surfaces a friendly envelope when permission is denied.
#[tauri::command]
pub async fn reconcile_hosts(state: State<'_, AppState>) -> AppResult<u32> {
    let registry = load_registry(&state)?;
    let pairs: Vec<(String, Ipv4Addr)> = registry
        .list_projects()
        .iter()
        .map(|p| (p.hostname.clone(), Ipv4Addr::LOCALHOST))
        .collect();
    let count = pairs.len() as u32;
    HostsManager::system()
        .replace_all(pairs)
        .map_err(AppError::Hosts)?;
    Ok(count)
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

async fn caddy_status(state: &AppState) -> SidecarStatus {
    let client = state
        .caddy_client
        .lock()
        .expect("caddy_client mutex poisoned")
        .clone();
    let (status, detail) = match client {
        None => (SidecarState::Stopped, Some("not started".into())),
        Some(c) => match c.is_alive().await {
            Ok(true) => (SidecarState::Running, Some("alive".into())),
            Ok(false) => (SidecarState::Unreachable, Some("unreachable".into())),
            Err(e) => (SidecarState::Unreachable, Some(e.to_string())),
        },
    };
    SidecarStatus {
        name: "caddy",
        status,
        detail,
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
