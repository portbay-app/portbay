//! Project CRUD commands.
//!
//! Thin wrappers around the registry CRUD already shipped in P1. The
//! frontend never touches `registry::Registry` directly — every read or
//! write goes through these commands so we can layer in side effects
//! (Caddy reconcile, hosts file write, cert issuance) in one place later.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use tauri::State;

use crate::commands::dto::{
    AddProjectInput, DetectedProject, ProjectView, UpdateProjectPatch, WorkspaceAppDto,
    WorkspaceScan,
};
use crate::error::{AppError, AppResult};
use crate::process_compose::Process;
use crate::registry::{store, Project, ProjectId, ProjectType, Readiness, Registry};
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
            let proc = pc_state.as_ref().and_then(|m| m.get(p.id.as_str()));
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
    let proc = pc_state.as_ref().and_then(|m| m.get(id.as_str()));
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

    let hostname = input
        .hostname
        .unwrap_or_else(|| format!("{}.{}", id_str, registry.domain_suffix));

    let readiness = input.port.map(|_| Readiness::Http {
        path: "/".into(),
        timeout_seconds: 75,
    });

    // Inherit the language's default runtime version (set in the Languages
    // panel) when this project doesn't pin one itself. For PHP we mirror it
    // into `php_version` too, since the FPM reconciler still reads that field.
    let runtime = registry.runtimes.default_for(input.kind);
    let php_version = if input.kind == ProjectType::Php {
        runtime.as_ref().map(|r| r.version.clone())
    } else {
        None
    };

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
        services: if input.https {
            vec!["caddy".into()]
        } else {
            vec![]
        },
        env: Default::default(),
        readiness,
        auto_start: input.auto_start,
        tags: vec![],
        document_root: None,
        php_version,
        runtime,
        workspace: input.workspace,
    };

    registry.add_project(project.clone())?;
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
        project.start_command = Some(cmd);
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
    if let Some(ws) = patch.workspace {
        project.workspace = Some(ws);
    }

    let snapshot = project.clone();
    save_registry(&state, &registry)?;
    state.reconciler.mark_dirty();

    // Look up live runtime after save.
    let pc_state = fetch_pc_state(&state).await;
    let proc = pc_state.as_ref().and_then(|m| m.get(id.as_str()));
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

    let (kind, suggested_port, suggested_start_command) = detect_kind(&p);

    Ok(DetectedProject {
        kind,
        suggested_id: id,
        suggested_name: dir_name,
        suggested_hostname,
        suggested_port,
        suggested_start_command,
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
            let (kind, port, detected_cmd) = detect_kind(&pkg.abs_dir);
            // Honour the repo's package manager rather than detect_kind's
            // hardcoded `pnpm dev`, but only for an app that has a dev command.
            let start_command = detected_cmd.map(|_| standalone_dev_command(layout.tool));
            WorkspaceAppDto {
                package: pkg.name.clone(),
                rel_dir: pkg.rel_dir.clone(),
                path: pkg.abs_dir.display().to_string(),
                kind,
                suggested_hostname: format!("{id}.{suffix}"),
                suggested_id: id,
                suggested_name: leaf.to_string(),
                suggested_port: port,
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

fn detect_kind(path: &Path) -> (ProjectType, u16, Option<String>) {
    let pkg = path.join("package.json");
    if pkg.exists() {
        let body = std::fs::read_to_string(&pkg).unwrap_or_default();
        // The run command follows the project's actual package manager
        // (`packageManager` field → lockfile → pnpm default) so the first Play
        // matches the lockfile instead of always guessing `pnpm dev`.
        let cmd = standalone_dev_command(crate::registry::workspace::detect_package_manager(path));
        // Cheap string match — full JSON parse isn't worth the cycles.
        if body.contains("\"next\"") {
            return (ProjectType::Next, 3000, Some(cmd));
        }
        if body.contains("\"vite\"") {
            return (ProjectType::Vite, 5173, Some(cmd));
        }
        return (ProjectType::Node, 3000, Some(cmd));
    }

    if path.join("composer.json").exists() || has_php_index(path) {
        return (ProjectType::Php, 8000, None);
    }

    if path.join("index.html").exists() {
        return (ProjectType::Static, 8000, None);
    }

    (ProjectType::Custom, 3000, None)
}

fn has_php_index(path: &Path) -> bool {
    path.join("index.php").exists() || path.join("public").join("index.php").exists()
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
        let (kind, port, cmd) = detect_kind(dir.path());
        assert_eq!(kind, ProjectType::Next);
        assert_eq!(port, 3000);
        assert_eq!(cmd.as_deref(), Some("bun run dev"));
    }

    #[test]
    fn detect_kind_npm_lockfile_yields_npm_run_dev() {
        let dir = tempfile::tempdir().unwrap();
        write_js_project(
            dir.path(),
            r#"{ "name": "app", "devDependencies": { "vite": "5" } }"#,
            Some("package-lock.json"),
        );
        let (kind, _port, cmd) = detect_kind(dir.path());
        assert_eq!(kind, ProjectType::Vite);
        assert_eq!(cmd.as_deref(), Some("npm run dev"));
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
        let (kind, _port, cmd) = detect_kind(dir.path());
        assert_eq!(kind, ProjectType::Node);
        assert_eq!(cmd.as_deref(), Some("yarn dev"));
    }

    #[test]
    fn detect_kind_defaults_to_pnpm_when_no_signal() {
        let dir = tempfile::tempdir().unwrap();
        write_js_project(dir.path(), r#"{ "name": "app" }"#, None);
        let (_kind, _port, cmd) = detect_kind(dir.path());
        assert_eq!(cmd.as_deref(), Some("pnpm dev"));
    }
}
