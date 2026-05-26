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
    // Choose the most robust origin for cloudflared:
    // - A project with a dev-server port → point straight at `127.0.0.1:<port>`.
    //   This drops the dependency on local DNS (/etc/hosts + dnsmasq) and Caddy
    //   being up, and avoids any Host-header/cert mismatch — the shared link
    //   keeps working for visitors as long as the dev server runs.
    // - A port-less project (PHP / static served by Caddy) must go through Caddy
    //   by hostname; cloudflared resolves it locally and `--no-tls-verify`
    //   accepts the mkcert cert.
    let upstream = match project.port {
        Some(port) => format!("http://127.0.0.1:{port}"),
        None => {
            let scheme = if project.https { "https" } else { "http" };
            format!("{scheme}://{}", project.hostname)
        }
    };

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
    // `.list()` returns owned data; the guard drops at the end of this statement
    // so we never hold the mutex across the await below.
    let mut tunnels = state.tunnels.lock().expect("tunnels mutex poisoned").list();
    for t in &mut tunnels {
        if t.running {
            t.origin_reachable = Some(probe_origin(&t.upstream_url).await);
        }
    }
    Ok(tunnels)
}

#[tauri::command]
pub async fn tunnel_status(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<Option<TunnelStatus>> {
    let mut status = state
        .tunnels
        .lock()
        .expect("tunnels mutex poisoned")
        .status(&id);
    if let Some(s) = status.as_mut() {
        if s.running {
            s.origin_reachable = Some(probe_origin(&s.upstream_url).await);
        }
    }
    Ok(status)
}

/// Quick liveness probe of the tunnel's local origin. Any HTTP response — even a
/// 4xx/5xx from the dev server — means the origin is up; a transport error means
/// it isn't (dev server stopped, DNS/Caddy down for a hostname upstream). Accepts
/// the mkcert self-signed cert for `https://*.test` origins and uses a short
/// timeout so polling stays snappy.
async fn probe_origin(upstream_url: &str) -> bool {
    let Ok(client) = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(std::time::Duration::from_millis(1500))
        .build()
    else {
        return false;
    };
    client.get(upstream_url).send().await.is_ok()
}
