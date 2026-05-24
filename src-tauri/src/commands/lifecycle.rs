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
pub async fn start_project(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> AppResult<()> {
    // Pure Caddy-served projects (Static sites, anything with no
    // `start_command`) have no Process Compose process. Calling
    // `client.start` for them hits PC's `/process/start/<name>` for a name
    // PC has never heard of → HTTP 400. They're served straight from disk by
    // Caddy's file_server, so "starting" them just means making sure the
    // route is live — handled by the reconcile tick below.
    if project_has_pc_process(&state, &id)? {
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
    }

    // After the process is up (or immediately, for static sites), force a
    // reconcile pass so the hosts file, dnsmasq, and Caddy all reflect the
    // project's hostname *before* we hand control back to the UI. Without
    // this, the user can press Play and click the URL before the background
    // reconciler has had a chance to add a route — landing them on a DNS
    // error instead of their freshly-started app. The tick is idempotent and
    // runs all sub-reconcilers; on success the project URL is immediately
    // resolvable.
    let _ = state.reconciler.tick(&app).await;

    Ok(())
}

/// True when the project has a Process Compose process backing it (i.e. it
/// pins a `start_command`). Static / pure-Caddy projects return `false`: they
/// have no daemon to start, stop, or restart.
fn project_has_pc_process(state: &State<'_, AppState>, project_id: &str) -> AppResult<bool> {
    let registry = load_registry(state)?;
    let project = registry
        .get_project(&ProjectId::new(project_id))
        .ok_or_else(|| AppError::NotFound(project_id.to_string()))?;
    Ok(project.start_command.is_some())
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

    // The directory a leaked dev server for THIS project would run in. For a
    // monorepo app pinned by a workspace filter, that's the app's sub-directory
    // (root/apps/web), not the repo root — so two apps sharing one root don't
    // both claim each other's orphans. Standalone projects use their own path.
    let working_dir = match &project.workspace {
        Some(ws) => ws.app_dir(&project.path).to_string_lossy().into_owned(),
        None => project.path.to_string_lossy().into_owned(),
    };
    // PID of our own process-compose, so we can tell PortBay's own running
    // dev server apart from a foreign holder of the same port.
    let pc_pid = state.pc.lock().unwrap_or_else(|e| e.into_inner()).pid();

    for port in ports {
        let Some(holder) = port_holder::find(port) else {
            continue;
        };

        // 1. A leaked PortBay dev server from a previous session (orphaned to
        //    launchd, cwd inside this project). The user clicked Start for this
        //    project — reclaim the port on their behalf. Kill the topmost
        //    matching ancestor so wrappers propagate the signal to their
        //    worker; for a bare orphan that's the holder itself.
        if holder.is_reclaimable_orphan(&working_dir) {
            let target = holder.kill_target(&working_dir);
            tracing::info!(
                project = %project_id,
                holder_pid = holder.pid,
                kill_pid = target,
                port = port,
                "reclaiming leaked PortBay-managed dev server before start",
            );
            let _ = port_holder::kill_gracefully(target, std::time::Duration::from_secs(2));
            if port_holder::find(port).is_none() {
                continue;
            }
            // Couldn't free it — surface as a conflict rather than looping.
            return Ok(Some((port, holder.display())));
        }

        // 2. PortBay's OWN running server (descends from our process-compose).
        //    Not a conflict — it's this project already up, or the fresh child
        //    a restart just spawned. Leave it alone.
        if pc_pid.is_some_and(|pp| holder.descends_from(pp)) {
            continue;
        }

        // 3. Anything else — a live process the user is running themselves
        //    (a terminal `npm run dev`, ServBay, …) or an unrelated process on
        //    this port. We never kill processes we don't own; surface a precise
        //    conflict so the user decides.
        return Ok(Some((port, holder.display())));
    }
    Ok(None)
}

#[tauri::command]
pub async fn stop_project(state: State<'_, AppState>, id: String) -> AppResult<()> {
    // Static / pure-Caddy projects have no PC process to stop. Caddy keeps
    // serving their files for as long as they're in the registry, so Stop is
    // a no-op rather than a 400 against a non-existent PC process.
    if !project_has_pc_process(&state, &id)? {
        return Ok(());
    }
    state.mark_stop_requested(&id);
    let client = state.pc_client()?;
    client.stop(&id).await?;
    Ok(())
}

#[tauri::command]
pub async fn restart_project(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> AppResult<()> {
    // Static / pure-Caddy projects have nothing to restart in PC; just
    // re-assert routing so the file_server route is fresh.
    if !project_has_pc_process(&state, &id)? {
        let _ = state.reconciler.tick(&app).await;
        return Ok(());
    }

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

    // Same rationale as start_project — guarantee routing is in sync
    // before the user retries the URL.
    let _ = state.reconciler.tick(&app).await;
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
