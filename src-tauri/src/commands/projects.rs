//! Project CRUD commands.
//!
//! Thin wrappers around the registry CRUD already shipped in P1. The
//! frontend never touches `registry::Registry` directly — every read or
//! write goes through these commands so we can layer in side effects
//! (Caddy reconcile, hosts file write, cert issuance) in one place later.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::{AppHandle, State};
use tauri_plugin_shell::process::CommandEvent;
use tauri_plugin_shell::ShellExt;

use crate::commands::dto::{
    AddProjectInput, DetectedProject, LanguageIntelligenceCapability, ProjectView,
    UpdateProjectPatch, WorkspaceAppDto, WorkspaceScan,
};
use crate::error::{AppError, AppResult};
use crate::process_compose::{Process, ProjectStatus};
use crate::registry::{
    store, Framework, MobileRunConfig, Project, ProjectDeploy, ProjectId, ProjectType, Readiness,
    Registry, Runtime, SandboxConfig, SandboxNetworkPolicy, WebServer,
};
use crate::state::AppState;

/// `list_projects()` — registry merged with live PC status.
///
/// When the daemon is unreachable, every project is reported as `Stopped`
/// (no runtime info). This is the graceful-degradation pattern the CLI
/// already follows.
#[tauri::command]
pub async fn list_projects(state: State<'_, AppState>) -> AppResult<Vec<ProjectView>> {
    let registry = load_registry(&state)?;
    let pc_state = fetch_pc_state(&state).await;
    // "Started" static sites — their Running/Stopped is the session set, not a
    // process (see `Project::is_static_served`).
    let started = started_static_ids(&state);

    let views = registry
        .list_projects()
        .iter()
        .map(|p| {
            let proc = pc_state
                .as_ref()
                .and_then(|m| p.process_compose_id().and_then(|key| m.get(key.as_str())));
            let mut view = ProjectView::from_project(p, proc);
            if p.is_static_served() {
                view.status = static_status(p, &started);
            }
            view
        })
        .collect();
    Ok(views)
}

/// Project ids the session marks as currently "started", filtered to those
/// that are static-served. The session file is the source of truth for whether
/// a process-less static site is serving.
fn started_static_ids(state: &State<'_, AppState>) -> HashSet<String> {
    crate::commands::lifecycle::load_session(state.inner())
        .into_iter()
        .collect()
}

/// Running when the static site is in the started set, Stopped otherwise. This
/// is the single place the static play/pause state maps onto the UI taxonomy;
/// the reconciler keys the Caddy route off the same session set.
fn static_status(project: &Project, started: &HashSet<String>) -> ProjectStatus {
    if started.contains(project.id.as_str()) {
        ProjectStatus::Running
    } else {
        ProjectStatus::Stopped
    }
}

/// `get_project(id)` — single project with merged live state.
#[tauri::command]
pub async fn get_project(state: State<'_, AppState>, id: String) -> AppResult<ProjectView> {
    let registry = load_registry(&state)?;
    let project = registry
        .get_project(&ProjectId::new(id.clone()))
        .ok_or_else(|| AppError::NotFound(id.clone()))?;
    let pc_state = fetch_pc_state(&state).await;
    let proc = pc_state.as_ref().and_then(|m| {
        project
            .process_compose_id()
            .and_then(|key| m.get(key.as_str()))
    });
    let mut view = ProjectView::from_project(project, proc);
    if project.is_static_served() {
        view.status = static_status(project, &started_static_ids(&state));
    }
    Ok(view)
}

/// `project_icon(id)` — best-effort detected favicon / app-icon for a
/// project, as a `data:` URL, or `null` when none is found (the UI then shows
/// the project's stack glyph). The scan result is cached per session in
/// [`AppState`] so the avatar doesn't re-walk the tree on every render. The
/// detection itself lives in [`crate::project_icon`].
#[tauri::command]
pub async fn project_icon(state: State<'_, AppState>, id: String) -> AppResult<Option<String>> {
    if let Some(hit) = state.cached_icon(&id) {
        return Ok(hit);
    }
    let registry = load_registry(&state)?;
    let project = registry
        .get_project(&ProjectId::new(id.clone()))
        .ok_or_else(|| AppError::NotFound(id.clone()))?;
    let data_url = crate::project_icon::detect_icon(project).map(|icon| icon.to_data_url());
    state.cache_icon(&id, data_url.clone());
    Ok(data_url)
}

/// `add_project(input)` — register a new project from a folder path.
///
/// Mirrors the CLI's `add` flow (`bin/portbay.rs::cmd_add`). Best-effort
/// `/etc/hosts` write — permission-denied surfaces as a hint in the error
/// envelope rather than failing the whole call, because the registry write
/// has already succeeded by then.
#[tauri::command]
pub async fn add_project(
    state: State<'_, AppState>,
    input: AddProjectInput,
) -> AppResult<ProjectView> {
    let path = PathBuf::from(&input.path)
        .canonicalize()
        .map_err(|e| AppError::BadInput(format!("path: {e}")))?;

    let dir_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("project")
        .to_string();

    let id_str = input.id.unwrap_or_else(|| slugify(&dir_name));
    let id = ProjectId::new(id_str.clone());

    let name = input.name.unwrap_or(dir_name);

    let mut registry = load_registry(&state)?;

    // Project-cap gate (anonymous 3 / free 6 / pro unlimited). The GUI gates
    // this proactively before opening the wizard; this is the backstop that
    // also covers the CLI and any non-gated path.
    if let Err(cap) = crate::entitlements::check_can_add(registry.projects.len()) {
        return Err(AppError::ProjectCapReached { cap });
    }
    // Sandboxed Run is community-capped (anonymous/free get a small allowance;
    // Pro unlimited), not Pro-only. A brand-new sandboxed project consumes a
    // slot against the projects already sandboxed.
    if input.sandbox.as_ref().is_some_and(|cfg| cfg.enabled) {
        let others = registry
            .projects
            .iter()
            .filter(|p| crate::sandbox::is_enabled(p))
            .count();
        if let Err(cap) = crate::entitlements::check_can_sandbox(others) {
            return Err(AppError::SandboxCapReached { cap });
        }
    }

    let hostname = input
        .hostname
        .unwrap_or_else(|| format!("{}.{}", id_str, registry.domain_suffix));

    let readiness = input.port.map(|_| Readiness::Http {
        path: "/".into(),
        timeout_seconds: 75,
    });

    // Prefer the project's own version-manager files, then fall back to the
    // language default from the Languages panel. For PHP we mirror it into
    // `php_version` too, since the FPM reconciler still reads that field.
    let runtime = crate::project_runtime::detect(&path)
        .or_else(|| registry.runtimes.default_for(input.kind))
        .or_else(|| detected_runtime_for(input.kind));
    let php_version = if input.kind == ProjectType::Php {
        input
            .php_version
            .clone()
            .or_else(|| runtime.as_ref().map(|r| r.version.clone()))
    } else {
        None
    };
    let document_root = if input.kind == ProjectType::Php {
        input.document_root.filter(|s| !s.trim().is_empty())
    } else {
        None
    };
    let web_server = if input.kind == ProjectType::Php && input.start_command.is_none() {
        input.web_server
    } else {
        None
    };
    let has_start_command = input.start_command.is_some();
    // Sub-stack is descriptive (logo/label only), so derive it from the folder
    // even when the user overrode the launch kind in the wizard.
    let framework = detect_kind(&path).framework;
    let project = Project {
        id,
        name,
        path,
        kind: input.kind,
        framework,
        start_command: input.start_command,
        port: input.port,
        extra_ports: vec![],
        hostname: hostname.clone(),
        https: input.https,
        services: default_services(input.kind, input.https, has_start_command),
        env: Default::default(),
        readiness,
        pre_start: Vec::new(),
        post_start: Vec::new(),
        auto_start: input.auto_start,
        tags: vec![],
        document_root,
        php_version,
        web_server,
        mobile_run: input.mobile_run,
        runtime,
        workspace: input.workspace,
        cors: None,
        sandbox: input.sandbox,
        domain: None,
        tunnel: None,
        deploy: None,
    };

    if registry.hostname_conflict(&project.hostname, None) {
        return Err(crate::registry::RegistryError::DuplicateHostname(project.hostname).into());
    }
    if let Some(port) = project.port {
        if registry.port_conflict(port, None) {
            return Err(crate::registry::RegistryError::DuplicatePort(port).into());
        }
    }

    registry.add_project(project.clone())?;
    if let Some(runtime) = &project.runtime {
        if let Err(err) = crate::project_runtime::ensure_marker_files(&project.path, runtime) {
            tracing::warn!(
                project_id = %project.id,
                error = %err,
                "failed to write project runtime marker files"
            );
        }
    }
    save_registry(&state, &registry)?;

    // Pre-stage the project-scoped PortBay MCP registration for the default
    // board agent (Claude — `.mcp.json`, the broadest convention) so the
    // `portbay://projects/<id>/…` URLs resolve the first time someone opens
    // their agent here, not only after the first card dispatch. Best-effort:
    // the board config doesn't exist yet (Claude is the default), the user can
    // re-target another agent from the board's "set up MCP" banner, and the
    // dispatch path re-runs this anyway — so a failure must never block create.
    #[cfg(feature = "tasks")]
    {
        let _ = crate::context::automation::ensure_project_mcp(
            &project.path,
            &project.id.to_string(),
            crate::context::config::AgentKind::Claude,
            None,
        );
    }

    // Hand off side-effects (hosts, certs, Caddy routes, PC YAML) to
    // the reconciler. The tick runs in the background; the user's
    // toast returns immediately.
    state.reconciler.mark_dirty();

    Ok(ProjectView::from_project(&project, None))
}

/// Streamed progress events from [`provision_python_env`].
#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ProvisionEvent {
    /// One line of child-process output (stdout or stderr).
    Log { line: String },
    /// Provisioning finished successfully.
    Done,
}

/// Create a Python virtualenv (`.venv`) for a project and install its declared
/// dependencies, streaming command output to the frontend over `on_event`.
///
/// Idempotent: an existing `.venv` is reused, but dependencies are still
/// (re)installed so a changed manifest takes effect. Prefers `uv` when present
/// on PATH (much faster) and otherwise falls back to the stdlib `venv` module
/// plus the venv's own `pip`. The interpreter is the project's pinned Python
/// runtime when it resolves to a managed/detected binary, else the system
/// `python3`. Once the venv exists, `inject_runtime_path` puts `.venv/bin` at
/// the front of PATH so Play and any task run inside it.
#[tauri::command]
pub async fn provision_python_env(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    on_event: Channel<ProvisionEvent>,
) -> AppResult<()> {
    let registry = load_registry(&state)?;
    let project = registry
        .get_project(&ProjectId::new(id.clone()))
        .ok_or_else(|| AppError::NotFound(id.clone()))?;
    if project.kind != ProjectType::Python {
        return Err(AppError::BadInput(format!(
            "project {id} is not a Python project"
        )));
    }
    let dir = project.path.clone();
    if !dir.is_dir() {
        return Err(AppError::BadInput(format!(
            "project path is not a directory: {}",
            dir.display()
        )));
    }

    // Resolve a Python interpreter: the project's pinned runtime if it resolves
    // to a managed/detected binary, else the system `python3` on PATH.
    let python = project
        .runtime
        .as_ref()
        .filter(|rt| rt.lang == "python")
        .and_then(|rt| crate::runtimes::resolve_binary(rt, &registry.runtimes))
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "python3".into());

    let venv = dir.join(".venv");
    let venv_bin = venv.join("bin");
    let venv_pip = venv_bin.join("pip").to_string_lossy().into_owned();
    let venv_python = venv_bin.join("python").to_string_lossy().into_owned();
    let use_uv = command_available(&app, "uv").await;

    // 1) Create the venv (skip if one already exists).
    if venv_bin.is_dir() {
        let _ = on_event.send(ProvisionEvent::Log {
            line: ".venv already exists — reusing it".into(),
        });
    } else if use_uv {
        run_streamed(&app, &on_event, "uv", &["venv", ".venv"], &dir).await?;
    } else {
        run_streamed(&app, &on_event, &python, &["-m", "venv", ".venv"], &dir).await?;
    }

    // 2) Install dependencies from whichever manifest is present.
    if dir.join("requirements.txt").exists() {
        if use_uv {
            run_streamed(
                &app,
                &on_event,
                "uv",
                &[
                    "pip",
                    "install",
                    "-r",
                    "requirements.txt",
                    "--python",
                    &venv_python,
                ],
                &dir,
            )
            .await?;
        } else {
            run_streamed(
                &app,
                &on_event,
                &venv_pip,
                &["install", "-r", "requirements.txt"],
                &dir,
            )
            .await?;
        }
    } else if dir.join("pyproject.toml").exists() || dir.join("setup.py").exists() {
        if use_uv {
            run_streamed(
                &app,
                &on_event,
                "uv",
                &["pip", "install", "-e", ".", "--python", &venv_python],
                &dir,
            )
            .await?;
        } else {
            run_streamed(&app, &on_event, &venv_pip, &["install", "-e", "."], &dir).await?;
        }
    } else {
        let _ = on_event.send(ProvisionEvent::Log {
            line:
                "No requirements.txt or pyproject.toml found — venv created with no extra packages"
                    .into(),
        });
    }

    let _ = on_event.send(ProvisionEvent::Done);
    Ok(())
}

/// Probe whether `program` is runnable on PATH (via `program --version`).
/// Used to prefer `uv` for Python provisioning when it's installed.
async fn command_available(app: &AppHandle, program: &str) -> bool {
    let Ok((mut rx, _child)) = app.shell().command(program).arg("--version").spawn() else {
        return false;
    };
    let mut ok = false;
    while let Some(event) = rx.recv().await {
        if let CommandEvent::Terminated(payload) = event {
            ok = payload.code == Some(0);
        }
    }
    ok
}

