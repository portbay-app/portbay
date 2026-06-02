//! Project lifecycle commands — start / stop / restart / stop_all / open.
//!
//! Every operation that touches Process Compose lives here. The `stop_all`
//! command is the reliability promise from `docs/UX_DESIGN.md` §5.1 — it
//! reports per-project outcomes so the frontend can surface partial
//! failures.

use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::commands::dto::{StopAllReport, StopAllResultEntry};
use crate::commands::events::emit_proc_log;
use crate::commands::projects::load_registry;
use crate::error::{AppError, AppResult};
use crate::port_holder;
use crate::registry::{Project, ProjectId, SandboxNetworkPolicy};
use crate::state::AppState;

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxStartOptions {
    #[serde(default)]
    pub network: SandboxNetworkPolicy,
    #[serde(default = "default_true")]
    pub ephemeral: bool,
}

fn default_true() -> bool {
    true
}

/// Result of a one-shot sandboxed dependency install. `output` is the tail of the
/// combined stdout+stderr (capped), and `violations` are the sandbox-denial lines
/// extracted from it so the UI can show what the profile blocked during install.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxInstallReport {
    /// The plain install command that was run (e.g. `pnpm install`), not the
    /// sandbox wrapper.
    pub command: String,
    pub ok: bool,
    pub exit_code: Option<i32>,
    pub output: String,
    /// Seatbelt file/exec denials extracted from the install output.
    pub violations: Vec<String>,
    /// Non-registry hosts the install tried (and was refused by the proxy) to
    /// reach — the network side of "what the sandbox blocked."
    pub blocked_hosts: Vec<String>,
}

/// Keep only the last `max_bytes` of `s`, prefixing an elision marker when
/// truncated, on a char boundary so the string stays valid UTF-8.
fn tail(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut start = s.len() - max_bytes;
    while start < s.len() && !s.is_char_boundary(start) {
        start += 1;
    }
    format!("… (truncated)\n{}", &s[start..])
}

/// Start every database instance linked to `project_id` so the app can connect
/// on boot. File-based engines (SQLite) have no daemon and are always
/// available, so they're skipped. Best-effort: a database that fails to start
/// is logged but doesn't block the project (the app surfaces the connection
/// error itself, which is more actionable than a blocked Play).
async fn start_linked_databases(app: &AppHandle, state: &State<'_, AppState>, project_id: &str) {
    let pid = ProjectId::new(project_id);
    let linked: Vec<String> = {
        let Ok(registry) = load_registry(state) else {
            return;
        };
        registry
            .list_databases()
            .iter()
            .filter(|inst| {
                !inst.engine.is_file_based() && inst.linked_projects.iter().any(|p| p == &pid)
            })
            .map(|inst| inst.process_id())
            .collect()
    };
    if linked.is_empty() {
        return;
    }
    // Reconcile so the `db-<id>` processes exist in PC's loaded YAML before we
    // start them (a stale reconcile would otherwise 404 the start).
    let _ = state.reconciler.tick(app).await;
    let Ok(client) = state.pc_client() else {
        return;
    };
    for process_id in linked {
        if let Err(e) = client.start(&process_id).await {
            tracing::warn!(
                target: "lifecycle",
                "linked database `{process_id}` for project `{project_id}` failed to start: {e}"
            );
        }
    }
}

