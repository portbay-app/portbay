//! Project CRUD commands.
//!
//! Thin wrappers around the registry CRUD already shipped in P1. The
//! frontend never touches `registry::Registry` directly — every read or
//! write goes through these commands so we can layer in side effects
//! (Caddy reconcile, hosts file write, cert issuance) in one place later.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::path::PathBuf;

use tauri::State;

use crate::commands::dto::{AddProjectInput, ProjectView, UpdateProjectPatch};
use crate::error::{AppError, AppResult};
use crate::hosts::HostsManager;
use crate::process_compose::Process;
use crate::registry::{store, Project, ProjectId, Readiness, Registry};
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
pub async fn get_project(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<ProjectView> {
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
        php_version: None,
    };

    registry.add_project(project.clone())?;
    save_registry(&state, &registry)?;

    // Best-effort hosts entry. The CLI does the same — see `cmd_add`.
    let _ = HostsManager::system().add(&hostname, Ipv4Addr::LOCALHOST);

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

    // Look up live runtime after save.
    let pc_state = fetch_pc_state(&state).await;
    let proc = pc_state.as_ref().and_then(|m| m.get(id.as_str()));
    Ok(ProjectView::from_project(&snapshot, proc))
}

/// `remove_project(id)` — full cleanup. Registry → cert dir → hosts entry.
#[tauri::command]
pub async fn remove_project(state: State<'_, AppState>, id: String) -> AppResult<()> {
    let mut registry = load_registry(&state)?;
    let pid = ProjectId::new(id.clone());
    let removed = registry.remove_project(&pid)?;
    save_registry(&state, &registry)?;

    // Best-effort cert dir cleanup — match the CLI's `cmd_remove`.
    if let Some(mut certs_root) = dirs::data_dir() {
        certs_root.push("PortBay");
        certs_root.push("certs");
        let dir = certs_root.join(removed.id.as_str());
        if dir.exists() {
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    // Best-effort hosts entry removal.
    let _ = HostsManager::system().remove(&removed.hostname);

    Ok(())
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

/// Lowercase + hyphenate + collapse runs of non-alphanumerics into a single
/// dash. Lifted from `bin/portbay.rs::slugify` — both surfaces produce the
/// same ids from the same inputs.
pub(crate) fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_dash = true;
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_matches_cli_behaviour() {
        assert_eq!(slugify("Nour Beiruti"), "nour-beiruti");
        assert_eq!(slugify("Tribal House CMS"), "tribal-house-cms");
        assert_eq!(slugify("__weird___name__"), "weird-name");
        assert_eq!(slugify("UPPER"), "upper");
    }
}