/// Spawn `program args` in `cwd`, streaming each output line to the frontend.
/// Returns an error if the process can't spawn or exits non-zero.
async fn run_streamed(
    app: &AppHandle,
    on_event: &Channel<ProvisionEvent>,
    program: &str,
    args: &[&str],
    cwd: &Path,
) -> AppResult<()> {
    let _ = on_event.send(ProvisionEvent::Log {
        line: format!("$ {program} {}", args.join(" ")),
    });
    let (mut rx, _child) = app
        .shell()
        .command(program)
        .args(args.iter().copied())
        .current_dir(cwd)
        .spawn()
        .map_err(|e| AppError::Internal(format!("failed to spawn {program}: {e}")))?;

    let mut exit_code: Option<i32> = None;
    while let Some(event) = rx.recv().await {
        match event {
            CommandEvent::Stdout(bytes) | CommandEvent::Stderr(bytes) => {
                let line = String::from_utf8_lossy(&bytes).trim_end().to_string();
                if !line.is_empty() {
                    let _ = on_event.send(ProvisionEvent::Log { line });
                }
            }
            CommandEvent::Terminated(payload) => exit_code = payload.code,
            _ => {}
        }
    }

    match exit_code {
        Some(0) => Ok(()),
        other => Err(AppError::Internal(format!(
            "{program} exited with code {other:?}"
        ))),
    }
}

/// `update_project(id, patch)` — apply a partial update + persist.
#[tauri::command]
pub async fn update_project(
    state: State<'_, AppState>,
    id: String,
    patch: UpdateProjectPatch,
) -> AppResult<ProjectView> {
    let mut registry = load_registry(&state)?;
    let pid = ProjectId::new(id.clone());

    // Sandbox community cap: newly enabling Sandboxed Run on a project consumes
    // a slot (Pro unlimited); flipping an already-sandboxed project doesn't.
    // Computed before the mutable borrow below to satisfy the borrow checker.
    if patch.sandbox.as_ref().is_some_and(|cfg| cfg.enabled) {
        let was_on = registry
            .get_project(&pid)
            .is_some_and(crate::sandbox::is_enabled);
        if !was_on {
            let others = registry
                .projects
                .iter()
                .filter(|p| p.id != pid && crate::sandbox::is_enabled(p))
                .count();
            if let Err(cap) = crate::entitlements::check_can_sandbox(others) {
                return Err(AppError::SandboxCapReached { cap });
            }
        }
    }

    // Reject a hostname/port that another project already owns (excluding this
    // one) before mutating — keeps two projects from silently sharing a Caddy
    // route or a port.
    if let Some(h) = patch.hostname.as_deref() {
        if registry.hostname_conflict(h, Some(&pid)) {
            return Err(crate::registry::RegistryError::DuplicateHostname(h.to_string()).into());
        }
    }
    if let Some(port) = patch.port {
        if registry.port_conflict(port, Some(&pid)) {
            return Err(crate::registry::RegistryError::DuplicatePort(port).into());
        }
    }

    let project = registry
        .get_project_mut(&pid)
        .ok_or_else(|| AppError::NotFound(id.clone()))?;

    if let Some(name) = patch.name {
        project.name = name;
    }
    // Mutable kind: promote/demote a project — e.g. a board-only `custom`
    // project (created from the Tasks page with no server) grows into a real
    // `next`/`php` app. `services` is recomputed from the new kind below
    // (unless the patch sends its own), so the converted project is actually
    // runnable instead of keeping the empty service list a board started with.
    let kind_changed = patch.kind.is_some_and(|k| k != project.kind);
    if let Some(kind) = patch.kind {
        project.kind = kind;
    }
    if let Some(hostname) = patch.hostname {
        project.hostname = hostname;
    }
    if let Some(port) = patch.port {
        project.port = Some(port);
    }
    if let Some(extras) = patch.extra_ports {
        project.extra_ports = extras;
    }
    let start_command_patched = patch.start_command.is_some();
    if let Some(cmd) = patch.start_command {
        project.start_command = cmd.and_then(|value| {
            let trimmed = value.trim().to_string();
            (!trimmed.is_empty()).then_some(trimmed)
        });
    }
    if let Some(https) = patch.https {
        project.https = https;
    }
    if let Some(auto) = patch.auto_start {
        project.auto_start = auto;
    }
    if let Some(readiness) = patch.readiness {
        project.readiness = Some(readiness);
    }
    if let Some(pre) = patch.pre_start {
        // Replace the whole ordered list; drop blank rows the editor may send.
        project.pre_start = pre
            .into_iter()
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty())
            .collect();
    }
    if let Some(post) = patch.post_start {
        project.post_start = post
            .into_iter()
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty())
            .collect();
    }
    if let Some(tags) = patch.tags {
        project.tags = tags;
    }
    if let Some(services) = patch.services {
        project.services = services;
    } else if kind_changed {
        // No explicit service list, but the kind switched — derive the right
        // services for the new kind (e.g. add `caddy`/`php-fpm`) so a promoted
        // board can serve. `https`/`start_command` above are already patched.
        project.services =
            default_services(project.kind, project.https, project.start_command.is_some());
    }
    if let Some(env) = patch.env {
        project.env = env;
    }
    if let Some(root) = patch.document_root {
        project.document_root = Some(root);
    }
    if let Some(ver) = patch.php_version {
        project.php_version = Some(ver);
    }
    if let Some(server) = patch.web_server {
        project.web_server = Some(server);
    }
    if let Some(mobile_run) = patch.mobile_run {
        project.mobile_run = Some(mobile_run);
        // Mobile projects carry the generated launch script stamped into
        // `start_command` (set at add time). A new destination/scheme/flavor
        // must regenerate it, or the pick would never take effect. An explicit
        // start_command in the same patch wins — the user is overriding.
        if !start_command_patched && crate::mobile::is_mobile_kind(project.kind) {
            if let Some(cmd) = crate::mobile::launch_command(project) {
                project.start_command = Some(cmd);
            }
        }
    }
    if let Some(ws) = patch.workspace {
        project.workspace = Some(ws);
    }
    if let Some(sandbox) = patch.sandbox {
        // The community cap was already enforced above, before the mutable
        // borrow. Here we just apply the policy.
        project.sandbox = Some(sandbox);
        project
            .tags
            .retain(|tag| tag != crate::sandbox::SANDBOX_TAG);
    }
    if let Some(cors) = patch.cors {
        // Pro gate (honest split): the basic listen port stays free; only a
        // custom cross-origin policy is gated. Introducing or changing an
        // *active* policy requires the `custom_port_cors` entitlement. An
        // existing policy is preserved on downgrade — we only reject the act
        // of changing it, never strip a configured value. Clearing (empty
        // origins) is always allowed. The GUI locks this proactively; this is
        // the core-side safety net for the CLI and hand-edited registries.
        let changed = project.cors.as_ref() != Some(&cors);
        if changed
            && cors.is_active()
            && !crate::entitlements::current().entitlements.custom_port_cors
        {
            return Err(AppError::ProRequired {
                feature: "Custom CORS",
            });
        }
        project.cors = if cors.is_active() { Some(cors) } else { None };
    }
    if let Some(domain) = patch.domain {
        // The editor always sends the full config; store an all-default config
        // as `None` so projects that never touch domain settings keep a clean
        // registry entry and behave identically to before the field existed.
        project.domain = (domain != crate::registry::DomainConfig::default()).then_some(domain);
    }
    if let Some(tunnel) = patch.tunnel {
        // Pro gate, mirroring CORS: attaching or changing an *active* custom
        // tunnel requires Pro; an existing one survives downgrade (we only
        // reject the change, never strip it). Clearing (blank config) is free.
        let changed = project.tunnel.as_ref() != Some(&tunnel);
        if changed && tunnel.is_active() && !crate::entitlements::is_pro() {
            return Err(AppError::ProRequired {
                feature: "Custom tunnel",
            });
        }
        project.tunnel = tunnel.is_active().then_some(tunnel);
    }

    let snapshot = project.clone();
    save_registry(&state, &registry)?;
    state.reconciler.mark_dirty();

    // Look up live runtime after save.
    let pc_state = fetch_pc_state(&state).await;
    let proc = pc_state.as_ref().and_then(|m| {
        snapshot
            .process_compose_id()
            .and_then(|key| m.get(key.as_str()))
    });
    Ok(ProjectView::from_project(&snapshot, proc))
}

/// Outcome of a one-shot readiness probe fired from the editor's "Probe now"
/// button. `ok` is whether the check would mark the project ready; `detail` is
/// a short human-readable line (status code, refusal, timeout) for the UI.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadinessProbeResult {
    pub ok: bool,
    pub detail: String,
    pub elapsed_ms: u64,
}

