//! Project CRUD commands.
//!
//! Thin wrappers around the registry CRUD already shipped in P1. The
//! frontend never touches `registry::Registry` directly — every read or
//! write goes through these commands so we can layer in side effects
//! (Caddy reconcile, hosts file write, cert issuance) in one place later.

use std::collections::{BTreeMap, HashMap};
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use tauri::{AppHandle, State};

use crate::commands::dto::{
    AddProjectInput, DetectedProject, ProjectView, UpdateProjectPatch, WorkspaceAppDto,
    WorkspaceScan,
};
use crate::error::{AppError, AppResult};
use crate::process_compose::Process;
use crate::registry::{
    store, MobileRunConfig, Project, ProjectId, ProjectType, Readiness, Registry, Runtime,
    SandboxConfig, SandboxNetworkPolicy, WebServer,
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

    let views = registry
        .list_projects()
        .iter()
        .map(|p| {
            let proc = pc_state
                .as_ref()
                .and_then(|m| p.process_compose_id().and_then(|key| m.get(key.as_str())));
            ProjectView::from_project(p, proc)
        })
        .collect();
    Ok(views)
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
    Ok(ProjectView::from_project(project, proc))
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
    let project = Project {
        id,
        name,
        path,
        kind: input.kind,
        start_command: input.start_command,
        port: input.port,
        extra_ports: vec![],
        hostname: hostname.clone(),
        https: input.https,
        services: default_services(input.kind, input.https, has_start_command),
        env: Default::default(),
        readiness,
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

    // Hand off side-effects (hosts, certs, Caddy routes, PC YAML) to
    // the reconciler. The tick runs in the background; the user's
    // toast returns immediately.
    state.reconciler.mark_dirty();

    Ok(ProjectView::from_project(&project, None))
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
    if let Some(hostname) = patch.hostname {
        project.hostname = hostname;
    }
    if let Some(port) = patch.port {
        project.port = Some(port);
    }
    if let Some(extras) = patch.extra_ports {
        project.extra_ports = extras;
    }
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
    if let Some(tags) = patch.tags {
        project.tags = tags;
    }
    if let Some(services) = patch.services {
        project.services = services;
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
        let cmd = standalone_dev_command(crate::registry::workspace::detect_package_manager(path));
        // Cheap string match — full JSON parse isn't worth the cycles.
        if body.contains("\"next\"") {
            return detection(ProjectType::Next, 3000, Some(cmd));
        }
        if body.contains("\"vite\"") {
            return detection(ProjectType::Vite, 5173, Some(cmd));
        }
        return detection(ProjectType::Node, 3000, Some(cmd));
    }

    if path.join("composer.json").exists() || has_php_index(path) {
        return detect_php_project(path);
    }

    if is_flutter_project(path) {
        let mobile_run = detect_flutter_run(path);
        return detection_with_mobile(
            ProjectType::Flutter,
            mobile_start_command(ProjectType::Flutter, mobile_run.as_ref()),
            mobile_run,
        );
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

    if is_android_project(path) {
        let mobile_run = detect_android_run(path);
        return detection_with_mobile(
            ProjectType::Android,
            mobile_start_command(ProjectType::Android, mobile_run.as_ref()),
            mobile_run,
        );
    }

    if path.join("index.html").exists() {
        return detection(ProjectType::Static, 8000, None);
    }

    detection(ProjectType::Custom, 3000, None)
}

fn detection(kind: ProjectType, port: u16, start_command: Option<String>) -> ProjectDetection {
    ProjectDetection {
        kind,
        port: (port > 0).then_some(port),
        start_command,
        document_root: None,
        php_version: None,
        web_server: None,
        mobile_run: None,
    }
}

fn detection_with_mobile(
    kind: ProjectType,
    start_command: Option<String>,
    mobile_run: Option<MobileRunConfig>,
) -> ProjectDetection {
    ProjectDetection {
        kind,
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
    let public = path.join("public");
    let document_root = if public.join("index.php").exists() || public.join("router.php").exists() {
        Some("public".to_string())
    } else {
        None
    };

    ProjectDetection {
        kind: ProjectType::Php,
        port: Some(port),
        start_command: None,
        document_root,
        php_version: version,
        web_server: None,
        mobile_run: None,
    }
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

fn mobile_start_command(kind: ProjectType, cfg: Option<&MobileRunConfig>) -> Option<String> {
    match kind {
        ProjectType::Flutter => {
            let mut args = vec!["flutter".to_string(), "run".to_string()];
            if let Some(flavor) = cfg.and_then(|c| clean_optional(&c.flavor)) {
                args.push("--flavor".into());
                args.push(shell_quote(flavor));
            }
            if let Some(device) = cfg.and_then(|c| clean_optional(&c.device)) {
                args.push("-d".into());
                args.push(shell_quote(device));
            }
            Some(args.join(" "))
        }
        ProjectType::Xcode => {
            let scheme = cfg.and_then(|c| clean_optional(&c.target));
            let Some(scheme) = scheme else {
                return Some("xed .".into());
            };
            let mut args = vec![
                "xcodebuild".to_string(),
                "-scheme".into(),
                shell_quote(scheme),
            ];
            if let Some(destination) = cfg.and_then(|c| clean_optional(&c.device)) {
                args.push("-destination".into());
                args.push(shell_quote(destination));
            }
            args.push("build".into());
            Some(args.join(" "))
        }
        ProjectType::Android => {
            let module = cfg.and_then(|c| clean_optional(&c.target)).unwrap_or("app");
            let variant = cfg
                .and_then(|c| clean_optional(&c.flavor))
                .map(capitalize_ascii)
                .unwrap_or_else(|| "Debug".into());
            let command = format!("./gradlew :{}:install{}", module, variant);
            if let Some(device) = cfg.and_then(|c| clean_optional(&c.device)) {
                Some(format!("ANDROID_SERIAL={} {command}", shell_quote(device)))
            } else {
                Some(command)
            }
        }
        _ => None,
    }
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

fn clean_optional(value: &Option<String>) -> Option<&str> {
    value.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

fn capitalize_ascii(value: &str) -> String {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    format!("{}{}", first.to_ascii_uppercase(), chars.as_str())
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
        ProjectType::Flutter | ProjectType::Xcode | ProjectType::Android => vec![],
        ProjectType::Php if has_start_command => vec!["caddy".into()],
        ProjectType::Php => vec!["caddy".into(), "php-fpm".into()],
        _ if https => vec!["caddy".into()],
        _ => vec![],
    }
}

fn shell_quote(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | ':'))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
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
        let g = state.pc_client.lock().expect("pc_client mutex poisoned");
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
    fn detect_kind_flutter_project_has_process_command_and_no_port() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("pubspec.yaml"), "name: app\n").unwrap();
        std::fs::create_dir_all(dir.path().join("lib")).unwrap();
        std::fs::write(dir.path().join("lib").join("main.dart"), "void main() {}\n").unwrap();

        let detected = detect_kind(dir.path());
        assert_eq!(detected.kind, ProjectType::Flutter);
        assert_eq!(detected.port, None);
        assert_eq!(detected.mobile_run, Some(MobileRunConfig::default()));
        assert_eq!(detected.start_command.as_deref(), Some("flutter run"));
    }

    #[test]
    fn detect_kind_xcode_project_opens_workspace() {
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
        assert_eq!(
            detected.start_command.as_deref(),
            Some("xcodebuild -scheme TribalHouse build")
        );
    }

    #[test]
    fn detect_kind_android_project_installs_debug_build() {
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
        assert_eq!(
            detected.start_command.as_deref(),
            Some("./gradlew :app:installDebug")
        );
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
}
