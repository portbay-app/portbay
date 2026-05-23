//! Tauri commands for `.portbay.json` export + import.
//!
//! Four surfaces:
//!
//! - `export_portfile(id)` — writes `<project_path>/.portbay.json`
//!   from the given project's registry entry.
//! - `detect_portfile(path)` — reads `<path>/.portbay.json` if present
//!   and returns the parsed file. The Add Project wizard uses this to
//!   pre-fill L2 fields when the user drops in a folder that already
//!   has a `.portbay.json` committed.
//! - `import_portfile_preview(path)` — parses the file at the given
//!   project path and returns the import plan (file body + required
//!   secrets list).
//! - `import_portfile_commit(path, secrets)` — materialises the
//!   final Project, adds it to the registry, marks the reconciler
//!   dirty, and returns the new project's id.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::commands::projects::{load_registry, save_registry};
use crate::error::{AppError, AppResult};
use crate::portfile::{self, PortbayFile, PORTBAY_FILE_NAME};
use crate::registry::ProjectId;
use crate::state::AppState;

/// What `import_portfile_preview` returns. `requiredSecrets` is the
/// list of env-var names the GUI must prompt the user for.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub file: PortbayFile,
    pub project_path: String,
    pub required_secrets: Vec<String>,
    /// True iff a project with the file's suggested id (derived from
    /// the directory name) already exists in the registry. The GUI
    /// surfaces a "Rename / Skip" prompt when so.
    pub id_collision: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportCommitInput {
    pub path: String,
    pub id: Option<String>,
    #[serde(default)]
    pub secrets: BTreeMap<String, String>,
}

#[tauri::command]
pub async fn export_portfile(state: State<'_, AppState>, id: String) -> AppResult<String> {
    let registry = load_registry(&state)?;
    let project = registry
        .get_project(&ProjectId::new(id.clone()))
        .ok_or_else(|| AppError::NotFound(id.clone()))?
        .clone();

    let file = portfile::export_project(&project);
    let json =
        portfile::to_json_string(&file).map_err(|e| AppError::Internal(format!("export: {e}")))?;

    let path = project.path.join(PORTBAY_FILE_NAME);
    std::fs::write(&path, json)
        .map_err(|e| AppError::Internal(format!("write {}: {e}", path.display())))?;

    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
pub async fn detect_portfile(path: String) -> AppResult<Option<PortbayFile>> {
    let candidate = PathBuf::from(&path).join(PORTBAY_FILE_NAME);
    if !candidate.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(&candidate)
        .map_err(|e| AppError::Internal(format!("read {}: {e}", candidate.display())))?;
    let file =
        portfile::from_json_bytes(&bytes).map_err(|e| AppError::Internal(format!("parse: {e}")))?;
    Ok(Some(file))
}

#[tauri::command]
pub async fn import_portfile_preview(
    state: State<'_, AppState>,
    path: String,
) -> AppResult<ImportPreview> {
    let project_path = PathBuf::from(&path);
    let file_path = project_path.join(PORTBAY_FILE_NAME);
    let bytes = std::fs::read(&file_path)
        .map_err(|e| AppError::BadInput(format!("read {}: {e}", file_path.display())))?;
    let file =
        portfile::from_json_bytes(&bytes).map_err(|e| AppError::Internal(format!("parse: {e}")))?;

    let suggested_id = derive_id_from_path(&project_path);
    let registry = load_registry(&state)?;
    let id_collision = registry
        .get_project(&ProjectId::new(suggested_id.clone()))
        .is_some();

    Ok(ImportPreview {
        required_secrets: file.secrets.clone(),
        file,
        project_path: project_path.to_string_lossy().into_owned(),
        id_collision,
    })
}

#[tauri::command]
pub async fn import_portfile_commit(
    state: State<'_, AppState>,
    input: ImportCommitInput,
) -> AppResult<String> {
    let project_path = PathBuf::from(&input.path);
    let file_path = project_path.join(PORTBAY_FILE_NAME);
    let bytes = std::fs::read(&file_path)
        .map_err(|e| AppError::BadInput(format!("read {}: {e}", file_path.display())))?;
    let file =
        portfile::from_json_bytes(&bytes).map_err(|e| AppError::Internal(format!("parse: {e}")))?;

    let id_str = input
        .id
        .unwrap_or_else(|| derive_id_from_path(&project_path));
    let id = ProjectId::new(&id_str);

    let plan = portfile::ImportPlan::new(file, project_path);
    let project = portfile::materialise_project(&plan, id, &input.secrets)
        .map_err(|e| AppError::Internal(format!("materialise: {e}")))?;

    let mut registry = load_registry(&state)?;
    registry.add_project(project)?;
    save_registry(&state, &registry)?;
    state.reconciler.mark_dirty();

    Ok(id_str)
}

fn derive_id_from_path(path: &std::path::Path) -> String {
    let last = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("imported");
    let mut out = String::with_capacity(last.len());
    let mut last_dash = true;
    for ch in last.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "imported".to_string()
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_id_lowercases_and_hyphenates() {
        assert_eq!(
            derive_id_from_path(std::path::Path::new("/Users/x/API Gateway")),
            "api-gateway"
        );
        assert_eq!(
            derive_id_from_path(std::path::Path::new("/Users/x/myapp")),
            "myapp"
        );
    }
}
