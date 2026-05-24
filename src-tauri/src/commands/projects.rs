//! Project CRUD commands.
//!
//! Thin wrappers around the registry CRUD already shipped in P1. The
//! frontend never touches `registry::Registry` directly — every read or
//! write goes through these commands so we can layer in side effects
//! (Caddy reconcile, hosts file write, cert issuance) in one place later.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use tauri::State;

use crate::commands::dto::{AddProjectInput, DetectedProject, ProjectView, UpdateProjectPatch};
use crate::error::{AppError, AppResult};
use crate::process_compose::Process;
use crate::registry::{store, Project, ProjectId, ProjectType, Readiness, Registry, Runtime};
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
    let runtime = default_runtime_for(input.kind, &registry.runtimes.defaults);
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
        // Cheap string match — full JSON parse isn't worth the cycles.
        if body.contains("\"next\"") {
            return (ProjectType::Next, 3000, Some("pnpm dev".into()));
        }
        if body.contains("\"vite\"") {
            return (ProjectType::Vite, 5173, Some("pnpm dev".into()));
        }
        return (ProjectType::Node, 3000, Some("pnpm dev".into()));
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

/// Resolve the default runtime a new project of `kind` should inherit from
/// the per-language defaults. Static/Custom projects have no managed runtime.
/// Returns `None` when no default is set for the mapped language.
fn default_runtime_for(kind: ProjectType, defaults: &BTreeMap<String, String>) -> Option<Runtime> {
    let lang = match kind {
        ProjectType::Next | ProjectType::Vite | ProjectType::Node => "node",
        ProjectType::Php => "php",
        ProjectType::Static | ProjectType::Custom => return None,
    };
    defaults.get(lang).map(|version| Runtime {
        lang: lang.to_string(),
        version: version.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_runtime_inherited_per_language() {
        let mut defaults = BTreeMap::new();
        defaults.insert("node".to_string(), "22".to_string());
        defaults.insert("php".to_string(), "8.3".to_string());

        assert_eq!(
            default_runtime_for(ProjectType::Next, &defaults),
            Some(Runtime {
                lang: "node".into(),
                version: "22".into()
            })
        );
        assert_eq!(
            default_runtime_for(ProjectType::Php, &defaults),
            Some(Runtime {
                lang: "php".into(),
                version: "8.3".into()
            })
        );
        // Static/Custom have no managed runtime.
        assert_eq!(default_runtime_for(ProjectType::Static, &defaults), None);
    }

    #[test]
    fn no_default_set_yields_no_runtime() {
        let defaults = BTreeMap::new();
        assert_eq!(default_runtime_for(ProjectType::Next, &defaults), None);
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
}