/// `probe_readiness(kind, port, path)` — run the configured readiness check
/// once against the dev server's *local* port (127.0.0.1, pre-Caddy, the same
/// target the reconciler hands Process Compose) and report the result.
///
/// This deliberately doesn't load the project: the editor calls it with the
/// values currently in the form so the user can test a probe before saving.
#[tauri::command]
pub async fn probe_readiness(
    kind: String,
    port: Option<u16>,
    path: Option<String>,
) -> AppResult<ReadinessProbeResult> {
    use std::time::{Duration, Instant};
    const TIMEOUT: Duration = Duration::from_secs(5);

    let started = Instant::now();
    let (ok, detail) = match kind.as_str() {
        "http" => {
            let port = port
                .ok_or_else(|| AppError::BadInput("an HTTP readiness probe needs a port".into()))?;
            let raw = path.unwrap_or_else(|| "/".into());
            let raw = raw.trim();
            let path = if raw.is_empty() {
                "/".to_string()
            } else if raw.starts_with('/') {
                raw.to_string()
            } else {
                format!("/{raw}")
            };
            let url = format!("http://127.0.0.1:{port}{path}");
            let client = reqwest::Client::builder()
                .timeout(TIMEOUT)
                .build()
                .map_err(|e| AppError::Internal(e.to_string()))?;
            match client.get(&url).send().await {
                // PC's http_get probe treats a reachable endpoint that doesn't
                // 5xx/connection-fail as healthy; we mirror that with < 400.
                Ok(resp) => {
                    let code = resp.status().as_u16();
                    (code < 400, format!("{url} → HTTP {code}"))
                }
                Err(e) if e.is_timeout() => (false, format!("{url} → timed out after 5s")),
                Err(e) if e.is_connect() => (
                    false,
                    format!("{url} → connection refused (nothing listening?)"),
                ),
                Err(e) => (false, format!("{url} → {e}")),
            }
        }
        "tcp" => {
            let port = port
                .ok_or_else(|| AppError::BadInput("a TCP readiness probe needs a port".into()))?;
            // connect_timeout is blocking — keep it off the async worker.
            let ok = tauri::async_runtime::spawn_blocking(move || {
                let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
                std::net::TcpStream::connect_timeout(&addr, TIMEOUT).is_ok()
            })
            .await
            .unwrap_or(false);
            let detail = if ok {
                format!("127.0.0.1:{port} accepted a TCP connection")
            } else {
                format!("127.0.0.1:{port} refused the connection or timed out")
            };
            (ok, detail)
        }
        "process" => (
            false,
            "“Trust the process” has no probe to run — readiness is just \
             “the process is alive”."
                .to_string(),
        ),
        other => {
            return Err(AppError::BadInput(format!(
                "unknown readiness kind: {other}"
            )))
        }
    };

    Ok(ReadinessProbeResult {
        ok,
        detail,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

/// `detect_project(path)` — quick framework + suggested-defaults probe.
///
/// Heuristic, not exhaustive. The Add Project wizard's L1 → L2 flow
/// fills its standard fields from this; the user edits before commit.
/// Match order matters — Next.js / Vite tests come before generic Node
/// so they win when a `package.json` references both.
#[tauri::command]
pub async fn detect_project(
    state: State<'_, AppState>,
    path: String,
) -> AppResult<DetectedProject> {
    let p = canonical_project_folder(&path)?;

    let dir_name = p
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("project")
        .to_string();
    let id = slugify(&dir_name);

    let registry = load_registry(&state)?;
    let suggested_hostname = format!("{id}.{}", registry.domain_suffix);

    let detected = detect_kind(&p);

    Ok(DetectedProject {
        kind: detected.kind,
        suggested_id: id,
        suggested_name: dir_name,
        suggested_hostname,
        suggested_port: detected.port,
        suggested_start_command: detected.start_command,
        suggested_document_root: detected.document_root,
        suggested_php_version: detected.php_version,
        suggested_web_server: detected.web_server,
        suggested_mobile_run: detected.mobile_run,
        language_intelligence: detect_language_intelligence(&p, detected.kind),
    })
}

/// `detect_workspace_apps(path)` — if `path` is a JS monorepo root, list the
/// runnable apps inside it so the wizard can offer to run just one.
///
/// Returns `Ok(None)` for a plain folder (the wizard then uses the normal
/// single-folder `detect_project` flow). Each returned app is pre-filled with
/// standalone-project defaults: its sub-directory as `path`, framework-detected
/// kind/port, and a `<package-manager> dev` command that runs only that app
/// (no `turbo --parallel` fan-out).
#[tauri::command]
pub async fn detect_workspace_apps(
    state: State<'_, AppState>,
    path: String,
) -> AppResult<Option<WorkspaceScan>> {
    let root = canonical_project_folder(&path)?;
    let Some(layout) = crate::registry::workspace::detect(&root) else {
        return Ok(None);
    };

    let registry = load_registry(&state)?;
    let suffix = &registry.domain_suffix;

    let apps = layout
        .packages
        .iter()
        .map(|pkg| {
            // Name/id from the directory leaf (`apps/web` → `web`); the package
            // name keeps the scope prefix that doesn't belong in a hostname.
            let leaf = Path::new(&pkg.rel_dir)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(&pkg.rel_dir);
            let id = slugify(leaf);
            let detected = detect_kind(&pkg.abs_dir);
            // Honour the repo's package manager rather than detect_kind's
            // hardcoded `pnpm dev`, but only for an app that has a dev command.
            let start_command = detected
                .start_command
                .map(|_| standalone_dev_command(layout.tool));
            WorkspaceAppDto {
                package: pkg.name.clone(),
                rel_dir: pkg.rel_dir.clone(),
                path: pkg.abs_dir.display().to_string(),
                kind: detected.kind,
                suggested_hostname: format!("{id}.{suffix}"),
                suggested_id: id,
                suggested_name: leaf.to_string(),
                suggested_port: detected.port,
                suggested_start_command: start_command,
                language_intelligence: detect_language_intelligence(&pkg.abs_dir, detected.kind),
            }
        })
        .collect();

    Ok(Some(WorkspaceScan {
        tool: layout.tool,
        apps,
    }))
}

/// The dev command that runs a single package from its OWN directory, in the
/// repo's package manager. Used for the Tier-1 flow where the chosen app is a
/// standalone project rooted at its sub-directory (so no workspace filter is
/// needed). Turbo isn't a package manager, so it maps to pnpm here; the
/// detector never selects Turbo as the tool anyway.
fn standalone_dev_command(tool: crate::registry::WorkspaceTool) -> String {
    use crate::registry::WorkspaceTool::*;
    match tool {
        Pnpm | Turbo => "pnpm dev".into(),
        Npm => "npm run dev".into(),
        Yarn => "yarn dev".into(),
        // Bun runs scripts as `bun run <script>`; bare `bun dev` collides with
        // reserved subcommands, so always go through `run`.
        Bun => "bun run dev".into(),
    }
}

/// Run a locally-installed framework binary through the repo's package manager.
///
/// Used for frameworks whose conventional dev script is NOT `dev` (Angular's
/// `ng serve`, Gatsby's `gatsby develop`, Vue CLI's `vue-cli-service serve`),
/// so `<pm> dev` would fail with "missing script". Resolving the binary from
/// `node_modules/.bin` via the package manager's exec works regardless of the
/// project's script names.
fn standalone_exec_command(tool: crate::registry::WorkspaceTool, bin: &str) -> String {
    use crate::registry::WorkspaceTool::*;
    match tool {
        Pnpm | Turbo => format!("pnpm exec {bin}"),
        Npm => format!("npm exec -- {bin}"),
        Yarn => format!("yarn exec {bin}"),
        Bun => format!("bunx {bin}"),
    }
}

/// The UI library a generic Vite/Node project is built with, for the logo only.
/// First match wins; the specific libraries are checked before the catch-all
/// React (a Vue/Svelte/Solid app never lists `react`). `None` keeps the plain
/// bundler/Node glyph. Only applied to the generic kinds — a specific kind like
/// Next/Astro already carries the correct logo.
fn js_lib_framework(body: &str) -> Option<Framework> {
    if body.contains("\"vue\"") {
        return Some(Framework::Vue);
    }
    if body.contains("\"svelte\"") {
        return Some(Framework::Svelte);
    }
    if body.contains("\"solid-js\"") {
        return Some(Framework::SolidJs);
    }
    if body.contains("\"preact\"") {
        return Some(Framework::Preact);
    }
    if body.contains("\"lit\"") {
        return Some(Framework::Lit);
    }
    if body.contains("\"alpinejs\"") {
        return Some(Framework::Alpine);
    }
    if body.contains("\"ember-source\"") || body.contains("\"ember-cli\"") {
        return Some(Framework::Ember);
    }
    if body.contains("\"react\"") {
        return Some(Framework::React);
    }
    None
}

/// Build a JS detection, attaching the UI-library framework when one was found.
fn js_detection(
    kind: ProjectType,
    framework: Option<Framework>,
    port: u16,
    cmd: String,
) -> ProjectDetection {
    match framework {
        Some(fw) => detection_fw(kind, fw, port, Some(cmd)),
        None => detection(kind, port, Some(cmd)),
    }
}

/// `validate_project_folder(path)` — canonicalise a dropped path and reject files.
#[tauri::command]
pub async fn validate_project_folder(path: String) -> AppResult<String> {
    Ok(canonical_project_folder(&path)?.display().to_string())
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloneSandboxedInput {
    pub url: String,
    pub parent_dir: Option<String>,
    #[serde(default)]
    pub network: SandboxNetworkPolicy,
    #[serde(default = "default_true")]
    pub ephemeral: bool,
    #[serde(default = "default_true")]
    pub start_after_import: bool,
}

fn default_true() -> bool {
    true
}

/// Clone a Git project from an external URL, register it with sandbox enabled,
/// and optionally start it immediately in that sandbox. This is the one-click
/// untrusted-source flow: normal local runs remain available, but imported
/// code starts inside the restricted profile until promoted.
#[tauri::command]
pub async fn clone_git_project_sandboxed(
    app: AppHandle,
    state: State<'_, AppState>,
    input: CloneSandboxedInput,
) -> AppResult<ProjectView> {
    // Sandbox community cap (Pro unlimited), checked up-front so we don't run a
    // network clone only to reject it. The clone always creates a *new*
    // sandboxed project, so it consumes a slot.
    {
        let registry = load_registry(&state)?;
        let others = registry
            .projects
            .iter()
            .filter(|p| crate::sandbox::is_enabled(p))
            .count();
        if let Err(cap) = crate::entitlements::check_can_sandbox(others) {
            return Err(AppError::SandboxCapReached { cap });
        }
    }
    let url = validate_git_url(&input.url)?;
    let parent = input
        .parent_dir
        .as_deref()
        .map(canonical_or_create_dir)
        .transpose()?
        .unwrap_or_else(|| {
            state
                .logs_dir
                .parent()
                .unwrap_or(&state.logs_dir)
                .join("sandbox")
                .join("imports")
        });
    std::fs::create_dir_all(&parent)
        .map_err(|e| AppError::Internal(format!("couldn't create import dir: {e}")))?;
    let dest = unique_clone_dir(&parent, repo_slug(&url));
    let clone_url = url.clone();
    let clone_dest = dest.clone();
    let clone_result: AppResult<()> = tokio::task::spawn_blocking(move || {
        std::process::Command::new("git")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_ASKPASS", "/bin/false")
            .args(["-c", "protocol.file.allow=never"])
            .args(["clone", "--depth", "1", "--filter=blob:none", "--"])
            .arg(&clone_url)
            .arg(&clone_dest)
            .status()
    })
    .await
    .map_err(|e| AppError::Internal(format!("git clone task failed: {e}")))?
    .map_err(|e| AppError::Internal(format!("couldn't start git clone: {e}")))
    .and_then(|status| {
        if status.success() {
            Ok(())
        } else {
            Err(AppError::Internal(format!(
                "git clone exited with status {status}"
            )))
        }
    });
    if let Err(err) = clone_result {
        let _ = std::fs::remove_dir_all(&dest);
        return Err(err);
    }

    let det = detect_kind(&dest);
    let dir_name = dest
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("project")
        .to_string();
    let id_str = slugify(&dir_name);
    let mut registry = load_registry(&state)?;
    if let Err(cap) = crate::entitlements::check_can_add(registry.projects.len()) {
        return Err(AppError::ProjectCapReached { cap });
    }
    let runtime = crate::project_runtime::detect(&dest)
        .or_else(|| registry.runtimes.default_for(det.kind))
        .or_else(|| detected_runtime_for(det.kind));
    let php_version = if det.kind == ProjectType::Php {
        det.php_version
            .clone()
            .or_else(|| runtime.as_ref().map(|r| r.version.clone()))
    } else {
        None
    };
    let has_start_command = det.start_command.is_some();
    let project = Project {
        id: ProjectId::new(id_str.clone()),
        name: dir_name,
        path: dest,
        kind: det.kind,
        framework: det.framework,
        start_command: det.start_command,
        port: det.port,
        extra_ports: vec![],
        hostname: format!("{id_str}.{}", registry.domain_suffix),
        https: true,
        services: default_services(det.kind, true, has_start_command),
        env: Default::default(),
        readiness: det.port.map(|_| Readiness::Http {
            path: "/".into(),
            timeout_seconds: 75,
        }),
        pre_start: Vec::new(),
        post_start: Vec::new(),
        auto_start: false,
        tags: vec![],
        document_root: det.document_root,
        php_version,
        web_server: det.web_server,
        mobile_run: det.mobile_run,
        runtime,
        workspace: None,
        cors: None,
        sandbox: Some(SandboxConfig::enabled(input.network, input.ephemeral)),
        domain: None,
        tunnel: None,
        deploy: None,
    };
    if registry.hostname_conflict(&project.hostname, None) {
        return Err(crate::registry::RegistryError::DuplicateHostname(project.hostname).into());
    }
    if let Some(port) = project.port {
        if registry.port_conflict(port, None) {
            return Err(crate::registry::RegistryError::DuplicatePort(port).into());
        }
    }
    registry.add_project(project.clone())?;
    save_registry(&state, &registry)?;
    state.reconciler.mark_dirty();
    if input.start_after_import && has_start_command {
        let _ = state.reconciler.tick(&app).await;
        state.pc_client()?.start(project.id.as_str()).await?;
    }
    Ok(ProjectView::from_project(&project, None))
}

fn validate_git_url(raw: &str) -> AppResult<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadInput("Git URL is required".into()));
    }
    if trimmed.chars().any(|c| c.is_control()) {
        return Err(AppError::BadInput(
            "Git URL cannot contain control characters".into(),
        ));
    }
    if trimmed.starts_with("git@") {
        if !trimmed.contains(':') || trimmed.ends_with(':') {
            return Err(AppError::BadInput(
                "SSH Git URLs must look like git@host:owner/repo.git".into(),
            ));
        }
        return Ok(trimmed.to_string());
    }
    let parsed = url::Url::parse(trimmed)
        .map_err(|_| AppError::BadInput("Enter an https:// or git@ Git URL".into()))?;
    match parsed.scheme() {
        "https" | "ssh" => Ok(trimmed.to_string()),
        other => Err(AppError::BadInput(format!(
            "unsupported Git URL scheme `{other}`"
        ))),
    }
}

fn canonical_or_create_dir(path: &str) -> AppResult<PathBuf> {
    let path = path.trim();
    if path.is_empty() {
        return Err(AppError::BadInput("parentDir is required".into()));
    }
    let p = PathBuf::from(path);
    std::fs::create_dir_all(&p)
        .map_err(|e| AppError::Internal(format!("couldn't create parent dir: {e}")))?;
    let canonical = p
        .canonicalize()
        .map_err(|e| AppError::BadInput(format!("parentDir: {e}")))?;
    validate_sandbox_import_parent(&canonical)?;
    assert_writable_directory(&canonical)?;
    Ok(canonical)
}

fn validate_sandbox_import_parent(path: &Path) -> AppResult<()> {
    let blocked = [
        Path::new("/"),
        Path::new("/Applications"),
        Path::new("/bin"),
        Path::new("/dev"),
        Path::new("/etc"),
        Path::new("/Library"),
        Path::new("/private"),
        Path::new("/private/etc"),
        Path::new("/private/var"),
        Path::new("/sbin"),
        Path::new("/System"),
        Path::new("/usr"),
        Path::new("/var"),
        Path::new("/Volumes"),
    ];
    if blocked.contains(&path) {
        return Err(AppError::BadInput(format!(
            "choose a normal writable project folder, not system root `{}`",
            path.display()
        )));
    }
    Ok(())
}

fn assert_writable_directory(path: &Path) -> AppResult<()> {
    if !path.is_dir() {
        return Err(AppError::BadInput(format!(
            "parentDir is not a folder: {}",
            path.display()
        )));
    }
    let marker = path.join(format!(
        ".portbay-write-test-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&marker)
    {
        Ok(_) => {
            let _ = std::fs::remove_file(marker);
            Ok(())
        }
        Err(e) => Err(AppError::BadInput(format!(
            "parentDir is not writable: {e}"
        ))),
    }
}

fn repo_slug(url: &str) -> String {
    let leaf = match url::Url::parse(url) {
        Ok(parsed) => parsed
            .path_segments()
            .and_then(|mut segments| segments.rfind(|segment| !segment.is_empty()))
            .unwrap_or("project")
            .to_string(),
        Err(_) => url
            .trim_end_matches('/')
            .rsplit(['/', ':'])
            .next()
            .unwrap_or("project")
            .to_string(),
    };
    let leaf = leaf.trim_end_matches(".git");
    let slug = slugify(leaf);
    if slug.is_empty() {
        "project".into()
    } else {
        slug
    }
}

fn unique_clone_dir(parent: &Path, base: String) -> PathBuf {
    let mut candidate = parent.join(&base);
    let mut n = 2;
    while candidate.exists() {
        candidate = parent.join(format!("{base}-{n}"));
        n += 1;
    }
    candidate
}

fn canonical_project_folder(path: &str) -> AppResult<PathBuf> {
    let p = PathBuf::from(path)
        .canonicalize()
        .map_err(|e| AppError::BadInput(format!("path: {e}")))?;
    let meta = p
        .metadata()
        .map_err(|e| AppError::BadInput(format!("path: {e}")))?;
    if !meta.is_dir() {
        return Err(AppError::BadInput(
            "Please drop a folder, not a file.".into(),
        ));
    }
    Ok(p)
}

pub struct ProjectDetection {
    pub kind: ProjectType,
    pub framework: Option<Framework>,
    pub port: Option<u16>,
    pub start_command: Option<String>,
    pub document_root: Option<String>,
    pub php_version: Option<String>,
    pub web_server: Option<WebServer>,
    pub mobile_run: Option<MobileRunConfig>,
}

pub fn detect_kind(path: &Path) -> ProjectDetection {
    let pkg = path.join("package.json");
    if pkg.exists() {
        let body = std::fs::read_to_string(&pkg).unwrap_or_default();
        // The run command follows the project's actual package manager
        // (`packageManager` field → lockfile → pnpm default) so the first Play
        // matches the lockfile instead of always guessing `pnpm dev`.
        let tool = crate::registry::workspace::detect_package_manager(path);
        let cmd = standalone_dev_command(tool);
        // Cheap string match on the raw text — full JSON parse isn't worth the
        // cycles. Each marker is quote-wrapped so it matches the dependency KEY
        // and not a longer package name (`"next"` won't hit `"next-themes"`).
        //
        // Ordering matters two ways:
        //   1. Expo first — an Expo app also lists react/react-native, so it
        //      must win over the generic Node fallback. Play runs `npx expo
        //      start` (Metro); no port/start_command stored — the reconciler
        //      generates the launch from the kind.
        //   2. The JS meta-frameworks below are matched BEFORE the generic
        //      `"vite"` branch, because SvelteKit/SolidStart/Qwik/Remix all
        //      carry Vite as a dependency and would otherwise be mis-detected
        //      as a plain Vite app. The default port is the framework's own dev
        //      default (PortBay proxies that port; it isn't injected into the
        //      command), so it must match what the dev server actually binds.
        if body.contains("\"expo\"") {
            let mobile_run = detect_expo_run(path);
            return detection_with_mobile(ProjectType::Expo, None, mobile_run);
        }
        if body.contains("\"next\"") {
            return detection(ProjectType::Next, 3000, Some(cmd));
        }
        if body.contains("\"astro\"") {
            return detection(ProjectType::Astro, 4321, Some(cmd));
        }
        if body.contains("\"@sveltejs/kit\"") {
            return detection(ProjectType::SvelteKit, 5173, Some(cmd));
        }
        if body.contains("\"nuxt\"") {
            return detection(ProjectType::Nuxt, 3000, Some(cmd));
        }
        if body.contains("@remix-run/") {
            return detection(ProjectType::Remix, 3000, Some(cmd));
        }
        if body.contains("\"gatsby\"") {
            // Gatsby scaffolds `develop`/`start`, never `dev`, so run the binary
            // directly. `gatsby develop` defaults to :8000.
            return detection(
                ProjectType::Gatsby,
                8000,
                Some(standalone_exec_command(tool, "gatsby develop")),
            );
        }
        if body.contains("\"@angular/core\"") {
            // Angular scaffolds `start: ng serve`, not `dev`. `ng serve`
            // defaults to :4200.
            return detection(
                ProjectType::Angular,
                4200,
                Some(standalone_exec_command(tool, "ng serve")),
            );
        }
        if body.contains("\"@solidjs/start\"") {
            return detection(ProjectType::SolidStart, 3000, Some(cmd));
        }
        if body.contains("\"@builder.io/qwik\"") || body.contains("@qwik.dev/") {
            return detection(ProjectType::Qwik, 5173, Some(cmd));
        }
        if body.contains("\"@vue/cli-service\"") {
            // Vue CLI scaffolds `serve: vue-cli-service serve`, not `dev`. A
            // Vite-based Vue app carries `"vite"` instead and falls through to
            // the Vite branch. `vue-cli-service serve` defaults to :8080.
            return detection(
                ProjectType::VueCli,
                8080,
                Some(standalone_exec_command(tool, "vue-cli-service serve")),
            );
        }
        if body.contains("\"preact-cli\"") {
            // preact-cli (webpack) scaffolds `dev: preact watch` on :8080. A
            // Vite-based Preact app carries `"vite"` and falls through instead.
            return detection(ProjectType::Preact, 8080, Some(cmd));
        }
        // Smaller JS meta-frameworks — detected before the generic Vite/Node
        // fallbacks so they keep their own logo + dev port. React Router 7 is
        // Vite-based; the others run their own dev server under Node. Match the
        // framework's build tooling (`@react-router/dev`), not the routing
        // library (`react-router`), which countless React SPAs also depend on.
        if body.contains("\"@react-router/dev\"") {
            return detection_fw(ProjectType::Vite, Framework::ReactRouter, 5173, Some(cmd));
        }
        if body.contains("\"@redwoodjs/") {
            return detection_fw(
                ProjectType::Node,
                Framework::Redwood,
                8910,
                Some(standalone_exec_command(tool, "redwood dev")),
            );
        }
        if body.contains("\"@docusaurus/core\"") {
            return detection_fw(
                ProjectType::Node,
                Framework::Docusaurus,
                3000,
                Some(standalone_exec_command(tool, "docusaurus start")),
            );
        }
        if body.contains("\"@11ty/eleventy\"") {
            return detection_fw(
                ProjectType::Node,
                Framework::Eleventy,
                8080,
                Some(standalone_exec_command(tool, "@11ty/eleventy --serve")),
            );
        }
        // Generic Vite / Node: attach the UI-library logo (React/Vue/Svelte/…)
        // so the project reads as its framework rather than the bare bundler.
        let lib = js_lib_framework(&body);
        if body.contains("\"vite\"") {
            return js_detection(ProjectType::Vite, lib, 5173, cmd);
        }
        return js_detection(ProjectType::Node, lib, 3000, cmd);
    }

    if path.join("composer.json").exists() || has_php_index(path) {
        return detect_php_project(path);
    }

    if is_python_project(path) {
        return detect_python_project(path);
    }

    if path.join("Gemfile").exists() {
        return detect_ruby_project(path);
    }

    // Hugo is checked before `go.mod`: a Hugo site may use Go modules, but its
    // signature config + scaffold dirs are unambiguous, and it launches its own
    // dev server rather than `go run`.
    if is_hugo_project(path) {
        return detection_fw(
            ProjectType::Go,
            Framework::Hugo,
            1313,
            Some("hugo server -D --port 1313".into()),
        );
    }
    if path.join("go.mod").exists() {
        return detect_go_project(path);
    }
    if path.join("Cargo.toml").exists() {
        return detect_rust_project(path);
    }
    if path.join("deno.json").exists() || path.join("deno.jsonc").exists() {
        return detect_deno_project(path);
    }
    if path.join("mix.exs").exists() {
        return detect_elixir_project(path);
    }
    if has_dotnet_project(path) {
        return detect_dotnet_project(path);
    }

    // Other language runtimes, keyed on each toolchain's signature file. They
    // run as long-lived processes (`port: 0` → no proxy) until the user sets a
    // port, mirroring the bare-Python/Go behaviour. Distinct markers, so order
    // among them doesn't matter — but they sit before the mobile + JVM
    // detectors below.
    if path.join("build.sbt").exists() {
        return detection(ProjectType::Scala, 0, Some("sbt run".into()));
    }
    if path.join("deps.edn").exists() || path.join("project.clj").exists() {
        let cmd = if path.join("project.clj").exists() {
            "lein run"
        } else {
            "clojure -M:run"
        };
        return detection(ProjectType::Clojure, 0, Some(cmd.into()));
    }
    if path.join("shard.yml").exists() {
        return detection(ProjectType::Crystal, 0, Some("shards run".into()));
    }
    if path.join("build.zig").exists() {
        return detection(ProjectType::Zig, 0, Some("zig build run".into()));
    }
    if has_child_with_extension(path, "nimble") {
        return detection(ProjectType::Nim, 0, Some("nimble run".into()));
    }
    if path.join("stack.yaml").exists() || has_child_with_extension(path, "cabal") {
        let cmd = if path.join("stack.yaml").exists() {
            "stack run"
        } else {
            "cabal run"
        };
        return detection(ProjectType::Haskell, 0, Some(cmd.into()));
    }
    if path.join("dune-project").exists() {
        return detection(ProjectType::OCaml, 0, Some("dune exec".into()));
    }

    if is_flutter_project(path) {
        let mobile_run = detect_flutter_run(path);
        return detection_with_mobile(
            ProjectType::Flutter,
            mobile_start_command(ProjectType::Flutter, mobile_run.as_ref()),
            mobile_run,
        );
    }

    // Dart is checked AFTER Flutter: a Flutter app also has a `pubspec.yaml`, so
    // the mobile detector must win; a bare `pubspec.yaml` is plain Dart.
    if path.join("pubspec.yaml").exists() {
        return detection(ProjectType::Dart, 0, Some("dart run".into()));
    }

    if has_child_with_extension(path, "xcworkspace") || has_child_with_extension(path, "xcodeproj")
    {
        let mobile_run = detect_xcode_run(path);
        return detection_with_mobile(
            ProjectType::Xcode,
            mobile_start_command(ProjectType::Xcode, mobile_run.as_ref()),
            mobile_run,
        );
    }

    // Swift is checked AFTER Xcode: an iOS app may carry a `Package.swift` for
    // dependencies, so the Xcode detector wins; a bare SPM package or a Vapor
    // server has no `.xcodeproj` and lands here.
    if path.join("Package.swift").exists() {
        return detect_swift_project(path);
    }

    if is_android_project(path) {
        let mobile_run = detect_android_run(path);
        return detection_with_mobile(
            ProjectType::Android,
            mobile_start_command(ProjectType::Android, mobile_run.as_ref()),
            mobile_run,
        );
    }

    // JVM (Maven/Gradle) is checked AFTER Android: an Android project also has
    // a `build.gradle`, so the mobile detector must win first.
    if has_jvm_project(path) {
        return detect_jvm_project(path);
    }

    if path.join("index.html").exists() {
        return detection(ProjectType::Static, 8000, None);
    }

    detection(ProjectType::Custom, 3000, None)
}

/// Detect a Ruby project and infer its dev server. Rails/Jekyll/Sinatra/Hanami
/// share the Ruby runtime; the framework tag drives the logo + smart defaults.
fn detect_ruby_project(path: &Path) -> ProjectDetection {
    let gemfile = std::fs::read_to_string(path.join("Gemfile"))
        .unwrap_or_default()
        .to_lowercase();
    if gemfile.contains("\"rails\"")
        || gemfile.contains("'rails'")
        || path.join("bin/rails").exists()
    {
        return detection_fw(
            ProjectType::Ruby,
            Framework::Rails,
            3000,
            Some("bin/rails server -p 3000".into()),
        );
    }
    if gemfile.contains("jekyll") || path.join("_config.yml").exists() {
        return detection_fw(
            ProjectType::Ruby,
            Framework::Jekyll,
            4000,
            Some("bundle exec jekyll serve --port 4000".into()),
        );
    }
    if gemfile.contains("sinatra") {
        return detection_fw(
            ProjectType::Ruby,
            Framework::Sinatra,
            4567,
            Some("bundle exec ruby app.rb".into()),
        );
    }
    if gemfile.contains("hanami") {
        return detection_fw(
            ProjectType::Ruby,
            Framework::Hanami,
            2300,
            Some("bundle exec hanami server".into()),
        );
    }
    // Plain Ruby project: runtime pinned, but no inferable web server.
    detection(
        ProjectType::Ruby,
        0,
        Some("bundle exec ruby main.rb".into()),
    )
}

/// Detect a Go project. Web frameworks all launch via `go run .` (the bind port
/// lives in code, not the manifest), so the port is the framework's quickstart
/// default and the tag drives the logo.
fn detect_go_project(path: &Path) -> ProjectDetection {
    let gomod = std::fs::read_to_string(path.join("go.mod"))
        .unwrap_or_default()
        .to_lowercase();
    let cmd = Some("go run .".to_string());
    if gomod.contains("gin-gonic/gin") {
        return detection_fw(ProjectType::Go, Framework::Gin, 8080, cmd);
    }
    if gomod.contains("labstack/echo") {
        return detection_fw(ProjectType::Go, Framework::Echo, 1323, cmd);
    }
    if gomod.contains("gofiber/fiber") {
        return detection_fw(ProjectType::Go, Framework::Fiber, 3000, cmd);
    }
    // Plain Go: a CLI or a server binding a port we can't infer. Process-only
    // until the user sets a port.
    detection(ProjectType::Go, 0, cmd)
}

/// Detect a Rust project. Web frameworks launch via `cargo run`; the port is
/// each framework's quickstart default.
fn detect_rust_project(path: &Path) -> ProjectDetection {
    let cargo = std::fs::read_to_string(path.join("Cargo.toml"))
        .unwrap_or_default()
        .to_lowercase();
    let cmd = Some("cargo run".to_string());
    if cargo.contains("actix-web") {
        return detection_fw(ProjectType::Rust, Framework::Actix, 8080, cmd);
    }
    if cargo.contains("axum") {
        return detection_fw(ProjectType::Rust, Framework::Axum, 3000, cmd);
    }
    if cargo.contains("rocket") {
        return detection_fw(ProjectType::Rust, Framework::Rocket, 8000, cmd);
    }
    if cargo.contains("leptos") {
        return detection_fw(ProjectType::Rust, Framework::Leptos, 3000, cmd);
    }
    detection(ProjectType::Rust, 0, cmd)
}

/// Detect a Deno project. Fresh is the common full-stack framework; everything
/// else runs through the project's own `deno task`.
fn detect_deno_project(path: &Path) -> ProjectDetection {
    let cfg = std::fs::read_to_string(path.join("deno.json"))
        .or_else(|_| std::fs::read_to_string(path.join("deno.jsonc")))
        .unwrap_or_default()
        .to_lowercase();
    if cfg.contains("$fresh/") || cfg.contains("fresh") || path.join("fresh.gen.ts").exists() {
        return detection_fw(
            ProjectType::Deno,
            Framework::Fresh,
            8000,
            Some("deno task start".into()),
        );
    }
    detection(ProjectType::Deno, 0, Some("deno task dev".into()))
}

/// Detect an Elixir project. Phoenix is the dominant web framework; bare Mix
/// projects run as a long-lived process.
fn detect_elixir_project(path: &Path) -> ProjectDetection {
    let mix = std::fs::read_to_string(path.join("mix.exs"))
        .unwrap_or_default()
        .to_lowercase();
    if mix.contains(":phoenix") || mix.contains("phoenix,") {
        return detection_fw(
            ProjectType::Elixir,
            Framework::Phoenix,
            4000,
            Some("mix phx.server".into()),
        );
    }
    detection(ProjectType::Elixir, 0, Some("mix run --no-halt".into()))
}

/// Detect a .NET project. A `Microsoft.NET.Sdk.Web` SDK marks an ASP.NET web
/// app (Kestrel on :5000); anything else is a plain `dotnet run` process.
fn detect_dotnet_project(path: &Path) -> ProjectDetection {
    let is_web = dotnet_project_files(path)
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .any(|body| body.contains("Microsoft.NET.Sdk.Web"));
    let cmd = Some("dotnet run".to_string());
    if is_web {
        return detection_fw(ProjectType::DotNet, Framework::AspNet, 5000, cmd);
    }
    detection(ProjectType::DotNet, 0, cmd)
}

/// Detect a JVM (Java/Kotlin) project built with Maven or Gradle. The language
/// is inferred from the source tree; the framework (Spring/Ktor) from the build
/// file. Spring Boot and Ktor both default to :8080.
fn detect_jvm_project(path: &Path) -> ProjectDetection {
    let mut build = String::new();
    for name in [
        "pom.xml",
        "build.gradle",
        "build.gradle.kts",
        "settings.gradle",
    ] {
        if let Ok(body) = std::fs::read_to_string(path.join(name)) {
            build.push_str(&body);
            build.push('\n');
        }
    }
    let build = build.to_lowercase();
    let is_maven = path.join("pom.xml").exists();
    let kind = if path.join("src/main/kotlin").is_dir() || path.join("build.gradle.kts").exists() {
        ProjectType::Kotlin
    } else {
        ProjectType::Java
    };

    let spring = build.contains("spring-boot") || build.contains("springframework.boot");
    let ktor = build.contains("io.ktor") || build.contains("ktor-server");

    let framework = if spring {
        Some(Framework::Spring)
    } else if ktor {
        Some(Framework::Ktor)
    } else {
        None
    };

    let cmd = match (is_maven, spring) {
        (true, true) => "./mvnw spring-boot:run",
        (true, false) => "./mvnw compile exec:java",
        (false, true) => "./gradlew bootRun",
        (false, false) => "./gradlew run",
    };
    let port = if spring || ktor { 8080 } else { 0 };

    match framework {
        Some(fw) => detection_fw(kind, fw, port, Some(cmd.into())),
        None => detection(kind, port, Some(cmd.into())),
    }
}

/// Hugo static-site generator: a `hugo.*` config, or a legacy `config.*` paired
/// with Hugo's scaffold dirs. Requiring the scaffold avoids false positives on
/// the many tools that ship a bare `config.toml`.
fn is_hugo_project(path: &Path) -> bool {
    let hugo_config = ["hugo.toml", "hugo.yaml", "hugo.yml", "hugo.json"]
        .iter()
        .any(|f| path.join(f).exists());
    if hugo_config {
        return true;
    }
    let legacy_config = ["config.toml", "config.yaml", "config.yml", "config.json"]
        .iter()
        .any(|f| path.join(f).exists());
    legacy_config && (path.join("archetypes").is_dir() || path.join("content").is_dir())
}

/// `.csproj`/`.fsproj`/`.sln` files in the project root identify a .NET project.
fn dotnet_project_files(path: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                e.eq_ignore_ascii_case("csproj")
                    || e.eq_ignore_ascii_case("fsproj")
                    || e.eq_ignore_ascii_case("sln")
            }) {
                out.push(p);
            }
        }
    }
    out
}

