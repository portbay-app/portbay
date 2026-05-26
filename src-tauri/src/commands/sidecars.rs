//! Sidecar health commands.
//!
//! Feeds the top-of-Dashboard sidecar health row (card #5) and the sidebar
//! footer pills. Polled by the frontend on a 3s cadence; also pushed
//! via `portbay://status` events when the reconcile loop notices a change.

use tauri::{AppHandle, State};

use crate::commands::dto::{SidecarHealth, SidecarState, SidecarStatus};
use crate::error::{AppError, AppResult};
use crate::hosts::HostsManager;
use crate::reconciler::StepOutcome;
use crate::state::AppState;

#[tauri::command]
pub async fn sidecar_status(app: AppHandle, state: State<'_, AppState>) -> AppResult<SidecarHealth> {
    let process_compose = pc_status(&state).await;
    let caddy = caddy_status(&state).await;
    let mkcert_ca = mkcert_status(&state);
    let dnsmasq = dnsmasq_status(&state);
    let mailpit = mailpit_status(&app, &state);
    let hosts_helper = hosts_status();

    Ok(SidecarHealth {
        process_compose,
        caddy,
        mkcert_ca,
        dnsmasq,
        mailpit,
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
/// a fresh one against the registry-derived YAML on disk. The action
/// button on the process-compose sidecar card maps to this command.
#[tauri::command]
pub async fn restart_pc(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    let yaml_path = crate::reconciler::default_yaml_path().map_err(AppError::Io)?;
    state.shutdown_pc();
    state.boot_pc(&app, &yaml_path)
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

/// `reconcile_hosts()` — force one immediate reconcile tick and return
/// the count of hostnames the hosts step wrote (or would have written).
/// Wired to the hosts sidecar card's action button. Replaces the older
/// inline `HostsManager::replace_all` path so the GUI button uses the
/// same code path as the periodic safety tick.
#[tauri::command]
pub async fn reconcile_hosts(app: AppHandle, state: State<'_, AppState>) -> AppResult<u32> {
    let report = state.reconciler.tick(&app).await;
    let hostname_count = HostsManager::system()
        .list_managed()
        .map(|v| v.len() as u32)
        .unwrap_or(0);
    match report.hosts {
        StepOutcome::Failed { error } => Err(AppError::Internal(format!("hosts: {error}"))),
        _ => Ok(hostname_count),
    }
}

async fn pc_status(state: &AppState) -> SidecarStatus {
    let client = state
        .pc_client
        .lock()
        .expect("pc_client mutex poisoned")
        .clone();
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
        // Report the public edge ports Caddy serves (HTTPS :443, HTTP :80 — the
        // standard ports the reconciler binds via find_free_https_port/`http_port`).
        // The dashboard card surfaces the leading number as Caddy's port; a bare
        // "alive" left it showing a dash.
        Some(c) => match c.is_alive().await {
            Ok(true) => (SidecarState::Running, Some("https :443 · http :80".into())),
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

fn mkcert_status(state: &AppState) -> SidecarStatus {
    let Some(mkcert) = state.mkcert.as_ref() else {
        return SidecarStatus {
            name: "mkcert",
            status: SidecarState::NotInstalled,
            detail: Some("bundled binary not found".into()),
            last_error: None,
        };
    };

    if mkcert.is_ca_installed() {
        SidecarStatus {
            name: "mkcert",
            status: SidecarState::Running,
            detail: Some("CA installed in system keychain".into()),
            last_error: None,
        }
    } else {
        SidecarStatus {
            name: "mkcert",
            status: SidecarState::Stopped,
            detail: Some("CA not installed — click Install local CA".into()),
            last_error: None,
        }
    }
}

fn dnsmasq_status(state: &AppState) -> SidecarStatus {
    let (running, port) = {
        let guard = state.dnsmasq.lock().expect("dnsmasq mutex poisoned");
        (guard.is_running(), guard.port())
    };
    if running {
        return SidecarStatus {
            name: "dnsmasq",
            status: SidecarState::Running,
            detail: Some(format!("listening on 127.0.0.1:{port}")),
            last_error: None,
        };
    }

    // Not running. Distinguish "binary missing" (NotInstalled) from
    // "binary present but didn't start" (Stopped) so the GUI can hint
    // appropriately.
    if which::which("dnsmasq").is_ok() {
        SidecarStatus {
            name: "dnsmasq",
            status: SidecarState::Stopped,
            detail: Some("binary present, not started".into()),
            last_error: None,
        }
    } else {
        SidecarStatus {
            name: "dnsmasq",
            status: SidecarState::NotInstalled,
            detail: Some("install via `brew install dnsmasq` or bundle a sidecar".into()),
            last_error: None,
        }
    }
}

fn mailpit_status(app: &AppHandle, state: &AppState) -> SidecarStatus {
    let (running, smtp, ui) = {
        let guard = state.mailpit.lock().expect("mailpit mutex poisoned");
        (guard.is_running(), guard.smtp_port(), guard.ui_port())
    };
    if running {
        return SidecarStatus {
            name: "mailpit",
            status: SidecarState::Running,
            detail: Some(format!("smtp :{smtp} · ui :{ui}")),
            last_error: None,
        };
    }
    // Sidecar-aware: the bundled binary isn't on PATH, so a PATH-only check
    // would wrongly report NotInstalled for every shipped build.
    if crate::mailpit::binary_available(app) {
        SidecarStatus {
            name: "mailpit",
            status: SidecarState::Stopped,
            detail: Some("binary present, not started".into()),
            last_error: None,
        }
    } else {
        SidecarStatus {
            name: "mailpit",
            status: SidecarState::NotInstalled,
            detail: Some("install via `brew install mailpit` or bundle a sidecar".into()),
            last_error: None,
        }
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
