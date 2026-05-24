//! Build-artifact scanning and cleanup for the project detail panel.
//!
//! Surfaces the disk weight of common build-output directories (`.next`,
//! `dist`, `node_modules`, `vendor`, …) per project type and lets the user
//! reclaim space with a one-click clean — the local-dev equivalent of
//! `rm -rf .next node_modules`.
//!
//! Safety: the clean commands take a *relative* dir key, not a
//! frontend-supplied absolute path. The key must match this module's
//! hardcoded catalogue for the project's type, and the resolved target is
//! re-checked to live inside the project folder before any deletion. Symlinks
//! are never followed while measuring, so a measure/clean can't escape the
//! project tree.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::State;

use crate::commands::projects::load_registry;
use crate::error::{AppError, AppResult};
use crate::registry::{ProjectId, ProjectType};
use crate::state::AppState;

/// One scanned artifact directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactDir {
    /// Project-relative key (e.g. ".next", "public/build"). The clean
    /// commands accept this, never an absolute path.
    pub rel: String,
    /// Human label for the row.
    pub label: String,
    /// Absolute path, for display + "reveal in Finder" affordances.
    pub path: String,
    pub size_bytes: u64,
    pub file_count: u64,
    /// Newest file mtime as Unix seconds, or `None` for an empty dir.
    pub last_modified: Option<u64>,
}

/// Known build-output dirs per project type: `(relative path, label)`.
/// Hardcoded so a clean target can never be an arbitrary user string.
fn artifact_catalogue(kind: ProjectType) -> &'static [(&'static str, &'static str)] {
    match kind {
        ProjectType::Next => &[(".next", "Next.js build"), ("node_modules", "Dependencies")],
        ProjectType::Vite => &[("dist", "Vite build"), ("node_modules", "Dependencies")],
        ProjectType::Node => &[("dist", "Build output"), ("node_modules", "Dependencies")],
        ProjectType::Php => &[
            ("vendor", "Composer packages"),
            ("public/build", "Front-end build"),
            ("bootstrap/cache", "Bootstrap cache"),
            ("storage/framework/cache", "Framework cache"),
        ],
        ProjectType::Static => &[("dist", "Build output"), ("build", "Build output")],
        ProjectType::Custom => &[
            ("node_modules", "Dependencies"),
            ("dist", "Build output"),
            ("build", "Build output"),
            (".next", "Next.js build"),
            ("vendor", "Composer packages"),
        ],
    }
}

/// Whether `rel` is a known artifact dir for this project type. Pure, so the
/// validation that guards deletion is unit-testable.
fn is_known_artifact(kind: ProjectType, rel: &str) -> bool {
    artifact_catalogue(kind).iter().any(|(r, _)| *r == rel)
}