fn has_dotnet_project(path: &Path) -> bool {
    !dotnet_project_files(path).is_empty()
}

fn has_jvm_project(path: &Path) -> bool {
    path.join("pom.xml").exists()
        || path.join("build.gradle").exists()
        || path.join("build.gradle.kts").exists()
}

/// Detect a Swift package. The Vapor server framework defaults to :8080; a bare
/// SPM package runs as a `swift run` process.
fn detect_swift_project(path: &Path) -> ProjectDetection {
    let manifest = std::fs::read_to_string(path.join("Package.swift"))
        .unwrap_or_default()
        .to_lowercase();
    if manifest.contains("vapor") {
        return detection_fw(
            ProjectType::Swift,
            Framework::Vapor,
            8080,
            Some("swift run".into()),
        );
    }
    detection(ProjectType::Swift, 0, Some("swift run".into()))
}

fn detect_language_intelligence(
    path: &Path,
    kind: ProjectType,
) -> Vec<LanguageIntelligenceCapability> {
    let mut out = Vec::new();
    let mut push = |language: &str, label: &str, files: Vec<String>, setup: &str| {
        if files.is_empty() {
            return;
        }
        out.push(LanguageIntelligenceCapability {
            language: language.into(),
            label: label.into(),
            files,
            features: vec![
                "diagnostics".into(),
                "completions".into(),
                "hover_help".into(),
            ],
            setup: setup.into(),
        });
    };

    let js_files = existing_files(
        path,
        &[
            "package.json",
            "tsconfig.json",
            "jsconfig.json",
            "vite.config.ts",
            "vite.config.js",
            "next.config.js",
            "next.config.mjs",
            "svelte.config.js",
        ],
    );
    if !js_files.is_empty()
        || matches!(
            kind,
            ProjectType::Next | ProjectType::Vite | ProjectType::Node | ProjectType::Expo
        )
    {
        push(
            "javascript_typescript",
            "JavaScript / TypeScript",
            ensure_marker(js_files, "package.json"),
            "Built in: syntax checks, package/tsconfig key completions, JS/TS keyword help.",
        );
    }

    push(
        "php",
        "PHP",
        existing_files(path, &["composer.json", "index.php", "public/index.php"]),
        "Built in: PHP keyword help and lightweight response-path diagnostics.",
    );
    push(
        "python",
        "Python",
        existing_files(
            path,
            &["pyproject.toml", "requirements.txt", "Pipfile", "manage.py"],
        ),
        "Built in: indentation/block diagnostics and Python keyword help.",
    );
    push(
        "sql",
        "SQL",
        existing_files(path, &["schema.sql", "database.sql", "dump.sql"]),
        "Built in: SQL keyword help and destructive-query warnings. The database workbench adds schema-aware completion.",
    );
    push(
        "config",
        "JSON / YAML / TOML / env",
        existing_files(
            path,
            &[
                "package.json",
                "tsconfig.json",
                "composer.json",
                "pyproject.toml",
                "Cargo.toml",
                "wrangler.toml",
                "docker-compose.yml",
                "docker-compose.yaml",
                ".env",
                ".env.local",
            ],
        ),
        "Built in: parser/format diagnostics and common config key completions.",
    );

    out
}