#[tauri::command]
pub async fn start_project(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> AppResult<()> {
    // Immediate, terminal-style feedback in the log view: name the project and
    // echo the command the moment Play is pressed. Without this the panel sits
    // blank until the child emits its first line — which can be many seconds
    // for a dev server that compiles before printing — making PortBay feel
    // laggy next to a terminal that echoes `$ pnpm dev` instantly. Live-only
    // narration; emitted before any of the slower start work below.
    if let Ok(reg) = load_registry(&state) {
        if let Some(p) = reg.get_project(&ProjectId::new(&id)) {
            emit_proc_log(&app, &id, "system", format!("▶ Starting {}…", p.name));
            if let Some(cmd) = p.start_command.as_deref().filter(|c| !c.trim().is_empty()) {
                emit_proc_log(&app, &id, "system", format!("$ {cmd}"));
            }
        }
    }

    // Bring up any database the project is linked to first, so the app can
    // connect the moment its own process starts.
    start_linked_databases(&app, &state, &id).await;

    // Pure Caddy-served projects have no Process Compose process. Generated
    // Nginx/Apache PHP backends do, but their process id is derived.
    if let Some(process_id) = project_pc_process_id(&state, &id)? {
        // Reconcile first so generated web-server process definitions exist in
        // Process Compose before we ask it to start one.
        let _ = state.reconciler.tick(&app).await;
        // Port pre-flight. If the project pins a port and something else
        // is already bound to it, either clean up the orphan ourselves
        // (only when we can prove the holder is one of our stale dev
        // servers) or surface a precise PortConflict error so the user
        // knows exactly which process to stop. Skipping this step would
        // let Process Compose try, fail mysteriously, and surface a bare
        // "exited with code 1" — the cause the user reported.
        if let Some(holder) = preflight_port(&state, &id)? {
            // Surface *why* the start was refused in the log itself, so the log
            // view stays the single source of truth rather than only a toast.
            emit_proc_log(
                &app,
                &id,
                "error",
                format!(
                    "✗ Port {} is already in use by {} — start aborted.",
                    holder.0, holder.1
                ),
            );
            return Err(AppError::PortConflict {
                port: holder.0,
                holder: holder.1,
            });
        }

        let client = state.pc_client()?;
        client.start(&process_id).await?;
        // Remember this project in the running session so
        // `reopen_previous_projects` can restart it next launch.
        session_add(&state, &id);
    } else if project_is_static_served(&state, &id)? {
        emit_proc_log(
            &app,
            &id,
            "system",
            "Serving static files via Caddy — no process to run.",
        );
        // A static site has no process — Caddy serves its files directly. Mark
        // it "started" in the session; the reconcile tick below then publishes
        // its route (it's no longer suppressed) and the status poller reports
        // Running. This is the Play half of static play/pause.
        session_add(&state, &id);
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

/// `start_project_sandboxed(id)` — macOS safety run for untrusted projects.
///
/// The project stays in the normal Process Compose lifecycle, but its generated
/// command is wrapped by a PortBay-owned macOS Seatbelt profile. The hidden
/// sandbox config persists until `promote_project_to_local` removes it, so
/// restarts remain sandboxed and the UI can show an explicit indicator.
///
/// Availability: macOS only (Seatbelt). The anonymous/free community tiers can
/// sandbox up to [`SANDBOX_COMMUNITY_CAP`](crate::entitlements::SANDBOX_COMMUNITY_CAP)
/// projects; Pro is unlimited.
#[tauri::command]
pub async fn start_project_sandboxed(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    options: Option<SandboxStartOptions>,
) -> AppResult<()> {
    // Platform gate: Sandboxed Run is enforced by the macOS Seatbelt sandbox
    // (`sandbox-exec`). There's no equivalent confinement on other platforms
    // yet, so refuse rather than run untrusted code unconfined.
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (&app, &state, &id, &options);
        Err(AppError::Unsupported {
            feature: "Sandboxed Run",
            reason: "Sandboxed Run is only available on macOS.",
        })
    }

    #[cfg(target_os = "macos")]
    {
        {
            let mut registry = load_registry(&state)?;
            let pid = ProjectId::new(id.clone());

            // Tier gate: the anonymous/free community tiers may sandbox up to a
            // small cap (Pro is unlimited). Re-running a project that's already
            // sandboxed must never trip the limit, so only a *newly* sandboxed
            // project counts — measured against the other sandboxed projects.
            let already_on = registry
                .get_project(&pid)
                .map(crate::sandbox::is_enabled)
                .unwrap_or(false);
            if !already_on {
                let others = registry
                    .projects
                    .iter()
                    .filter(|p| p.id != pid && crate::sandbox::is_enabled(p))
                    .count();
                if let Err(cap) = crate::entitlements::check_can_sandbox(others) {
                    return Err(AppError::SandboxCapReached { cap });
                }
            }

            let project = registry
                .get_project_mut(&pid)
                .ok_or_else(|| AppError::NotFound(id.clone()))?;
            if project.start_command.is_none() && project.workspace.is_none() {
                return Err(AppError::BadInput(
                    "Sandboxed Run requires a project command to supervise".into(),
                ));
            }
            let options = options.unwrap_or(SandboxStartOptions {
                network: SandboxNetworkPolicy::LoopbackOnly,
                ephemeral: true,
            });
            crate::sandbox::enable(project, options.network, options.ephemeral);
            let data_dir = state.logs_dir.parent().unwrap_or(&state.logs_dir);
            // Fail closed: prove macOS accepts this exact profile before we
            // persist the sandboxed state or start the project. We never want a
            // path where the project runs but confinement silently didn't apply.
            crate::sandbox::preflight(data_dir, project)
                .map_err(|e| AppError::Internal(format!("sandbox could not be activated: {e}")))?;
            crate::sandbox::reset_ephemeral_state(data_dir, project)
                .map_err(|e| AppError::Internal(format!("sandbox reset failed: {e}")))?;
            crate::commands::projects::save_registry(&state, &registry)?;
        }
        let _ = state.reconciler.tick(&app).await;
        start_project(app, state, id).await
    }
}

/// Remove the sandbox wrapper from a project after the user has inspected it.
#[tauri::command]
pub async fn promote_project_to_local(state: State<'_, AppState>, id: String) -> AppResult<()> {
    let mut registry = load_registry(&state)?;
    let project = registry
        .get_project_mut(&ProjectId::new(id.clone()))
        .ok_or_else(|| AppError::NotFound(id.clone()))?;
    crate::sandbox::disable(project);
    crate::commands::projects::save_registry(&state, &registry)?;
    state.reconciler.mark_dirty();
    Ok(())
}

#[tauri::command]
pub async fn sandbox_violations(
    state: State<'_, AppState>,
    id: String,
    limit: Option<u32>,
) -> AppResult<Vec<String>> {
    let client = state.pc_client()?;
    let lines = client.logs(&id, 0, limit.unwrap_or(250)).await?;
    Ok(crate::sandbox::violation_lines(&lines))
}

/// `install_project_sandboxed(id)` — run the project's dependency install under
/// the sandbox with the network phase split: the install gets `Outbound` egress
/// (package managers need their registry) while the project's *run* policy is
/// untouched and stays as restrictive as the user set it. This is the right
/// place to contain a supply-chain attack — `postinstall` / Composer scripts run
/// at install time, confined here so they can fetch dependencies but can't read
/// your credentials, keychains, or other projects' files.
///
/// One-shot: it runs to completion and returns the output, rather than joining
/// the supervised Process Compose lifecycle. macOS only (Seatbelt), same as
/// [`start_project_sandboxed`].
///
/// Egress is pinned to package registries: the install runs `loopback_only` and
/// reaches the network only through [`crate::install_proxy`], an allowlisting
/// CONNECT proxy on `127.0.0.1`. A Seatbelt profile can't express domain pinning
/// (it filters by IP/port, not DNS name), so the proxy enforces it and reports
/// any non-registry host the install was refused as `blocked_hosts`.
#[tauri::command]
pub async fn install_project_sandboxed(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<SandboxInstallReport> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (&state, &id);
        Err(AppError::Unsupported {
            feature: "Sandboxed install",
            reason: "Sandboxed install is only available on macOS.",
        })
    }

    #[cfg(target_os = "macos")]
    {
        let registry = load_registry(&state)?;
        let project = registry
            .get_project(&ProjectId::new(id.clone()))
            .ok_or_else(|| AppError::NotFound(id.clone()))?
            .clone();

        let Some(install_cmd) = crate::sandbox::install_command(&project) else {
            return Err(AppError::BadInput(
                "No dependency manifest found to install — expected a package.json or composer.json."
                    .into(),
            ));
        };

        let data_dir = state
            .logs_dir
            .parent()
            .unwrap_or(&state.logs_dir)
            .to_path_buf();

        // Fail closed: prove macOS accepts the install profile before running.
        crate::sandbox::preflight_install(&data_dir, &project)
            .map_err(|e| AppError::Internal(format!("sandbox could not be activated: {e}")))?;
        // Give the (ephemeral) cache scratch a clean dir if ephemeral mode is on.
        crate::sandbox::reset_ephemeral_state(&data_dir, &project)
            .map_err(|e| AppError::Internal(format!("sandbox reset failed: {e}")))?;

        // Registry-pinning proxy: install runs loopback-only and reaches the
        // network only through this allowlisting CONNECT proxy on 127.0.0.1, so
        // a malicious postinstall can fetch dependencies but can't phone home to
        // an arbitrary host.
        let proxy = crate::install_proxy::RunningProxy::start()
            .await
            .map_err(|e| AppError::Internal(format!("could not start registry proxy: {e}")))?;

        // For node-type projects, put the managed Node bin dir on PATH during
        // install so PortBay's own `corepack` resolves, enable the pnpm/yarn
        // shims into the per-project corepack dir (our managed Node ships no
        // `pnpm` shim), and run the install through them so the pinned PM is
        // materialized into COREPACK_HOME for the offline run phase.
        let (node_env, effective_install_cmd) = {
            use crate::registry::ProjectType;
            let is_node = matches!(
                project.kind,
                ProjectType::Next | ProjectType::Vite | ProjectType::Node
            );
            if is_node {
                let rts = &registry.runtimes;
                let node_bin = project
                    .runtime
                    .as_ref()
                    .and_then(|rt| crate::runtimes::resolve_binary(rt, rts))
                    .or_else(|| crate::runtimes::resolve_default_node(rts));
                let bin_dir = node_bin.as_deref().and_then(|b| b.parent());
                let corepack_home = data_dir
                    .join("sandbox")
                    .join(project.id.as_str())
                    .join("corepack");
                let env = crate::sandbox::node_install_env_prefix(bin_dir, Some(&corepack_home));
                // Always materialize the pnpm/yarn shims into the per-project
                // corepack home and route the install through them, so the
                // pinned PM lands in COREPACK_HOME (writable in the sandbox) for
                // the offline run phase — whether on a managed Node (shims lead
                // a minimal PATH) or the system Node (shims enabled via the
                // login-shell corepack, COREPACK_HOME steered at the managed
                // dir). `corepack enable` only creates symlinks (no network).
                let cmd = format!(
                    "{}{}",
                    crate::sandbox::corepack_enable_preamble(&corepack_home),
                    install_cmd
                );
                (env, cmd)
            } else {
                (String::new(), install_cmd.clone())
            }
        };
        let wrapped = crate::sandbox::wrap_install_command(
            &data_dir,
            &project,
            &effective_install_cmd,
            &proxy.proxy_url(),
            &node_env,
        );
        let workdir = project.path.clone();

        // Installs can legitimately take minutes; cap at 15 to avoid a wedged
        // child blocking forever. The command runs through `/bin/sh -c` because
        // the wrapper is a shell line (env prefix + `sandbox-exec …`).
        let run = tokio::time::timeout(
            std::time::Duration::from_secs(900),
            tokio::process::Command::new("/bin/sh")
                .arg("-c")
                .arg(&wrapped)
                .current_dir(&workdir)
                .output(),
        )
        .await;

        // Always tear the proxy down (and harvest refused hosts), even on
        // timeout/spawn error, so it never outlives the install.
        let blocked_hosts = proxy.stop();

        let output = run
            .map_err(|_| AppError::Internal("sandboxed install timed out after 15 minutes".into()))?
            .map_err(|e| AppError::Internal(format!("could not run install: {e}")))?;

        let mut combined = String::new();
        combined.push_str(&String::from_utf8_lossy(&output.stdout));
        combined.push_str(&String::from_utf8_lossy(&output.stderr));
        let violations = crate::sandbox::violation_lines(
            &combined.lines().map(str::to_string).collect::<Vec<_>>(),
        );

        Ok(SandboxInstallReport {
            command: install_cmd,
            ok: output.status.success(),
            exit_code: output.status.code(),
            output: tail(&combined, 16_384),
            violations,
            blocked_hosts,
        })
    }
}

/// `force_start_project(id)` — the user's explicit "stop whatever's on the port
/// and start anyway" choice, invoked from the port-conflict confirmation. Unlike
/// `start_project`'s pre-flight (which only reclaims PortBay's *own* leaked
/// orphans and surfaces foreign holders untouched), this SIGTERM→SIGKILLs a
/// same-user foreign holder too. It still won't kill our own running server, and
/// it can't touch a root-owned holder (`kill` returns EPERM) — that case falls
/// through and re-surfaces as a normal `PortConflict`, so the user gets an
/// honest "couldn't free it" rather than a mysterious PC failure.
#[tauri::command]
pub async fn force_start_project(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> AppResult<()> {
    if let Some(process_id) = project_pc_process_id(&state, &id)? {
        let _ = state.reconciler.tick(&app).await;
        force_free_ports(&state, &id).await?;
        // Re-check: anything we couldn't kill (e.g. a root-owned process) still
        // blocks the bind — surface it rather than letting PC flail.
        if let Some((port, holder)) = preflight_port(&state, &id)? {
            return Err(AppError::PortConflict { port, holder });
        }
        let client = state.pc_client()?;
        client.start(&process_id).await?;
        // Remember this project in the running session so
        // `reopen_previous_projects` can restart it next launch.
        session_add(&state, &id);
    }
    let _ = state.reconciler.tick(&app).await;
    Ok(())
}

/// Forcibly free every port this project binds by killing whatever holds it —
/// except PortBay's own running server (no point killing what we're about to
/// reuse). A reclaimable orphan is killed at its topmost wrapper (so the worker
/// dies with it); a foreign holder is killed at its own pid. Root-owned holders
/// survive (`kill` EPERM) and are caught by the caller's re-check.
async fn force_free_ports(state: &State<'_, AppState>, id: &str) -> AppResult<()> {
    let registry = load_registry(state)?;
    let Some(project) = registry.get_project(&ProjectId::new(id)) else {
        return Ok(());
    };
    let (ports, working_dir) = project_ports_and_dir(project);
    let pc_pid = state.pc.lock().unwrap_or_else(|e| e.into_inner()).pid();

    let kills: Vec<u32> = ports
        .into_iter()
        .filter_map(|port| {
            let holder = port_holder::find(port)?;
            // Never kill our own running dev server.
            if pc_pid.is_some_and(|pp| holder.descends_from(pp)) {
                return None;
            }
            Some(if holder.is_reclaimable_orphan(&working_dir) {
                holder.kill_target(&working_dir)
            } else {
                holder.pid
            })
        })
        .collect();
    if kills.is_empty() {
        return Ok(());
    }

    tokio::task::spawn_blocking(move || {
        for pid in kills {
            tracing::info!(kill_pid = pid, "force-freeing port at user request");
            let _ = port_holder::kill_gracefully(pid, std::time::Duration::from_secs(2));
        }
    })
    .await
    .map_err(|e| AppError::Internal(format!("force-free task failed: {e}")))?;
    Ok(())
}

/// Process Compose id backing this project, when one exists. Normal dev-server
/// projects use the registry id; generated PHP web-server backends use their
/// derived `web-nginx-*` / `web-apache-*` process ids.
fn project_pc_process_id(
    state: &State<'_, AppState>,
    project_id: &str,
) -> AppResult<Option<String>> {
    let registry = load_registry(state)?;
    let project = registry
        .get_project(&ProjectId::new(project_id))
        .ok_or_else(|| AppError::NotFound(project_id.to_string()))?;
    Ok(pc_process_id_for_project(project))
}

fn pc_process_id_for_project(project: &Project) -> Option<String> {
    project.process_compose_id()
}

/// Whether `project_id` is a static site served directly by Caddy (no process),
/// whose started/paused state is the session set. Used by Start/Stop to toggle
/// serving. A missing project reads as `false`.
fn project_is_static_served(state: &State<'_, AppState>, project_id: &str) -> AppResult<bool> {
    let registry = load_registry(state)?;
    Ok(registry
        .get_project(&ProjectId::new(project_id))
        .map(|p| p.is_static_served())
        .unwrap_or(false))
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

    // Ports this start will try to bind + the directory a leaked dev server
    // for THIS project would run in. Shared with the post-stop reaper so both
    // paths classify holders identically.
    let (ports, working_dir) = project_ports_and_dir(project);
    if ports.is_empty() {
        return Ok(None);
    }
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

/// The ports a project binds (primary + extras) and the working directory a
/// leaked dev server for it would run in. For a monorepo app pinned by a
/// workspace filter that's the app's sub-directory (`root/apps/web`), not the
/// repo root — so sibling apps sharing one root never claim each other's
/// orphans. Standalone projects use their own path. Shared by the start
/// pre-flight and the post-stop reaper so both classify holders identically.
fn project_ports_and_dir(project: &Project) -> (Vec<u16>, String) {
    let mut ports: Vec<u16> = Vec::new();
    if let Some(p) = project.port {
        ports.push(p);
    }
    ports.extend(project.extra_ports.iter().copied());
    let working_dir = match &project.workspace {
        Some(ws) => ws.app_dir(&project.path).to_string_lossy().into_owned(),
        None => project.path.to_string_lossy().into_owned(),
    };
    (ports, working_dir)
}

/// Reap any leaked dev-server worker still holding this project's port(s) after
/// a stop. Process Compose SIGTERMs the command it spawned, but npm/pnpm/turbo
/// wrappers don't always forward that to the real worker (`next-server`), which
/// then orphans to launchd and keeps the port. We kill **only** a reclaimable
/// orphan — one whose cwd ties it to this project AND that has no live parent —
/// reusing the exact predicate the start pre-flight trusts, so a foreign
/// process the user runs themselves is never touched. Returns the count reaped.
///
/// The lsof/ps/kill work is blocking, so it runs on the blocking pool.
async fn reap_owned_orphans(state: &State<'_, AppState>, id: &str) -> u32 {
    let Ok(registry) = load_registry(state) else {
        return 0;
    };
    let Some(project) = registry.get_project(&ProjectId::new(id)) else {
        return 0;
    };
    let (ports, working_dir) = project_ports_and_dir(project);
    if ports.is_empty() {
        return 0;
    }
    let id = id.to_string();
    tokio::task::spawn_blocking(move || {
        let mut reaped = 0;
        for port in ports {
            let Some(holder) = port_holder::find(port) else {
                continue;
            };
            if holder.is_reclaimable_orphan(&working_dir) {
                let target = holder.kill_target(&working_dir);
                tracing::info!(
                    project = %id,
                    kill_pid = target,
                    port = port,
                    "reaping leaked dev server orphaned by stop",
                );
                let _ = port_holder::kill_gracefully(target, std::time::Duration::from_secs(2));
                reaped += 1;
            }
        }
        reaped
    })
    .await
    .unwrap_or(0)
}

/// Grace window between asking Process Compose to stop a process and checking
/// whether its worker leaked. Long enough for the wrapper to die and the worker
/// to reparent to launchd (so the orphan gate recognises it), short enough to
/// keep Stop feeling instant.
const STOP_REAP_DELAY: std::time::Duration = std::time::Duration::from_millis(750);

#[tauri::command]
pub async fn stop_project(app: AppHandle, state: State<'_, AppState>, id: String) -> AppResult<()> {
    if let Ok(reg) = load_registry(&state) {
        if let Some(p) = reg.get_project(&ProjectId::new(&id)) {
            emit_proc_log(&app, &id, "system", format!("■ Stopping {}…", p.name));
        }
    }
    // Stop any tunnel sharing this project first: a cloudflared tunnel pointed
    // at a now-stopped project would otherwise stay "running" in the UI while
    // every visitor gets an error. Best-effort — NotRunning is fine. Done before
    // the static-project early return so it covers Caddy-served projects too.
    {
        let mut tunnels = state.tunnels.lock().expect("tunnels mutex poisoned");
        if tunnels.stop(&id).is_ok() {
            // Tunnel was actually sharing this project — flip its :80 route back
            // out of normalize-all mode now, rather than waiting for the periodic
            // reconcile tick (matches the stop_tunnel command).
            state.reconciler.mark_dirty();
        }
    }
    state.persist_tunnel_state();
    // Drop it from the running session so it isn't reopened next launch.
    session_remove(&state, &id);

    // No PC process to stop. For a static site, the `session_remove` above just
    // flipped it to paused — nudge the reconciler so Caddy drops its route now
    // and it actually stops serving (the Stop half of static play/pause). Other
    // pure-Caddy projects (e.g. PHP-FPM) keep serving for as long as they're in
    // the registry, so Stop stays a no-op for them.
    let Some(process_id) = project_pc_process_id(&state, &id)? else {
        if project_is_static_served(&state, &id)? {
            state.reconciler.mark_dirty();
        }
        return Ok(());
    };
    state.mark_stop_requested(&id);
    let client = state.pc_client()?;
    client.stop(&process_id).await?;

    // PC's SIGTERM may leave the real dev-server worker orphaned and still on
    // the port; reap it so the next Start doesn't hit a self-inflicted
    // "port in use" conflict.
    tokio::time::sleep(STOP_REAP_DELAY).await;
    let _ = reap_owned_orphans(&state, &id).await;
    Ok(())
}

#[tauri::command]
pub async fn restart_project(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> AppResult<()> {
    if let Ok(reg) = load_registry(&state) {
        if let Some(p) = reg.get_project(&ProjectId::new(&id)) {
            emit_proc_log(&app, &id, "system", format!("↻ Restarting {}…", p.name));
        }
    }
    // Static / pure-Caddy projects have nothing to restart in PC; just
    // re-assert routing so the file_server route is fresh.
    let Some(process_id) = project_pc_process_id(&state, &id)? else {
        let _ = state.reconciler.tick(&app).await;
        return Ok(());
    };
    let _ = state.reconciler.tick(&app).await;

    // Restart kills the child too, so wrapper-translated SIGTERM exits
    // (npm → exit 1) shouldn't be flagged as crashes either.
    state.mark_stop_requested(&id);
    let client = state.pc_client()?;
    client.restart(&process_id).await?;

    // After PC fires the restart, wait briefly and confirm the port
    // got reclaimed by the new child. If a foreign process holds it,
    // surface the same PortConflict envelope start_project uses.
    tokio::time::sleep(std::time::Duration::from_millis(750)).await;
    if let Some((port, holder)) = preflight_port(&state, &id)? {
        emit_proc_log(
            &app,
            &id,
            "error",
            format!("✗ Port {port} is already in use by {holder} — restart aborted."),
        );
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
    // Kill every tunnel first, independent of PC. A share pointed at a project
    // we're about to stop would otherwise stay "running" while visitors get
    // errors — and doing it before `pc_client()?` means a dead daemon never
    // leaves a tunnel up. Covers static / Caddy-served shares too (no PC entry).
    {
        let stopped = state
            .tunnels
            .lock()
            .expect("tunnels mutex poisoned")
            .stop_all();
        if stopped > 0 {
            state.reconciler.mark_dirty();
        }
    }
    state.persist_tunnel_state();

    let client = state.pc_client()?;
    let processes = client.processes().await?;
    let registry = load_registry(&state)?;
    let process_to_project: std::collections::HashMap<String, String> = registry
        .list_projects()
        .iter()
        .filter_map(|project| {
            project
                .process_compose_id()
                .map(|process_id| (process_id, project.id.as_str().to_string()))
        })
        .collect();

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
        let process_id = p.name.clone();
        let report_id = process_to_project
            .get(&process_id)
            .cloned()
            .unwrap_or_else(|| process_id.clone());
        state.mark_stop_requested(&report_id);
        state.mark_stop_requested(&process_id);
        match client.stop(&process_id).await {
            Ok(()) => {
                report.stopped += 1;
                report.results.push(StopAllResultEntry {
                    id: report_id,
                    ok: true,
                    error: None,
                });
            }
            Err(e) => {
                report.failed += 1;
                report.results.push(StopAllResultEntry {
                    id: report_id,
                    ok: false,
                    error: Some(e.to_string()),
                });
            }
        }
    }

    // Reap any dev-server workers that orphaned instead of dying with their
    // wrapper, so Stop All leaves zero PortBay-owned servers squatting on
    // ports. One grace wait covers them all; each reap only touches that
    // project's reclaimable orphans (never a foreign process).
    if !report.results.is_empty() {
        tokio::time::sleep(STOP_REAP_DELAY).await;
        for entry in &report.results {
            let _ = reap_owned_orphans(&state, &entry.id).await;
        }
    }

    // Nothing is running now → clear the reopen-on-launch session.
    session_clear(&state);

    // Clearing the session just paused every static site (they serve only while
    // started); re-reconcile so Caddy drops their routes now instead of at the
    // next periodic tick. The status poller flips their badges to Stopped within
    // a tick off the same cleared session.
    if registry
        .list_projects()
        .iter()
        .any(|p| p.is_static_served())
    {
        state.reconciler.mark_dirty();
    }

    Ok(report)
}

// ---- Session persistence for `reopen_previous_projects` -------------------
//
// The set of project ids the user currently has running, kept in
// `<data-dir>/session.json`. Maintained incrementally on start/stop (so there's
// no async snapshot at quit time) and consumed once at boot by the reopen task.

fn session_file(state: &AppState) -> std::path::PathBuf {
    let data_dir = state.logs_dir.parent().unwrap_or(&state.logs_dir);
    data_dir.join("session.json")
}

pub(crate) fn load_session(state: &AppState) -> Vec<String> {
    std::fs::read(session_file(state))
        .ok()
        .and_then(|b| serde_json::from_slice::<Vec<String>>(&b).ok())
        .unwrap_or_default()
}

fn write_session(state: &AppState, ids: &[String]) {
    if let Ok(json) = serde_json::to_vec(ids) {
        let _ = std::fs::write(session_file(state), json);
    }
}

fn session_add(state: &AppState, id: &str) {
    let mut ids = load_session(state);
    if !ids.iter().any(|x| x == id) {
        ids.push(id.to_string());
        write_session(state, &ids);
    }
}

fn session_remove(state: &AppState, id: &str) {
    let mut ids = load_session(state);
    let before = ids.len();
    ids.retain(|x| x != id);
    if ids.len() != before {
        write_session(state, &ids);
    }
}

fn session_clear(state: &AppState) {
    write_session(state, &[]);
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

/// Reveal a filesystem path in the OS file manager (Finder on macOS), robust to
/// paths that don't exist yet.
///
/// The opener plugin's `reveal_item_in_dir` canonicalises the path first and
/// errors if it's missing — so "Reveal data folder" on a database whose data
/// dir hasn't been created yet (instance never started) would fail silently.
/// Instead we reveal the path when it exists, else open the nearest existing
/// ancestor directory, and only error when nothing along the path exists.
#[tauri::command]
pub async fn reveal_in_finder(app: AppHandle, path: String) -> AppResult<()> {
    let target = std::path::PathBuf::from(&path);
    if target.exists() {
        return app
            .opener()
            .reveal_item_in_dir(&target)
            .map_err(|e| AppError::Internal(format!("reveal failed: {e}")));
    }
    // Not created yet — open the closest ancestor that does exist so the button
    // still lands the user in the right place rather than doing nothing.
    let mut cursor = target.as_path();
    while let Some(parent) = cursor.parent() {
        if parent.exists() {
            return app
                .opener()
                .open_path(parent.to_string_lossy(), None::<&str>)
                .map_err(|e| AppError::Internal(format!("open failed: {e}")));
        }
        cursor = parent;
    }
    Err(AppError::NotFound(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{ProjectType, Workspace, WorkspaceTool};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn project_at(
        path: &str,
        port: Option<u16>,
        extra: Vec<u16>,
        ws: Option<Workspace>,
    ) -> Project {
        Project {
            cors: None,
            sandbox: None,
            id: ProjectId::new("p"),
            name: "P".into(),
            path: PathBuf::from(path),
            kind: ProjectType::Node,
            start_command: Some("pnpm dev".into()),
            port,
            extra_ports: extra,
            hostname: "p.test".into(),
            https: false,
            services: vec![],
            env: BTreeMap::new(),
            readiness: None,
            auto_start: false,
            pre_start: vec![],
            post_start: vec![],
            tags: vec![],
            document_root: None,
            php_version: None,
            web_server: None,
            mobile_run: None,
            runtime: None,
            workspace: ws,
            domain: None,
            tunnel: None,
            deploy: None,
        }
    }

    #[test]
    fn ports_and_dir_standalone_uses_project_path() {
        let p = project_at("/repos/site", Some(3000), vec![4000], None);
        let (ports, dir) = project_ports_and_dir(&p);
        assert_eq!(ports, vec![3000, 4000]);
        assert_eq!(dir, "/repos/site");
    }

    #[test]
    fn ports_and_dir_monorepo_uses_workspace_app_dir() {
        let ws = Workspace {
            package: "@acme/web".into(),
            rel_dir: "apps/web".into(),
            tool: WorkspaceTool::Pnpm,
        };
        let p = project_at("/repos/monorepo", Some(3000), vec![], Some(ws));
        let (ports, dir) = project_ports_and_dir(&p);
        assert_eq!(ports, vec![3000]);
        // The app sub-dir, so a sibling app sharing the root never claims this
        // one's orphan (and vice versa).
        assert_eq!(dir, "/repos/monorepo/apps/web");
    }

    #[test]
    fn ports_and_dir_no_port_yields_empty() {
        let p = project_at("/repos/static", None, vec![], None);
        let (ports, _) = project_ports_and_dir(&p);
        assert!(ports.is_empty());
    }
}
