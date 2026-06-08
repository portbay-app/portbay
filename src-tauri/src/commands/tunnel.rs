//! Cloudflare Tunnel commands.
//!
//! Three surfaces:
//!
//! - `start_tunnel(id)` — spawn cloudflared against the project's Caddy
//!   `:80` route (so Origin/Host normalisation applies), block until the
//!   public `trycloudflare.com` URL is announced, trigger a Caddy reconcile
//!   so the route flips to `normalize_all = true`, and return the full status.
//! - `stop_tunnel(id)` — kill the per-project cloudflared child, then
//!   trigger a Caddy reconcile to flip the route back to no-normalise.
//! - `list_tunnels()` — every active tunnel + its public URL.
//! - `tunnel_status(id)` — single-project lookup (used by polling
//!   modals while the URL is still being assigned).

use tauri::{AppHandle, State};

use crate::commands::projects::load_registry;
use crate::error::{AppError, AppResult};
use crate::registry::ProjectId;
use crate::state::AppState;
use crate::tunnel::{DetectedTunnel, TunnelStatus};

#[tauri::command]
pub async fn start_tunnel(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> AppResult<TunnelStatus> {
    // Route the tunnel back through Caddy so the per-project route's
    // Origin/Host normalisation applies; cloudflared sends the project hostname
    // as the `Host` header (`--http-host-header`) so Caddy matches the route.
    let registry = load_registry(&state)?;
    let project = registry
        .get_project(&ProjectId::new(&id))
        .ok_or_else(|| AppError::NotFound(id.clone()))?;

    // If the project has a custom named tunnel attached and the user is Pro,
    // Share routes through it (stable hostname) instead of a quick ephemeral
    // link. A non-Pro user with a leftover config falls back to quick share, so
    // sharing always works. The named tunnel goes straight to the dev origin
    // (preserving the real custom Host for OAuth/webhooks), so — unlike quick —
    // it doesn't flip the Caddy route's Origin normalisation.
    if let Some(cfg) = project.tunnel.as_ref().filter(|c| c.is_active()) {
        if crate::entitlements::is_pro() {
            let (config_path, upstream_url) =
                crate::tunnel::write_named_config(project, cfg).map_err(AppError::Tunnel)?;
            let public_url = format!("https://{}", cfg.hostname);
            let status = {
                let mut mgr = state.tunnels.lock().unwrap_or_else(|e| e.into_inner());
                mgr.start_custom(&app, &id, &config_path, &upstream_url, public_url)?
            };
            state.persist_tunnel_state();
            return Ok(status);
        }
    }

    let hostname = project.hostname.clone();

    // Always reach Caddy over plain HTTP on :80. While the tunnel is active the
    // project's :80 route serves (with Origin/Host normalisation) rather than
    // redirecting to https — even for https projects — so cloudflared never has
    // to do TLS to Caddy by IP (which can't carry SNI, so Caddy would have no
    // cert to present and the handshake would fail → 502).
    let upstream_url = "http://127.0.0.1:80".to_string();

    // Spawn + pull out the URL handle under one brief lock — we then
    // drop the lock before awaiting, because `MutexGuard` isn't `Send`
    // and Tauri requires the command future to be `Send`.
    let url_handle = {
        let mut mgr = state.tunnels.lock().unwrap_or_else(|e| e.into_inner());
        mgr.start(&app, &id, &hostname, &upstream_url)?;
        mgr.url_handle(&id)?
    };

    // Flip the project's :80 route to normalize_all = true (Origin/Host rewriting
    // on plain requests) NOW, while cloudflared is still negotiating the public
    // URL. The tunnel record is already in the manager, so the reconcile counts
    // it as active. Doing this before the URL wait closes the window where a
    // freshly-announced tunnel URL would hit the still-in-place http→https
    // redirect — a 308 to the unreachable `.test` host — for the first tick.
    state.reconciler.mark_dirty();

    if let Err(e) = crate::tunnel::wait_for_url(url_handle).await {
        // cloudflared never announced a URL: tear the tunnel down and revert the
        // route so a dead share doesn't leave the normalised :80 route in place.
        let _ = state
            .tunnels
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .stop(&id);
        state.reconciler.mark_dirty();
        state.persist_tunnel_state();
        return Err(AppError::Tunnel(e));
    }

    // Mirror the now-active tunnel (with its public URL) so the CLI / MCP
    // server can see it without reaching into our in-memory manager.
    state.persist_tunnel_state();

    state
        .tunnels
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .status(&id)
        .ok_or_else(|| AppError::Internal("tunnel disappeared after start".into()))
}

#[tauri::command]
pub async fn stop_tunnel(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state
        .tunnels
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .stop(&id)?;

    // Trigger a Caddy reconcile now that this project's tunnel is gone,
    // so the route flips back to normalize_all = false (CSRF intact,
    // plain requests untouched for local .test access).
    state.reconciler.mark_dirty();
    state.persist_tunnel_state();

    Ok(())
}

#[tauri::command]
pub async fn list_tunnels(state: State<'_, AppState>) -> AppResult<Vec<TunnelStatus>> {
    // `.list()` returns owned data; the guard drops at the end of this statement
    // so we never hold the mutex across the await below.
    let mut tunnels = state
        .tunnels
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .list();
    for t in &mut tunnels {
        if t.running {
            t.origin_reachable = Some(probe_origin(&t.upstream_url).await);
        }
    }
    // Keep the cross-process mirror fresh with the origin-probed view.
    state.mirror_tunnels(&tunnels);
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
        .unwrap_or_else(|e| e.into_inner())
        .status(&id);
    if let Some(s) = status.as_mut() {
        if s.running {
            s.origin_reachable = Some(probe_origin(&s.upstream_url).await);
        }
    }
    Ok(status)
}

/// List the user's named Cloudflare tunnels detected under `~/.cloudflared`
/// (one per credentials file), for the per-project "attach a custom tunnel"
/// picker. Read-only; empty when the user has no named tunnels. Not Pro-gated —
/// detection is harmless; *running* one is gated at attach + share time.
#[tauri::command]
pub async fn list_named_tunnels() -> AppResult<Vec<DetectedTunnel>> {
    Ok(crate::tunnel::detect_named_tunnels())
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