fn existing_files(path: &Path, names: &[&str]) -> Vec<String> {
    names
        .iter()
        .filter(|name| path.join(name).exists())
        .map(|name| (*name).to_string())
        .collect()
}

fn ensure_marker(mut files: Vec<String>, marker: &str) -> Vec<String> {
    if files.is_empty() {
        files.push(marker.into());
    }
    files
}

fn detection(kind: ProjectType, port: u16, start_command: Option<String>) -> ProjectDetection {
    ProjectDetection {
        kind,
        framework: None,
        port: (port > 0).then_some(port),
        start_command,
        document_root: None,
        php_version: None,
        web_server: None,
        mobile_run: None,
    }
}

/// Like [`detection`], but tags the result with a detected sub-stack. Used by
/// the per-language detectors (PHP/Python/Ruby/Go/…) so the framework rides
/// along with the kind + port + command.
fn detection_fw(
    kind: ProjectType,
    framework: Framework,
    port: u16,
    start_command: Option<String>,
) -> ProjectDetection {
    ProjectDetection {
        framework: Some(framework),
        ..detection(kind, port, start_command)
    }
}

fn detection_with_mobile(
    kind: ProjectType,
    start_command: Option<String>,
    mobile_run: Option<MobileRunConfig>,
) -> ProjectDetection {
    ProjectDetection {
        kind,
        framework: None,
        port: None,
        start_command,
        document_root: None,
        php_version: None,
        web_server: None,
        mobile_run,
    }
}

fn detect_php_project(path: &Path) -> ProjectDetection {
    let port = 8000;
    let version = detected_runtime_for(ProjectType::Php).map(|rt| rt.version);
    let composer = std::fs::read_to_string(path.join("composer.json")).unwrap_or_default();
    let (framework, fw_doc_root) = detect_php_framework(path, &composer);

    // Document root: prefer the framework's conventional web root when that dir
    // actually exists (Laravel `public/`, Drupal/Craft `web/`, CakePHP
    // `webroot/`, Magento `pub/`); otherwise fall back to the generic `public/`
    // sniff so a hand-rolled PHP app still resolves correctly.
    let document_root = fw_doc_root
        .filter(|dir| path.join(dir).is_dir())
        .map(|dir| dir.to_string())
        .or_else(|| {
            let public = path.join("public");
            (public.join("index.php").exists() || public.join("router.php").exists())
                .then(|| "public".to_string())
        });

    ProjectDetection {
        kind: ProjectType::Php,
        framework,
        port: Some(port),
        start_command: None,
        document_root,
        php_version: version,
        web_server: None,
        mobile_run: None,
    }
}

/// Identify the PHP framework / CMS and its conventional web root from the
/// composer manifest plus a few signature files. Order matters: CMSs built on
/// another framework (Statamic on Laravel, Craft on Yii) must be matched before
/// their base, since the base dependency is also present.
fn detect_php_framework(path: &Path, composer: &str) -> (Option<Framework>, Option<&'static str>) {
    // WordPress usually ships no composer manifest — detect by its loader files.
    // Bedrock (`roots/wordpress`) serves from `web/` with WordPress in `web/wp`;
    // a classic install serves from the project root.
    if composer.contains("roots/wordpress") {
        return (Some(Framework::WordPress), Some("web"));
    }
    if path.join("wp-config.php").exists()
        || path.join("wp-load.php").exists()
        || path.join("wp-settings.php").exists()
        || composer.contains("johnpbloch/wordpress")
    {
        return (Some(Framework::WordPress), None);
    }
    if composer.contains("statamic/cms") {
        return (Some(Framework::Statamic), Some("public"));
    }
    if composer.contains("laravel/framework") || path.join("artisan").exists() {
        return (Some(Framework::Laravel), Some("public"));
    }
    if composer.contains("craftcms/cms") {
        return (Some(Framework::CraftCms), Some("web"));
    }
    if composer.contains("drupal/core") {
        return (Some(Framework::Drupal), Some("web"));
    }
    if composer.contains("symfony/framework-bundle") || composer.contains("symfony/symfony") {
        return (Some(Framework::Symfony), Some("public"));
    }
    if composer.contains("codeigniter4/framework") {
        return (Some(Framework::CodeIgniter), Some("public"));
    }
    if composer.contains("cakephp/cakephp") {
        return (Some(Framework::CakePhp), Some("webroot"));
    }
    if composer.contains("yiisoft/yii2") || composer.contains("yiisoft/yii-") {
        return (Some(Framework::Yii), Some("web"));
    }
    if composer.contains("magento/product-community-edition")
        || composer.contains("magento/magento2-base")
    {
        return (Some(Framework::Magento), Some("pub"));
    }
    if composer.contains("slim/slim") {
        return (Some(Framework::Slim), Some("public"));
    }
    // Joomla: signature admin tree + root configuration file.
    if path.join("configuration.php").exists() && path.join("administrator").is_dir() {
        return (Some(Framework::Joomla), None);
    }
    (None, None)
}

/// Detect a Python project and infer its run command.
///
/// A recognised web framework (Django/FastAPI/Flask) gets a default — but
/// editable — dev command bound to port 8000, which Caddy reverse-proxies just
/// like PHP. Anything else (a plain script, a research / LLM-eval harness, a
/// library) gets no port and no command: it runs as a board/process-only
/// project whose Python runtime is still pinned by the runtime layer. We never
/// hide an inferred command — it lands in `start_command` for the user to edit.
fn detect_python_project(path: &Path) -> ProjectDetection {
    // Django ships a `manage.py`; its dev server is unambiguous.
    if path.join("manage.py").exists() {
        return detection_fw(
            ProjectType::Python,
            Framework::Django,
            8000,
            Some("python manage.py runserver 0.0.0.0:8000".into()),
        );
    }

    // The rest are libraries, not marker files — sniff the declared deps. The
    // entrypoint module name varies between projects, so each command is a
    // best-effort default the user is expected to confirm or edit.
    let deps = python_dependency_text(path);
    if deps.contains("streamlit") {
        return detection_fw(
            ProjectType::Python,
            Framework::Streamlit,
            8501,
            Some("streamlit run app.py".into()),
        );
    }
    if deps.contains("reflex") {
        return detection_fw(
            ProjectType::Python,
            Framework::Reflex,
            3000,
            Some("reflex run".into()),
        );
    }
    if deps.contains("gradio") {
        return detection_fw(
            ProjectType::Python,
            Framework::Gradio,
            7860,
            Some("python app.py".into()),
        );
    }
    if deps.contains("fastapi") {
        return detection_fw(
            ProjectType::Python,
            Framework::FastApi,
            8000,
            Some("uvicorn main:app --reload --port 8000".into()),
        );
    }
    if deps.contains("flask") {
        return detection_fw(
            ProjectType::Python,
            Framework::Flask,
            8000,
            Some("flask run --port 8000".into()),
        );
    }

    // No web framework: a script / research / library project. No server, so no
    // port and no start command — it behaves like a board/process-only project.
    detection(ProjectType::Python, 0, None)
}

/// Marker files that identify a Python project.
fn is_python_project(path: &Path) -> bool {
    path.join("pyproject.toml").exists()
        || path.join("requirements.txt").exists()
        || path.join("setup.py").exists()
        || path.join("Pipfile").exists()
        || path.join("manage.py").exists()
}

