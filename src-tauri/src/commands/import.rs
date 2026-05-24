//! Migration-import commands.
//!
//! Two surfaces:
//!
//! - `detect_sources()` lists which source tools (Herd, ServBay, MAMP)
//!   are installed locally and how many sites they expose.
//! - `import_projects(source, ids)` translates each chosen site into a
//!   PortBay `Project`, writes them to the registry, marks the
//!   reconciler dirty, and returns the rows that landed (plus any
//!   skipped with a reason).

use std::collections::HashMap;
use std::path::PathBuf;

use serde::Serialize;
use tauri::State;

use crate::commands::projects::{load_registry, save_registry};
use crate::error::{AppError, AppResult};
use crate::import::{self, DetectedSource, ImportSource, ImportedSite};
use crate::registry::{Project, ProjectId, ProjectType, Readiness, Runtime};
use crate::state::AppState;

/// One row in the import preview. Built from `ImportedSite` plus
/// per-row collision flags so the GUI can render check/cross marks.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreviewRow {
    pub site: ImportedSite,
    /// True if a project with the same id already exists in PortBay.
    pub id_collision: bool,
    /// True if a project with the same path already exists.
    pub path_collision: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub imported: Vec<String>,
    pub skipped: Vec<SkippedRow>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedRow {
    pub site: ImportedSite,
    pub reason: String,
}

#[tauri::command]
pub async fn detect_sources() -> AppResult<Vec<DetectedSource>> {
    Ok(import::detect_all())
}

#[tauri::command]
pub async fn preview_import(
    state: State<'_, AppState>,
    source: ImportSource,
) -> AppResult<Vec<ImportPreviewRow>> {
    let sites = import::read_all(source).map_err(|e| AppError::Internal(e.to_string()))?;
    let registry = load_registry(&state)?;
    let existing_ids: std::collections::HashSet<String> = registry
        .list_projects()
        .iter()
        .map(|p| p.id.as_str().to_string())
        .collect();
    let existing_paths: std::collections::HashSet<PathBuf> = registry
        .list_projects()
        .iter()
        .map(|p| p.path.clone())
        .collect();

    let rows = sites
        .into_iter()
        .map(|site| ImportPreviewRow {
            id_collision: existing_ids.contains(&site.suggested_id),
            path_collision: existing_paths.contains(&PathBuf::from(&site.path)),
            site,
        })
        .collect();
    Ok(rows)
}

#[tauri::command]
pub async fn import_projects(
    state: State<'_, AppState>,
    source: ImportSource,
    ids: Vec<String>,
) -> AppResult<ImportResult> {
    let all_sites = import::read_all(source).map_err(|e| AppError::Internal(e.to_string()))?;
    let by_id: HashMap<String, ImportedSite> = all_sites
        .into_iter()
        .map(|s| (s.suggested_id.clone(), s))
        .collect();

    let mut registry = load_registry(&state)?;
    let mut imported: Vec<String> = Vec::new();
    let mut skipped: Vec<SkippedRow> = Vec::new();

    for id in ids {
        let Some(site) = by_id.get(&id) else {
            skipped.push(SkippedRow {
                site: ImportedSite::from_parts(source, String::new(), String::new(), None, false),
                reason: format!("id `{id}` not present in current scan"),
            });
            continue;
        };

        let project = match build_project(site) {
            Ok(p) => p,
            Err(reason) => {
                skipped.push(SkippedRow {
                    site: site.clone(),
                    reason,
                });
                continue;
            }
        };

        match registry.add_project(project) {
            Ok(()) => imported.push(site.suggested_id.clone()),
            Err(e) => skipped.push(SkippedRow {
                site: site.clone(),
                reason: e.to_string(),
            }),
        }
    }

    if !imported.is_empty() {
        save_registry(&state, &registry)?;
        state.reconciler.mark_dirty();
    }

    Ok(ImportResult { imported, skipped })
}

fn build_project(site: &ImportedSite) -> std::result::Result<Project, String> {
    let path = PathBuf::from(&site.path);
    if !path.is_absolute() {
        return Err(format!("path is not absolute: {}", site.path));
    }
    let id = ProjectId::new(&site.suggested_id);
    let kind = if site.php_version.is_some() {
        ProjectType::Php
    } else {
        ProjectType::Custom
    };
    Ok(Project {
        id,
        name: site.suggested_name.clone(),
        path,
        kind,
        start_command: None,
        port: None,
        extra_ports: vec![],
        hostname: site.hostname.clone(),
        https: site.https,
        services: if site.https {
            vec!["caddy".into()]
        } else {
            vec![]
        },
        env: Default::default(),
        readiness: Some(Readiness::Process),
        auto_start: false,
        tags: vec![site.source.tag().to_string()],
        document_root: None,
        php_version: site.php_version.clone(),
        // Populate the structured runtime pin too (not just the legacy field),
        // so imported PHP sites converge onto `runtime` like GUI-created ones.
        runtime: site.php_version.clone().map(|version| Runtime {
            lang: "php".into(),
            version,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::ImportSource;

    #[test]
    fn build_project_marks_php_kind_when_version_present() {
        let site = ImportedSite::from_parts(
            ImportSource::Herd,
            "/tmp/myapp".into(),
            "myapp.test".into(),
            Some("8.3".into()),
            true,
        );
        let p = build_project(&site).unwrap();
        assert!(matches!(p.kind, ProjectType::Php));
        assert_eq!(p.php_version.as_deref(), Some("8.3"));
        assert!(p.https);
        assert_eq!(p.tags, vec!["source:herd"]);
    }

    #[test]
    fn build_project_marks_custom_when_no_php() {
        let site = ImportedSite::from_parts(
            ImportSource::Mamp,
            "/tmp/static-site".into(),
            "static.test".into(),
            None,
            false,
        );
        let p = build_project(&site).unwrap();
        assert!(matches!(p.kind, ProjectType::Custom));
        assert!(p.php_version.is_none());
        assert_eq!(p.tags, vec!["source:mamp"]);
    }

    #[test]
    fn build_project_rejects_relative_path() {
        let site = ImportedSite::from_parts(
            ImportSource::Herd,
            "relative/path".into(),
            "x.test".into(),
            None,
            false,
        );
        assert!(build_project(&site).is_err());
    }
}
