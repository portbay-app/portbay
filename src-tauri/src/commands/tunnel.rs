//! Cloudflare Tunnel commands.
//!
//! Three surfaces:
//!
//! - `start_tunnel(id)` — spawn cloudflared against the project's
//!   URL, block until the public `trycloudflare.com` URL is
//!   announced, and return the full status.
//! - `stop_tunnel(id)` — kill the per-project cloudflared child.
//! - `list_tunnels()` — every active tunnel + its public URL.
//! - `tunnel_status(id)` — single-project lookup (used by polling
//!   modals while the URL is still being assigned).

use tauri::{AppHandle, State};

use crate::commands::projects::load_registry;
use crate::error::{AppError, AppResult};
use crate::registry::ProjectId;
use crate::state::AppState;
use crate::tunnel::TunnelStatus;

#[tauri::command]
pub async fn start_tunnel(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> AppResult<TunnelStatus> {
    // Resolve the project's URL from the registry so the GUI doesn't
    // have to know the scheme/hostname mapping.
    let registry = load_registry(&state)?;
    let project = registry
        .get_project(&ProjectId::new(&id))
        .ok_or_else(|| AppError::NotFound(id.clone()))?;
    let scheme = if project.https { "https" } else { "http" };
    let upstream = format!("{scheme}://{}", project.hostname);

    // Spawn + pull out the URL handle under one brief lock — we then
    // drop the lock before awaiting, because `MutexGuard` isn't `Send`
    // and Tauri requires the command future to be `Send`.
    let url_handle = {
        let mut mgr = state.tunnels.lock().expect("tunnels mutex poisoned");
        mgr.start(&app, &id, &upstream)?;
        mgr.url_handle(&id)?
    };

    let _url = crate::tunnel::wait_for_url(url_handle)
        .await
        .map_err(AppError::Tunnel)?;

    state
        .tunnels
        .lock()
        .expect("tunnels mutex poisoned")
        .status(&id)
        .ok_or_else(|| AppError::Internal("tunnel disappeared after start".into()))
}

#[tauri::command]
pub async fn stop_tunnel(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state
        .tunnels
        .lock()
        .expect("tunnels mutex poisoned")
        .stop(&id)?;
    Ok(())
}

#[tauri::command]
pub async fn list_tunnels(state: State<'_, AppState>) -> AppResult<Vec<TunnelStatus>> {
    Ok(state.tunnels.lock().expect("tunnels mutex poisoned").list())
}

#[tauri::command]
pub async fn tunnel_status(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<Option<TunnelStatus>> {
    Ok(state
        .tunnels
        .lock()
        .expect("tunnels mutex poisoned")
        .status(&id))
}