/// Concatenated, lower-cased text of a Python project's dependency manifests,
/// for cheap substring sniffing (mirrors the `package.json` `contains` check
/// used for JS frameworks).
fn python_dependency_text(path: &Path) -> String {
    let mut text = String::new();
    for name in ["requirements.txt", "pyproject.toml", "Pipfile", "setup.py"] {
        if let Ok(body) = std::fs::read_to_string(path.join(name)) {
            text.push_str(&body);
            text.push('\n');
        }
    }
    text.to_lowercase()
}

fn detect_flutter_run(_path: &Path) -> Option<MobileRunConfig> {
    Some(MobileRunConfig::default())
}

fn detect_xcode_run(path: &Path) -> Option<MobileRunConfig> {
    Some(MobileRunConfig {
        target: find_xcode_scheme(path),
        flavor: None,
        device: None,
    })
}

fn detect_android_run(path: &Path) -> Option<MobileRunConfig> {
    Some(MobileRunConfig {
        target: find_android_module(path).or_else(|| Some("app".into())),
        flavor: Some("debug".into()),
        device: None,
    })
}

/// Expo run config. `device` ("ios"/"android") selects which simulator
/// `npx expo start` auto-opens; left unset so the user picks i/a in Metro.
fn detect_expo_run(_path: &Path) -> Option<MobileRunConfig> {
    Some(MobileRunConfig::default())
}

/// The Play command for a detected mobile project. Delegates to the single
/// source of truth in [`crate::mobile`], which generates a complete launch
/// (boot simulator/emulator → build → install → launch → attach logs) rather
/// than a bare build/install — so Play actually opens the app in its simulator.
fn mobile_start_command(kind: ProjectType, cfg: Option<&MobileRunConfig>) -> Option<String> {
    let cfg = cfg.cloned().unwrap_or_default();
    crate::mobile::launch_command_for(kind, &cfg)
}

fn has_php_index(path: &Path) -> bool {
    path.join("index.php").exists() || path.join("public").join("index.php").exists()
}

fn is_flutter_project(path: &Path) -> bool {
    path.join("pubspec.yaml").exists()
        && (path.join("lib").join("main.dart").exists()
            || path.join("android").is_dir()
            || path.join("ios").is_dir())
}

fn is_android_project(path: &Path) -> bool {
    path.join("gradlew").exists()
        && (path.join("settings.gradle").exists() || path.join("settings.gradle.kts").exists())
        && (path.join("app").join("build.gradle").exists()
            || path.join("app").join("build.gradle.kts").exists()
            || path.join("build.gradle").exists()
            || path.join("build.gradle.kts").exists())
}

fn find_xcode_scheme(path: &Path) -> Option<String> {
    let entries = std::fs::read_dir(path).ok()?;
    for entry in entries.flatten() {
        let root = entry.path();
        let ext = root.extension().and_then(|s| s.to_str());
        if !matches!(ext, Some("xcworkspace") | Some("xcodeproj")) {
            continue;
        }
        let schemes = root.join("xcshareddata").join("xcschemes");
        if let Some(scheme) = first_scheme_file(&schemes) {
            return Some(scheme);
        }
    }
    first_child_stem_with_extension(path, "xcodeproj")
        .or_else(|| first_child_stem_with_extension(path, "xcworkspace"))
}

fn first_scheme_file(path: &Path) -> Option<String> {
    let entries = std::fs::read_dir(path).ok()?;
    entries.flatten().find_map(|entry| {
        let path = entry.path();
        (path.extension().and_then(|s| s.to_str()) == Some("xcscheme"))
            .then(|| path.file_stem()?.to_str().map(str::to_string))
            .flatten()
    })
}

fn first_child_stem_with_extension(path: &Path, ext: &str) -> Option<String> {
    let entries = std::fs::read_dir(path).ok()?;
    entries.flatten().find_map(|entry| {
        let path = entry.path();
        (path.extension().and_then(|s| s.to_str()) == Some(ext))
            .then(|| path.file_stem()?.to_str().map(str::to_string))
            .flatten()
    })
}

fn find_android_module(path: &Path) -> Option<String> {
    for module in ["app", "mobile", "androidApp"] {
        if path.join(module).join("build.gradle").exists()
            || path.join(module).join("build.gradle.kts").exists()
        {
            return Some(module.into());
        }
    }
    None
}

fn has_child_with_extension(path: &Path, ext: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(path) else {
        return false;
    };
    entries.flatten().any(|entry| {
        entry
            .path()
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s == ext)
    })
}

fn detected_runtime_for(kind: ProjectType) -> Option<Runtime> {
    match kind {
        ProjectType::Php => crate::php::detect_all()
            .into_iter()
            .next()
            .map(|p| Runtime {
                lang: "php".into(),
                version: p.version,
            }),
        ProjectType::Flutter => crate::runtimes::runtime_by_id("flutter")?
            .detect()
            .into_iter()
            .next()
            .map(|p| Runtime {
                lang: "flutter".into(),
                version: p.version,
            }),
        _ => None,
    }
}

fn default_services(kind: ProjectType, https: bool, has_start_command: bool) -> Vec<String> {
    match kind {
        ProjectType::Flutter | ProjectType::Xcode | ProjectType::Android | ProjectType::Expo => {
            vec![]
        }
        ProjectType::Php if has_start_command => vec!["caddy".into()],
        ProjectType::Php => vec!["caddy".into(), "php-fpm".into()],
        _ if https => vec!["caddy".into()],
        _ => vec![],
    }
}

/// `remove_project(id)` — drop the entry from the registry. The
/// reconciler handles cert-dir reaping, hosts removal, Caddy route
/// deletion, and PC YAML regeneration on the next tick (kicked
/// immediately via `mark_dirty`).
#[tauri::command]
pub async fn remove_project(state: State<'_, AppState>, id: String) -> AppResult<()> {
    let mut registry = load_registry(&state)?;
    let pid = ProjectId::new(id.clone());
    let _removed = registry.remove_project(&pid)?;
    save_registry(&state, &registry)?;
    state.invalidate_icon(&id);
    state.clear_proc_log(&id);
    state.reconciler.mark_dirty();
    Ok(())
}

/// `set_xdebug_mode(id, mode)` — flip a PHP project's `XDEBUG_MODE` env
/// var. Passing `"off"` (or an empty string) deletes the var entirely;
/// any other value sets it. This is a project-env mutation, not PHP
/// detection — it persists through the same dirty-and-reconcile flow as
/// `update_project`, so the next PC tick re-spawns the project's entry.
#[tauri::command]
pub async fn set_xdebug_mode(
    state: State<'_, AppState>,
    id: String,
    mode: String,
) -> AppResult<ProjectView> {
    let mut registry = load_registry(&state)?;
    let pid = ProjectId::new(id.clone());

    let project = registry
        .get_project_mut(&pid)
        .ok_or_else(|| AppError::NotFound(id.clone()))?;

    let mut env: BTreeMap<String, String> = project.env.clone();
    let mode = mode.trim();
    if mode.is_empty() || mode.eq_ignore_ascii_case("off") {
        env.remove("XDEBUG_MODE");
    } else {
        env.insert("XDEBUG_MODE".into(), mode.to_string());
    }
    project.env = env;

    let snapshot = project.clone();
    save_registry(&state, &registry)?;
    state.reconciler.mark_dirty();

    Ok(ProjectView::from_project(&snapshot, None))
}

