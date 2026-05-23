//! Project lifecycle commands — start / stop / restart / stop_all / open.
//!
//! Every operation that touches Process Compose lives here. The `stop_all`
//! command is the reliability promise from `docs/UX_DESIGN.md` §5.1 — it
//! reports per-project outcomes so the frontend can surface partial
//! failures.

use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::commands::dto::{StopAllReport, StopAllResultEntry};
use crate::commands::projects::load_registry;
use crate::error::{AppError, AppResult};
use crate::port_holder;
use crate::registry::ProjectId;
use crate::state::AppState;

#[tauri::command]
pub async fn start_project(state: State<'_, AppState>, id: String) -> AppResult<()> {
    // Port pre-flight. If the project pins a port and something else
    // is already bound to it, either clean up the orphan ourselves
    // (only when we can prove the holder is one of our stale dev
    // servers) or surface a precise PortConflict error so the user
    // knows exactly which process to stop. Skipping this step would
    // let Process Compose try, fail mysteriously, and surface a bare
    // "exited with code 1" — the cause the user reported.
    if let Some(holder) = preflight_port(&state, &id)? {
        return Err(AppError::PortConflict {
            port: holder.0,
            holder: holder.1,
        });
    }

    let client = state.pc_client()?;
    client.start(&id).await?;
    Ok(())
}

/// Walk the registry for the given project, look up any port it pins
/// (primary `port` + `extra_ports`), and check whether anything is
/// already listening. PortBay-owned orphans get killed in place;
/// external holders bubble up as a structured conflict.
///
/// Returns:
///   - `Ok(None)` → no conflict, safe to proceed.
///   - `Ok(Some((port, holder_label)))` → a conflict the caller
///     should surface as `AppError::PortConflict`.
///   - `Err(...)` only for registry I/O failures.
fn preflight_port(
    state: &State<'_, AppState>,
    project_id: &str,
) -> AppResult<Option<(u16, String)>> {
    let registry = load_registry(state)?;
    let project = registry
        .get_project(&ProjectId::new(project_id))
        .ok_or_else(|| AppError::NotFound(project_id.to_string()))?;

    // Build the set of ports the start will try to bind. The primary
    // port + extras share the same enforcement.
    let mut ports: Vec<u16> = Vec::new();
    if let Some(p) = project.port {
        ports.push(p);
    }
    ports.extend(project.extra_ports.iter().copied());
    if ports.is_empty() {
        return Ok(None);
    }

    let working_dir = project.path.to_string_lossy().into_owned();

    for port in ports {
        let Some(holder) = port_holder::find(port) else {
            continue;
        };
        // If the holder (or any of its ancestors) is one of our own
        // stale dev servers, kill the topmost matching ancestor so
        // wrappers propagate the signal down. The user explicitly
        // clicked Start — we know they want this port for this
        // project. Worker processes (e.g. Next.js's `next-server`)
        // hide the path, but the dev-server shell that spawned them
        // carries it; the ancestor walk catches those.
        if holder.looks_like_portbay_orphan(&working_dir) {
            let target = holder.kill_target(&working_dir);
            tracing::info!(
                project = %project_id,
                holder_pid = holder.pid,
                kill_pid = target,
                port = port,
                "killing stale PortBay-managed dev server before restart",
            );
            let _ = port_holder::kill_gracefully(
                target,
                std::time::Duration::from_secs(2),
            );
            // Re-check; if the slot is now free, keep going.
            if port_holder::find(port).is_none() {
                continue;
            }
        }
        return Ok(Some((port, holder.display())));
    }
    Ok(None)
}

#[tauri::command]
pub async fn stop_project(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.mark_stop_requested(&id);
    let client = state.pc_client()?;
    client.stop(&id).await?;
    Ok(())
}

#[tauri::command]
pub async fn restart_project(state: State<'_, AppState>, id: String) -> AppResult<()> {
    // Restart kills the child too, so wrapper-translated SIGTERM exits
    // (npm → exit 1) shouldn't be flagged as crashes either.
    state.mark_stop_requested(&id);
    let client = state.pc_client()?;
    client.restart(&id).await?;

    // After PC fires the restart, wait briefly and confirm the port
    // got reclaimed by the new child. If a foreign process holds it,
    // surface the same PortConflict envelope start_project uses.
    tokio::time::sleep(std::time::Duration::from_millis(750)).await;
    if let Some((port, holder)) = preflight_port(&state, &id)? {
        return Err(AppError::PortConflict { port, holder });
    }
    Ok(())
}

/// `preview_port_conflict(port)` — synchronous lsof probe used by the
/// Add-project wizard so the user sees an inline warning while
/// typing the port. Returns the holder label, or None when the
/// port is free.
#[tauri::command]
pub async fn preview_port_conflict(port: u16) -> AppResult<Option<String>> {
    Ok(port_holder::find(port).map(|h| h.display()))
}

/// `stop_all()` — universal kill switch.
///
/// Pulls the current process list, stops them individually so we can
/// report per-project success/failure. Errors on individual processes are
/// captured in the report rather than aborting the whole call — the
/// frontend renders the table-of-outcomes to the user.
#[tauri::command]
pub async fn stop_all(state: State<'_, AppState>) -> AppResult<StopAllReport> {
    let client = state.pc_client()?;
    let processes = client.processes().await?;

    let mut report = StopAllReport {
        stopped: 0,
        failed: 0,
        results: Vec::with_capacity(processes.len()),
    };

    for p in processes {
        if !p.is_running {
            // Already stopped — don't waste an HTTP call. Don't report
            // either; the table only cares about projects we actually touched.
            continue;
        }
        let id = p.name.clone();
        state.mark_stop_requested(&id);
        match client.stop(&id).await {
            Ok(()) => {
                report.stopped += 1;
                report.results.push(StopAllResultEntry {
                    id,
                    ok: true,
                    error: None,
                });
            }
            Err(e) => {
                report.failed += 1;
                report.results.push(StopAllResultEntry {
                    id,
                    ok: false,
                    error: Some(e.to_string()),
                });
            }
        }
    }

    Ok(report)
}

/// `open_project(id)` — open the project's URL in the default browser.
///
/// Uses `tauri-plugin-shell`'s capability-scoped `open` rather than the
/// CLI's `std::process::Command::new("open")` — capabilities make this
/// auditable and portable to Linux/Windows when those targets land.
#[tauri::command]
pub async fn open_project(app: AppHandle, state: State<'_, AppState>, id: String) -> AppResult<()> {
    let registry = load_registry(&state)?;
    let project = registry
        .get_project(&ProjectId::new(id.clone()))
        .ok_or_else(|| AppError::NotFound(id))?;
    let scheme = if project.https { "https" } else { "http" };
    let url = format!("{scheme}://{}", project.hostname);
    app.opener()
        .open_url(&url, None::<&str>)
        .map_err(|e| AppError::Internal(format!("opener failed: {e}")))?;
    Ok(())
}