/// `scan_artifacts(id)` — measure every catalogued artifact dir that exists in
/// the project folder. The walk is blocking, so it runs off the async runtime.
#[tauri::command]
pub async fn scan_artifacts(state: State<'_, AppState>, id: String) -> AppResult<Vec<ArtifactDir>> {
    let registry = load_registry(&state)?;
    let project = registry
        .get_project(&ProjectId::new(id.clone()))
        .ok_or_else(|| AppError::NotFound(id.clone()))?;
    let base = project.path.clone();
    let kind = project.kind;

    let present: Vec<(String, String, PathBuf)> = artifact_catalogue(kind)
        .iter()
        .filter_map(|(rel, label)| {
            let p = base.join(rel);
            p.is_dir().then(|| (rel.to_string(), label.to_string(), p))
        })
        .collect();

    let result = tokio::task::spawn_blocking(move || {
        present
            .into_iter()
            .map(|(rel, label, p)| {
                let (size_bytes, file_count, last_modified) = measure_dir(&p);
                ArtifactDir {
                    rel,
                    label,
                    path: p.to_string_lossy().into_owned(),
                    size_bytes,
                    file_count,
                    last_modified,
                }
            })
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|e| AppError::Internal(format!("artifact scan task failed: {e}")))?;

    Ok(result)
}

/// `clean_artifact(id, rel)` — delete one catalogued artifact dir. Returns the
/// number of bytes reclaimed. A no-op (returns 0) if the dir is absent.
#[tauri::command]
pub async fn clean_artifact(state: State<'_, AppState>, id: String, rel: String) -> AppResult<u64> {
    let (base, kind) = project_base(&state, &id)?;
    if !is_known_artifact(kind, &rel) {
        return Err(AppError::BadInput(format!(
            "`{rel}` is not a known artifact directory for this project"
        )));
    }
    let target = base.join(&rel);
    tokio::task::spawn_blocking(move || remove_within(&base, &target))
        .await
        .map_err(|e| AppError::Internal(format!("artifact clean task failed: {e}")))?
}

/// `clean_all_artifacts(id)` — delete every catalogued artifact dir present in
/// the project. Returns total bytes reclaimed.
#[tauri::command]
pub async fn clean_all_artifacts(state: State<'_, AppState>, id: String) -> AppResult<u64> {
    let (base, kind) = project_base(&state, &id)?;
    let targets: Vec<PathBuf> = artifact_catalogue(kind)
        .iter()
        .map(|(rel, _)| base.join(rel))
        .filter(|p| p.is_dir())
        .collect();

    tokio::task::spawn_blocking(move || {
        let mut freed = 0u64;
        for target in targets {
            freed += remove_within(&base, &target)?;
        }
        Ok(freed)
    })
    .await
    .map_err(|e| AppError::Internal(format!("artifact clean task failed: {e}")))?
}

/// Resolve a project's base path + type, erroring if the id is unknown.
fn project_base(state: &AppState, id: &str) -> AppResult<(PathBuf, ProjectType)> {
    let registry = load_registry(state)?;
    let project = registry
        .get_project(&ProjectId::new(id))
        .ok_or_else(|| AppError::NotFound(id.to_string()))?;
    Ok((project.path.clone(), project.kind))
}

/// Measure (and then delete) `target`, but only after confirming it really
/// lives inside `base` — defence in depth against a symlinked artifact dir
/// pointing elsewhere. Returns bytes reclaimed; 0 if the dir is absent.
fn remove_within(base: &Path, target: &Path) -> AppResult<u64> {
    if !target.is_dir() {
        return Ok(0);
    }
    let base_c = base
        .canonicalize()
        .map_err(|e| AppError::Internal(format!("canonicalize project path: {e}")))?;
    let target_c = target
        .canonicalize()
        .map_err(|e| AppError::Internal(format!("canonicalize artifact path: {e}")))?;
    if !target_c.starts_with(&base_c) {
        return Err(AppError::BadInput(
            "artifact directory resolves outside the project folder".into(),
        ));
    }
    let (size, _, _) = measure_dir(&target_c);
    std::fs::remove_dir_all(&target_c)
        .map_err(|e| AppError::Internal(format!("remove artifact directory: {e}")))?;
    Ok(size)
}

/// Iteratively sum a directory's file sizes, file count, and newest mtime.
/// Symlinks are skipped (never followed) so the walk can't cycle or escape.
fn measure_dir(root: &Path) -> (u64, u64, Option<u64>) {
    use std::time::UNIX_EPOCH;

    let mut size = 0u64;
    let mut count = 0u64;
    let mut newest: Option<u64> = None;
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                stack.push(entry.path());
            } else if ft.is_file() {
                if let Ok(meta) = entry.metadata() {
                    size += meta.len();
                    count += 1;
                    if let Ok(d) = meta.modified().and_then(|m| {
                        m.duration_since(UNIX_EPOCH)
                            .map_err(|_| std::io::ErrorKind::Other.into())
                    }) {
                        let s = d.as_secs();
                        newest = Some(newest.map_or(s, |n| n.max(s)));
                    }
                }
            }
        }
    }
    (size, count, newest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn catalogue_matches_known_dirs_only() {
        assert!(is_known_artifact(ProjectType::Next, ".next"));
        assert!(is_known_artifact(ProjectType::Next, "node_modules"));
        assert!(is_known_artifact(ProjectType::Php, "public/build"));
        // Not in the Next catalogue.
        assert!(!is_known_artifact(ProjectType::Next, "vendor"));
        // Never a valid key — guards against path injection.
        assert!(!is_known_artifact(ProjectType::Custom, "../../etc"));
        assert!(!is_known_artifact(ProjectType::Vite, ""));
    }

    #[test]
    fn measure_dir_sums_size_and_count() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("dist");
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::write(root.join("a.js"), b"hello").unwrap(); // 5 bytes
        fs::write(root.join("nested/b.js"), b"world!!").unwrap(); // 7 bytes

        let (size, count, mtime) = measure_dir(&root);
        assert_eq!(size, 12);
        assert_eq!(count, 2);
        assert!(mtime.is_some());
    }

    #[test]
    fn remove_within_deletes_and_reports_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().to_path_buf();
        let target = base.join("node_modules");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("pkg.js"), b"0123456789").unwrap(); // 10 bytes

        let freed = remove_within(&base, &target).unwrap();
        assert_eq!(freed, 10);
        assert!(!target.exists(), "artifact dir must be removed");
    }

    #[test]
    fn remove_within_is_a_noop_for_absent_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().to_path_buf();
        let freed = remove_within(&base, &base.join("dist")).unwrap();
        assert_eq!(freed, 0);
    }
}