/// `project_get_deploy(id)` — the project's saved deploy config, or `None`.
#[tauri::command]
pub async fn project_get_deploy(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<Option<ProjectDeploy>> {
    let registry = load_registry(&state)?;
    let project = registry
        .get_project(&ProjectId::new(id.clone()))
        .ok_or_else(|| AppError::NotFound(id.clone()))?;
    Ok(project.deploy.clone())
}

/// `project_set_deploy(id, deploy)` — save (or clear, with `None`) a project's
/// deploy target. Steps are trimmed and blank rows dropped; a config with no
/// host + remote path is treated as "clear" so the editor can persist an empty
/// form without leaving a half-set target behind.
#[tauri::command]
pub async fn project_set_deploy(
    state: State<'_, AppState>,
    id: String,
    deploy: Option<ProjectDeploy>,
) -> AppResult<ProjectView> {
    let mut registry = load_registry(&state)?;
    let pid = ProjectId::new(id.clone());
    let project = registry
        .get_project_mut(&pid)
        .ok_or_else(|| AppError::NotFound(id.clone()))?;

    project.deploy = deploy.and_then(|mut d| {
        d.remote_path = d.remote_path.trim().to_string();
        d.local_subdir = d
            .local_subdir
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        d.steps = d
            .steps
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        d.exclude = d
            .exclude
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        d.is_active().then_some(d)
    });

    let snapshot = project.clone();
    save_registry(&state, &registry)?;
    Ok(ProjectView::from_project(&snapshot, None))
}

// =============================================================================
// Helpers shared with other command modules
// =============================================================================

pub(crate) fn load_registry(state: &AppState) -> AppResult<Registry> {
    store::load_or_default(&state.registry_path, &state.domain_suffix).map_err(AppError::Registry)
}

pub(crate) fn save_registry(state: &AppState, reg: &Registry) -> AppResult<()> {
    store::save_to(reg, &state.registry_path).map_err(AppError::Registry)
}

/// Snapshot PC's `/processes` keyed by name. Returns `None` if the daemon
/// is unreachable (graceful degradation — same as the CLI).
pub(crate) async fn fetch_pc_state(state: &AppState) -> Option<HashMap<String, Process>> {
    // Take the client out of the mutex briefly, drop the guard, then await.
    let client = {
        let g = state.pc_client.lock().unwrap_or_else(|e| e.into_inner());
        g.clone()?
    };
    let processes = client.processes().await.ok()?;
    Some(processes.into_iter().map(|p| (p.name.clone(), p)).collect())
}

// Single source of truth lives in `crate::util`. Re-exported here so the
// established `crate::commands::projects::slugify` path keeps resolving.
pub(crate) use crate::util::slugify;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_runtime_inherited_per_language() {
        let mut defaults = BTreeMap::new();
        defaults.insert("node".to_string(), "22".to_string());
        defaults.insert("php".to_string(), "8.3".to_string());
        let settings = crate::registry::RuntimeSettings {
            defaults,
            ..Default::default()
        };

        assert_eq!(
            settings.default_for(ProjectType::Next),
            Some(crate::registry::Runtime {
                lang: "node".into(),
                version: "22".into()
            })
        );
        assert_eq!(
            settings.default_for(ProjectType::Php),
            Some(crate::registry::Runtime {
                lang: "php".into(),
                version: "8.3".into()
            })
        );
        // Static/Custom have no managed runtime.
        assert_eq!(settings.default_for(ProjectType::Static), None);
    }

    #[test]
    fn no_default_set_yields_no_runtime() {
        let settings = crate::registry::RuntimeSettings::default();
        assert_eq!(settings.default_for(ProjectType::Next), None);
    }

    #[test]
    fn project_type_auto_detects_language_environment() {
        // End-to-end of the add-project chain: a folder is classified by
        // `detect_kind`, and that kind drives the managed language runtime via
        // `default_for`. Proves the new framework/runtime kinds resolve to the
        // right language environment (or `None` for languages PortBay doesn't
        // manage a runtime for — they run on the system toolchain).
        let mut defaults = BTreeMap::new();
        for (lang, ver) in [
            ("node", "22"),
            ("php", "8.3"),
            ("python", "3.12"),
            ("go", "1.23"),
            ("ruby", "3.3"),
            ("flutter", "3.24"),
        ] {
            defaults.insert(lang.to_string(), ver.to_string());
        }
        let settings = crate::registry::RuntimeSettings {
            defaults,
            ..Default::default()
        };
        let lang_for = |dir: &Path| settings.default_for(detect_kind(dir).kind).map(|r| r.lang);

        let cases: &[(&str, &dyn Fn(&Path), Option<&str>)] = &[
            // JS meta-framework on the Node runtime.
            (
                "astro",
                &|d: &Path| write_js_project(d, r#"{ "dependencies": { "astro": "4" } }"#, None),
                Some("node"),
            ),
            // PHP framework.
            (
                "laravel",
                &|d: &Path| {
                    write_file(
                        d,
                        "composer.json",
                        r#"{ "require": { "laravel/framework": "^11" } }"#,
                    )
                },
                Some("php"),
            ),
            // Python framework.
            (
                "django",
                &|d: &Path| write_file(d, "manage.py", ""),
                Some("python"),
            ),
            // Ruby framework -> ruby runtime.
            (
                "rails",
                &|d: &Path| write_file(d, "Gemfile", "gem \"rails\"\n"),
                Some("ruby"),
            ),
            // Go.
            (
                "go",
                &|d: &Path| write_file(d, "go.mod", "module app\n"),
                Some("go"),
            ),
            // Languages PortBay has no managed runtime for resolve to None.
            (
                "rust",
                &|d: &Path| write_file(d, "Cargo.toml", "[package]\nname=\"x\"\n"),
                None,
            ),
            ("deno", &|d: &Path| write_file(d, "deno.json", "{}\n"), None),
            (
                "elixir",
                &|d: &Path| write_file(d, "mix.exs", "defmodule X do\nend\n"),
                None,
            ),
        ];

        for (name, setup, expected) in cases {
            let dir = tempfile::tempdir().unwrap();
            setup(dir.path());
            assert_eq!(
                lang_for(dir.path()).as_deref(),
                *expected,
                "language environment for {name} project",
            );
        }
    }

    #[test]
    fn slugify_matches_cli_behaviour() {
        assert_eq!(slugify("Marketing Site"), "marketing-site");
        assert_eq!(slugify("API Gateway"), "api-gateway");
        assert_eq!(slugify("__weird___name__"), "weird-name");
        assert_eq!(slugify("UPPER"), "upper");
    }

    #[test]
    fn sandbox_clone_rejects_local_or_malformed_git_urls() {
        assert!(validate_git_url("https://github.com/portbay-app/portbay.git").is_ok());
        assert!(validate_git_url("git@github.com:portbay-app/portbay.git").is_ok());
        assert!(validate_git_url("file:///tmp/repo.git").is_err());
        assert!(validate_git_url("/tmp/repo").is_err());
        assert!(validate_git_url("git@github.com:").is_err());
        assert!(validate_git_url("https://github.com/org/repo.git\n--upload-pack=sh").is_err());
    }

    #[test]
    fn sandbox_clone_rejects_system_install_roots() {
        assert!(validate_sandbox_import_parent(Path::new("/")).is_err());
        assert!(validate_sandbox_import_parent(Path::new("/System")).is_err());
        assert!(validate_sandbox_import_parent(Path::new("/Volumes")).is_err());
        assert!(validate_sandbox_import_parent(Path::new("/Volumes/DevSSD/Projects")).is_ok());
    }

    #[test]
    fn repo_slug_falls_back_for_empty_repo_names() {
        assert_eq!(repo_slug("https://github.com/"), "project");
        assert_eq!(repo_slug("https://github.com/org/my-app.git"), "my-app");
    }

    #[test]
    fn canonical_project_folder_accepts_directories() {
        let dir = tempfile::tempdir().unwrap();
        let path = canonical_project_folder(dir.path().to_str().unwrap()).unwrap();
        assert!(path.is_dir());
    }

    #[test]
    fn canonical_project_folder_rejects_files() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("index.html");
        std::fs::write(&file, "<h1>nope</h1>").unwrap();
        let err = canonical_project_folder(file.to_str().unwrap()).unwrap_err();
        assert!(err.to_string().contains("folder"));
    }

    /// Write a `package.json` with the given raw fields plus a framework dep,
    /// then optionally drop a lockfile beside it.
    fn write_js_project(dir: &Path, package_json: &str, lockfile: Option<&str>) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("package.json"), package_json).unwrap();
        if let Some(name) = lockfile {
            std::fs::write(dir.join(name), "").unwrap();
        }
    }

    #[test]
    fn detect_kind_bun_lockfile_yields_bun_run_dev() {
        let dir = tempfile::tempdir().unwrap();
        write_js_project(
            dir.path(),
            r#"{ "name": "app", "dependencies": { "next": "14" } }"#,
            Some("bun.lockb"),
        );
        let detected = detect_kind(dir.path());
        assert_eq!(detected.kind, ProjectType::Next);
        assert_eq!(detected.port, Some(3000));
        assert_eq!(detected.start_command.as_deref(), Some("bun run dev"));
    }

    #[test]
    fn detect_kind_npm_lockfile_yields_npm_run_dev() {
        let dir = tempfile::tempdir().unwrap();
        write_js_project(
            dir.path(),
            r#"{ "name": "app", "devDependencies": { "vite": "5" } }"#,
            Some("package-lock.json"),
        );
        let detected = detect_kind(dir.path());
        assert_eq!(detected.kind, ProjectType::Vite);
        assert_eq!(detected.start_command.as_deref(), Some("npm run dev"));
    }

    #[test]
    fn detect_kind_astro_project_uses_astro_port() {
        let dir = tempfile::tempdir().unwrap();
        write_js_project(
            dir.path(),
            r#"{ "name": "blog", "dependencies": { "astro": "4" } }"#,
            None,
        );
        let detected = detect_kind(dir.path());
        assert_eq!(detected.kind, ProjectType::Astro);
        // Astro dev binds :4321 — PortBay proxies that port, so it must match.
        assert_eq!(detected.port, Some(4321));
        assert_eq!(detected.start_command.as_deref(), Some("pnpm dev"));
    }

    #[test]
    fn detect_kind_sveltekit_beats_generic_vite() {
        // SvelteKit carries Vite as a dependency; the specific framework marker
        // must win over the generic `"vite"` branch.
        let dir = tempfile::tempdir().unwrap();
        write_js_project(
            dir.path(),
            r#"{ "name": "app", "devDependencies": { "@sveltejs/kit": "2", "vite": "5" } }"#,
            None,
        );
        let detected = detect_kind(dir.path());
        assert_eq!(detected.kind, ProjectType::SvelteKit);
        assert_eq!(detected.port, Some(5173));
    }

    #[test]
    fn detect_kind_nuxt_project_uses_nuxt_port() {
        let dir = tempfile::tempdir().unwrap();
        write_js_project(
            dir.path(),
            r#"{ "name": "site", "dependencies": { "nuxt": "3" } }"#,
            None,
        );
        let detected = detect_kind(dir.path());
        assert_eq!(detected.kind, ProjectType::Nuxt);
        assert_eq!(detected.port, Some(3000));
    }

    #[test]
    fn detect_kind_remix_project_detected() {
        let dir = tempfile::tempdir().unwrap();
        write_js_project(
            dir.path(),
            r#"{ "name": "app", "dependencies": { "@remix-run/react": "2" } }"#,
            None,
        );
        let detected = detect_kind(dir.path());
        assert_eq!(detected.kind, ProjectType::Remix);
        assert_eq!(detected.port, Some(3000));
    }

    #[test]
    fn detect_kind_angular_runs_ng_serve_on_4200() {
        // Angular scaffolds `start: ng serve`, not `dev`, so the command runs
        // the binary directly via the package manager's exec.
        let dir = tempfile::tempdir().unwrap();
        write_js_project(
            dir.path(),
            r#"{ "name": "app", "dependencies": { "@angular/core": "18" } }"#,
            Some("package-lock.json"),
        );
        let detected = detect_kind(dir.path());
        assert_eq!(detected.kind, ProjectType::Angular);
        assert_eq!(detected.port, Some(4200));
        assert_eq!(
            detected.start_command.as_deref(),
            Some("npm exec -- ng serve")
        );
    }

    #[test]
    fn detect_kind_gatsby_runs_gatsby_develop() {
        let dir = tempfile::tempdir().unwrap();
        write_js_project(
            dir.path(),
            r#"{ "name": "site", "dependencies": { "gatsby": "5" } }"#,
            None,
        );
        let detected = detect_kind(dir.path());
        assert_eq!(detected.kind, ProjectType::Gatsby);
        assert_eq!(detected.port, Some(8000));
        assert_eq!(
            detected.start_command.as_deref(),
            Some("pnpm exec gatsby develop")
        );
    }

    #[test]
    fn detect_kind_vue_cli_runs_vue_cli_service() {
        // The `@vue/cli-service` marker distinguishes Vue CLI (webpack, :8080)
        // from a Vite-based Vue app (which falls through to the Vite branch).
        let dir = tempfile::tempdir().unwrap();
        write_js_project(
            dir.path(),
            r#"{ "name": "app", "devDependencies": { "@vue/cli-service": "5" } }"#,
            None,
        );
        let detected = detect_kind(dir.path());
        assert_eq!(detected.kind, ProjectType::VueCli);
        assert_eq!(detected.port, Some(8080));
        assert_eq!(
            detected.start_command.as_deref(),
            Some("pnpm exec vue-cli-service serve")
        );
    }

    #[test]
    fn detect_kind_vite_vue_app_stays_vite() {
        // A Vue app on Vite has no `@vue/cli-service`, so it must remain Vite.
        let dir = tempfile::tempdir().unwrap();
        write_js_project(
            dir.path(),
            r#"{ "name": "app", "dependencies": { "vue": "3" }, "devDependencies": { "vite": "5" } }"#,
            None,
        );
        let detected = detect_kind(dir.path());
        assert_eq!(detected.kind, ProjectType::Vite);
    }

    #[test]
    fn detect_kind_plain_node_app_still_node() {
        // No framework marker — the generic Node fallback must be unchanged.
        let dir = tempfile::tempdir().unwrap();
        write_js_project(
            dir.path(),
            r#"{ "name": "api", "dependencies": { "express": "4" } }"#,
            None,
        );
        let detected = detect_kind(dir.path());
        assert_eq!(detected.kind, ProjectType::Node);
        assert_eq!(detected.port, Some(3000));
    }

    fn write_file(dir: &Path, name: &str, body: &str) {
        let target = dir.join(name);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(target, body).unwrap();
    }

    #[test]
    fn detect_kind_laravel_uses_public_root_and_logo() {
        let dir = tempfile::tempdir().unwrap();
        write_file(
            dir.path(),
            "composer.json",
            r#"{ "require": { "laravel/framework": "^11" } }"#,
        );
        std::fs::create_dir_all(dir.path().join("public")).unwrap();
        let detected = detect_kind(dir.path());
        assert_eq!(detected.kind, ProjectType::Php);
        assert_eq!(detected.framework, Some(Framework::Laravel));
        assert_eq!(detected.document_root.as_deref(), Some("public"));
    }

    #[test]
    fn detect_kind_statamic_beats_laravel() {
        // Statamic ships on Laravel, so `laravel/framework` is also present —
        // the more specific CMS must win.
        let dir = tempfile::tempdir().unwrap();
        write_file(
            dir.path(),
            "composer.json",
            r#"{ "require": { "laravel/framework": "^11", "statamic/cms": "^5" } }"#,
        );
        let detected = detect_kind(dir.path());
        assert_eq!(detected.framework, Some(Framework::Statamic));
    }

    #[test]
    fn detect_kind_drupal_uses_web_root() {
        let dir = tempfile::tempdir().unwrap();
        write_file(
            dir.path(),
            "composer.json",
            r#"{ "require": { "drupal/core": "^10" } }"#,
        );
        std::fs::create_dir_all(dir.path().join("web")).unwrap();
        let detected = detect_kind(dir.path());
        assert_eq!(detected.framework, Some(Framework::Drupal));
        assert_eq!(detected.document_root.as_deref(), Some("web"));
    }

    #[test]
    fn detect_kind_wordpress_without_composer() {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "wp-config.php", "<?php");
        write_file(dir.path(), "index.php", "<?php");
        let detected = detect_kind(dir.path());
        assert_eq!(detected.kind, ProjectType::Php);
        assert_eq!(detected.framework, Some(Framework::WordPress));
    }

    #[test]
    fn detect_kind_django_tagged_framework() {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "manage.py", "");
        let detected = detect_kind(dir.path());
        assert_eq!(detected.kind, ProjectType::Python);
        assert_eq!(detected.framework, Some(Framework::Django));
    }

    #[test]
    fn detect_kind_streamlit_app() {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "requirements.txt", "streamlit==1.40\n");
        let detected = detect_kind(dir.path());
        assert_eq!(detected.kind, ProjectType::Python);
        assert_eq!(detected.framework, Some(Framework::Streamlit));
        assert_eq!(detected.port, Some(8501));
    }

    #[test]
    fn detect_kind_rails_project() {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "Gemfile", "gem \"rails\", \"~> 7.1\"\n");
        let detected = detect_kind(dir.path());
        assert_eq!(detected.kind, ProjectType::Ruby);
        assert_eq!(detected.framework, Some(Framework::Rails));
        assert_eq!(detected.port, Some(3000));
        assert_eq!(
            detected.start_command.as_deref(),
            Some("bin/rails server -p 3000")
        );
    }

    #[test]
    fn detect_kind_go_gin_project() {
        let dir = tempfile::tempdir().unwrap();
        write_file(
            dir.path(),
            "go.mod",
            "module app\n\nrequire github.com/gin-gonic/gin v1.10.0\n",
        );
        let detected = detect_kind(dir.path());
        assert_eq!(detected.kind, ProjectType::Go);
        assert_eq!(detected.framework, Some(Framework::Gin));
        assert_eq!(detected.start_command.as_deref(), Some("go run ."));
    }

    #[test]
    fn detect_kind_rust_axum_falls_back_to_rust_logo() {
        // Axum has no brand mark, so the framework tag is set but the frontend
        // renders the Rust language glyph. Detection still records the kind.
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "Cargo.toml", "[dependencies]\naxum = \"0.7\"\n");
        let detected = detect_kind(dir.path());
        assert_eq!(detected.kind, ProjectType::Rust);
        assert_eq!(detected.framework, Some(Framework::Axum));
    }

    #[test]
    fn detect_kind_hugo_before_go_mod() {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "hugo.toml", "title = 'Blog'\n");
        write_file(dir.path(), "go.mod", "module blog\n");
        let detected = detect_kind(dir.path());
        assert_eq!(detected.kind, ProjectType::Go);
        assert_eq!(detected.framework, Some(Framework::Hugo));
        assert_eq!(detected.port, Some(1313));
    }

    #[test]
    fn detect_kind_phoenix_project() {
        let dir = tempfile::tempdir().unwrap();
        write_file(
            dir.path(),
            "mix.exs",
            "defp deps do\n[{:phoenix, \"~> 1.7\"}]\nend\n",
        );
        let detected = detect_kind(dir.path());
        assert_eq!(detected.kind, ProjectType::Elixir);
        assert_eq!(detected.framework, Some(Framework::Phoenix));
        assert_eq!(detected.port, Some(4000));
    }

    #[test]
    fn detect_kind_aspnet_web_project() {
        let dir = tempfile::tempdir().unwrap();
        write_file(
            dir.path(),
            "App.csproj",
            "<Project Sdk=\"Microsoft.NET.Sdk.Web\"></Project>",
        );
        let detected = detect_kind(dir.path());
        assert_eq!(detected.kind, ProjectType::DotNet);
        assert_eq!(detected.framework, Some(Framework::AspNet));
    }

    #[test]
    fn detect_kind_spring_maven_project() {
        let dir = tempfile::tempdir().unwrap();
        write_file(
            dir.path(),
            "pom.xml",
            "<project><parent><groupId>org.springframework.boot</groupId></parent></project>",
        );
        let detected = detect_kind(dir.path());
        assert_eq!(detected.kind, ProjectType::Java);
        assert_eq!(detected.framework, Some(Framework::Spring));
        assert_eq!(detected.port, Some(8080));
        assert_eq!(
            detected.start_command.as_deref(),
            Some("./mvnw spring-boot:run")
        );
    }

    #[test]
    fn detect_kind_android_gradle_beats_jvm() {
        // An Android project also has a build.gradle; the mobile detector must
        // win over the generic JVM detector. `is_android_project` keys on the
        // Gradle wrapper + settings + app build file.
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "gradlew", "#!/bin/sh\n");
        write_file(dir.path(), "settings.gradle", "");
        write_file(dir.path(), "build.gradle", "");
        write_file(dir.path(), "app/build.gradle", "");
        let detected = detect_kind(dir.path());
        assert_eq!(detected.kind, ProjectType::Android);
    }

    #[test]
    fn framework_wire_values_match_frontend_contract() {
        // The frontend `Framework` union (src/lib/types/projects.ts) hard-codes
        // these snake_case strings. If serde's output drifts, the logo lookup
        // silently breaks — so pin the contract here.
        let cases = [
            (Framework::WordPress, "\"word_press\""),
            (Framework::CraftCms, "\"craft_cms\""),
            (Framework::CodeIgniter, "\"code_igniter\""),
            (Framework::CakePhp, "\"cake_php\""),
            (Framework::FastApi, "\"fast_api\""),
            (Framework::AspNet, "\"asp_net\""),
            (Framework::Laravel, "\"laravel\""),
            (Framework::SolidJs, "\"solid_js\""),
            (Framework::ReactRouter, "\"react_router\""),
            (Framework::React, "\"react\""),
        ];
        for (fw, wire) in cases {
            assert_eq!(serde_json::to_string(&fw).unwrap(), wire);
        }
        // ProjectType wire values the frontend also hard-codes. `OCaml` is the
        // tricky one — serde inserts `_` before each non-leading uppercase.
        assert_eq!(
            serde_json::to_string(&ProjectType::DotNet).unwrap(),
            "\"dot_net\""
        );
        assert_eq!(
            serde_json::to_string(&ProjectType::OCaml).unwrap(),
            "\"o_caml\""
        );
    }

    #[test]
    fn detect_kind_vite_react_tagged_react() {
        let dir = tempfile::tempdir().unwrap();
        write_js_project(
            dir.path(),
            r#"{ "dependencies": { "react": "18" }, "devDependencies": { "vite": "5" } }"#,
            None,
        );
        let detected = detect_kind(dir.path());
        assert_eq!(detected.kind, ProjectType::Vite);
        assert_eq!(detected.framework, Some(Framework::React));
    }

    #[test]
    fn detect_kind_vite_vue_tagged_vue() {
        let dir = tempfile::tempdir().unwrap();
        write_js_project(
            dir.path(),
            r#"{ "dependencies": { "vue": "3" }, "devDependencies": { "vite": "5" } }"#,
            None,
        );
        let detected = detect_kind(dir.path());
        assert_eq!(detected.kind, ProjectType::Vite);
        assert_eq!(detected.framework, Some(Framework::Vue));
    }

    #[test]
    fn detect_kind_next_has_no_ui_lib_framework() {
        // A specific kind (Next) keeps its own logo — the UI-lib descriptor is
        // only attached to the generic Vite/Node kinds.
        let dir = tempfile::tempdir().unwrap();
        write_js_project(
            dir.path(),
            r#"{ "dependencies": { "next": "14", "react": "18" } }"#,
            None,
        );
        let detected = detect_kind(dir.path());
        assert_eq!(detected.kind, ProjectType::Next);
        assert_eq!(detected.framework, None);
    }

    #[test]
    fn detect_kind_react_router_7() {
        let dir = tempfile::tempdir().unwrap();
        write_js_project(
            dir.path(),
            r#"{ "dependencies": { "react-router": "7" }, "devDependencies": { "@react-router/dev": "7", "vite": "6" } }"#,
            None,
        );
        let detected = detect_kind(dir.path());
        assert_eq!(detected.kind, ProjectType::Vite);
        assert_eq!(detected.framework, Some(Framework::ReactRouter));
    }

    #[test]
    fn detect_kind_eleventy_project() {
        let dir = tempfile::tempdir().unwrap();
        write_js_project(
            dir.path(),
            r#"{ "devDependencies": { "@11ty/eleventy": "3" } }"#,
            None,
        );
        let detected = detect_kind(dir.path());
        assert_eq!(detected.kind, ProjectType::Node);
        assert_eq!(detected.framework, Some(Framework::Eleventy));
        assert_eq!(detected.port, Some(8080));
    }

    #[test]
    fn detect_kind_bedrock_wordpress_web_root() {
        let dir = tempfile::tempdir().unwrap();
        write_file(
            dir.path(),
            "composer.json",
            r#"{ "require": { "roots/wordpress": "^6" } }"#,
        );
        std::fs::create_dir_all(dir.path().join("web")).unwrap();
        let detected = detect_kind(dir.path());
        assert_eq!(detected.framework, Some(Framework::WordPress));
        assert_eq!(detected.document_root.as_deref(), Some("web"));
    }

    #[test]
    fn detect_kind_scala_sbt_project() {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "build.sbt", "name := \"app\"\n");
        let detected = detect_kind(dir.path());
        assert_eq!(detected.kind, ProjectType::Scala);
        assert_eq!(detected.start_command.as_deref(), Some("sbt run"));
    }

    #[test]
    fn detect_kind_dart_after_flutter() {
        // A bare pubspec.yaml (no Flutter dep) is plain Dart.
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "pubspec.yaml", "name: cli_app\n");
        let detected = detect_kind(dir.path());
        assert_eq!(detected.kind, ProjectType::Dart);
    }

    #[test]
    fn detect_kind_swift_vapor_project() {
        let dir = tempfile::tempdir().unwrap();
        write_file(
            dir.path(),
            "Package.swift",
            ".package(url: \"https://github.com/vapor/vapor.git\", from: \"4.0.0\")",
        );
        let detected = detect_kind(dir.path());
        assert_eq!(detected.kind, ProjectType::Swift);
        assert_eq!(detected.framework, Some(Framework::Vapor));
        assert_eq!(detected.port, Some(8080));
    }

    #[test]
    fn detect_kind_haskell_stack_project() {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "stack.yaml", "resolver: lts-22\n");
        let detected = detect_kind(dir.path());
        assert_eq!(detected.kind, ProjectType::Haskell);
        assert_eq!(detected.start_command.as_deref(), Some("stack run"));
    }

    #[test]
    fn detect_kind_zig_project() {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "build.zig", "pub fn build() void {}\n");
        let detected = detect_kind(dir.path());
        assert_eq!(detected.kind, ProjectType::Zig);
    }

    #[test]
    fn detect_kind_flutter_project_has_process_command_and_no_port() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("pubspec.yaml"), "name: app\n").unwrap();
        std::fs::create_dir_all(dir.path().join("lib")).unwrap();
        std::fs::write(dir.path().join("lib").join("main.dart"), "void main() {}\n").unwrap();

        let detected = detect_kind(dir.path());
        assert_eq!(detected.kind, ProjectType::Flutter);
        assert_eq!(detected.port, None);
        assert_eq!(detected.mobile_run, Some(MobileRunConfig::default()));
        // Launch attaches to the running app (hot-reload host), so it stays a
        // long-running PC process; the pid file enables hot reload/restart
        // signals. See `crate::mobile`.
        let cmd = detected
            .start_command
            .expect("flutter projects get a command");
        assert!(cmd.ends_with("exec flutter run --pid-file .portbay-flutter.pid"));
    }

    #[test]
    fn detect_kind_xcode_project_builds_and_launches_simulator() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(
            dir.path()
                .join("Mobile.xcodeproj")
                .join("xcshareddata")
                .join("xcschemes"),
        )
        .unwrap();
        std::fs::write(
            dir.path()
                .join("Mobile.xcodeproj")
                .join("xcshareddata")
                .join("xcschemes")
                .join("TribalHouse.xcscheme"),
            "",
        )
        .unwrap();

        let detected = detect_kind(dir.path());
        assert_eq!(detected.kind, ProjectType::Xcode);
        assert_eq!(detected.port, None);
        assert_eq!(
            detected
                .mobile_run
                .as_ref()
                .and_then(|m| m.target.as_deref()),
            Some("TribalHouse")
        );
        // The launcher boots a simulator, builds the detected scheme, installs,
        // and launches attached to the console (full launch, not a bare build).
        let cmd = detected.start_command.unwrap();
        assert!(cmd.contains("SCHEME='TribalHouse'"));
        assert!(cmd.contains("xcodebuild"));
        assert!(cmd.contains("simctl install"));
        assert!(cmd.contains("simctl launch --console-pty"));
    }

    #[test]
    fn detect_kind_android_project_installs_and_launches() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("gradlew"), "").unwrap();
        std::fs::write(dir.path().join("settings.gradle.kts"), "").unwrap();
        std::fs::create_dir_all(dir.path().join("app")).unwrap();
        std::fs::write(dir.path().join("app").join("build.gradle.kts"), "").unwrap();

        let detected = detect_kind(dir.path());
        assert_eq!(detected.kind, ProjectType::Android);
        assert_eq!(detected.port, None);
        let mobile = detected.mobile_run.as_ref().unwrap();
        assert_eq!(mobile.target.as_deref(), Some("app"));
        assert_eq!(mobile.flavor.as_deref(), Some("debug"));
        // Launcher installs the debug build then launches + tails logcat. The
        // "debug" flavor is the build type, so the task is `installDebug` (not
        // doubled into `installDebugDebug`).
        let cmd = detected.start_command.unwrap();
        assert!(cmd.contains(":app:installDebug"));
        assert!(!cmd.contains("installDebugDebug"));
        assert!(cmd.contains("adb -s \"$SER\" logcat"));
    }

    #[test]
    fn detect_kind_django_project_runs_manage_py() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("manage.py"), "# django\n").unwrap();
        std::fs::write(dir.path().join("requirements.txt"), "Django>=5.0\n").unwrap();

        let detected = detect_kind(dir.path());
        assert_eq!(detected.kind, ProjectType::Python);
        assert_eq!(detected.port, Some(8000));
        assert_eq!(
            detected.start_command.as_deref(),
            Some("python manage.py runserver 0.0.0.0:8000")
        );
    }

    #[test]
    fn detect_kind_fastapi_project_runs_uvicorn() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("requirements.txt"), "fastapi\nuvicorn\n").unwrap();

        let detected = detect_kind(dir.path());
        assert_eq!(detected.kind, ProjectType::Python);
        assert_eq!(detected.port, Some(8000));
        assert!(detected
            .start_command
            .as_deref()
            .unwrap()
            .contains("uvicorn"));
    }

    #[test]
    fn detect_kind_bare_python_project_has_no_server() {
        // A research / LLM-eval / library project: a manifest but no web
        // framework. It gets typed as Python (so the runtime + venv flow apply)
        // but with no port and no start command — a board/process-only project.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("pyproject.toml"),
            "[project]\nname = \"evals\"\ndependencies = [\"numpy\", \"pytest\"]\n",
        )
        .unwrap();

        let detected = detect_kind(dir.path());
        assert_eq!(detected.kind, ProjectType::Python);
        assert_eq!(detected.port, None);
        assert_eq!(detected.start_command, None);
    }

    #[test]
    fn detect_kind_js_wins_over_python_when_both_present() {
        // A Python backend with a JS frontend at the root resolves to the JS
        // type today (package.json is checked first). Documented, not ideal —
        // splitting such a repo is a separate concern.
        let dir = tempfile::tempdir().unwrap();
        write_js_project(dir.path(), r#"{ "name": "app", "dependencies": {} }"#, None);
        std::fs::write(dir.path().join("requirements.txt"), "flask\n").unwrap();

        let detected = detect_kind(dir.path());
        assert_eq!(detected.kind, ProjectType::Node);
    }

    #[test]
    fn detect_kind_package_manager_field_beats_lockfile() {
        let dir = tempfile::tempdir().unwrap();
        // npm lockfile present, but the field declares yarn — the field wins.
        write_js_project(
            dir.path(),
            r#"{ "name": "app", "packageManager": "yarn@4.1.0", "dependencies": {} }"#,
            Some("package-lock.json"),
        );
        let detected = detect_kind(dir.path());
        assert_eq!(detected.kind, ProjectType::Node);
        assert_eq!(detected.start_command.as_deref(), Some("yarn dev"));
    }

    #[test]
    fn detect_kind_defaults_to_pnpm_when_no_signal() {
        let dir = tempfile::tempdir().unwrap();
        write_js_project(dir.path(), r#"{ "name": "app" }"#, None);
        let detected = detect_kind(dir.path());
        assert_eq!(detected.start_command.as_deref(), Some("pnpm dev"));
    }

    #[test]
    fn language_intelligence_detects_mixed_web_project_files() {
        let dir = tempfile::tempdir().unwrap();
        write_js_project(
            dir.path(),
            r#"{ "name": "app", "dependencies": { "vite": "5" } }"#,
            None,
        );
        std::fs::write(dir.path().join(".env"), "PORT=5173\n").unwrap();
        std::fs::write(dir.path().join("schema.sql"), "select 1;\n").unwrap();

        let caps = detect_language_intelligence(dir.path(), ProjectType::Vite);
        let langs: Vec<_> = caps.iter().map(|c| c.language.as_str()).collect();
        assert!(langs.contains(&"javascript_typescript"));
        assert!(langs.contains(&"config"));
        assert!(langs.contains(&"sql"));
        assert!(caps
            .iter()
            .all(|c| c.features.contains(&"diagnostics".to_string())));
    }

    #[test]
    fn language_intelligence_uses_project_kind_when_marker_is_sparse() {
        let dir = tempfile::tempdir().unwrap();
        let caps = detect_language_intelligence(dir.path(), ProjectType::Next);
        let js = caps
            .iter()
            .find(|c| c.language == "javascript_typescript")
            .unwrap();
        assert_eq!(js.files, vec!["package.json"]);
    }
}
