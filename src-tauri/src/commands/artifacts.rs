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
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

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
        ProjectType::Flutter => &[
            ("build", "Flutter build"),
            (".dart_tool", "Dart tool cache"),
            (".pub-cache", "Pub cache"),
        ],
        ProjectType::Xcode => &[("build", "Xcode build"), ("DerivedData", "Derived data")],
        ProjectType::Android => &[
            ("build", "Gradle build"),
            ("app/build", "Android app build"),
            (".gradle", "Gradle cache"),
        ],
        ProjectType::Custom => &[
            ("node_modules", "Dependencies"),
            ("dist", "Build output"),
            ("build", "Build output"),
            (".next", "Next.js build"),
            ("vendor", "Composer packages"),
        ],
    }
}

/// The built-in catalogue plus the user's custom extra dirs (from
/// preferences), applied to every project type. Custom entries that duplicate a
/// built-in key are skipped so a row never appears twice.
fn effective_catalogue(kind: ProjectType, extra: &[String]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = artifact_catalogue(kind)
        .iter()
        .map(|(r, l)| (r.to_string(), l.to_string()))
        .collect();
    for rel in sanitize_extra_dirs(extra) {
        if !out.iter().any(|(r, _)| *r == rel) {
            out.push((rel, "Custom".to_string()));
        }
    }
    out
}

/// Drop custom-dir entries that could escape the project tree or are
/// meaningless as a relative key. `remove_within` re-checks containment as
/// defence in depth, but rejecting these up front keeps bad keys out of scan
/// results and the clean catalogue entirely.
fn sanitize_extra_dirs(extra: &[String]) -> Vec<String> {
    extra
        .iter()
        .map(|s| s.trim().trim_end_matches('/'))
        .filter(|s| !s.is_empty() && !s.starts_with('/') && !s.contains("..") && !s.contains('\\'))
        .map(|s| s.to_string())
        .collect()
}

/// Whether `rel` is a known artifact dir for this project type — including the
/// user's custom dirs. Pure, so the validation that guards deletion is
/// unit-testable.
fn is_known_artifact(kind: ProjectType, rel: &str, extra: &[String]) -> bool {
    effective_catalogue(kind, extra)
        .iter()
        .any(|(r, _)| r == rel)
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
    let extra = state.preferences_snapshot().auto_clean_extra_dirs;

    let present: Vec<(String, String, PathBuf)> = effective_catalogue(kind, &extra)
        .into_iter()
        .filter_map(|(rel, label)| {
            let p = base.join(&rel);
            p.is_dir().then_some((rel, label, p))
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
    let extra = state.preferences_snapshot().auto_clean_extra_dirs;
    if !is_known_artifact(kind, &rel, &extra) {
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
    let extra = state.preferences_snapshot().auto_clean_extra_dirs;
    tokio::task::spawn_blocking(move || clean_project_artifacts(&base, kind, &extra))
        .await
        .map_err(|e| AppError::Internal(format!("artifact clean task failed: {e}")))?
}

/// Delete every catalogued (+ custom) artifact dir present under `base`,
/// returning total bytes reclaimed. Blocking — call inside `spawn_blocking`.
/// Shared by the per-project clean command and the background scheduler.
fn clean_project_artifacts(base: &Path, kind: ProjectType, extra: &[String]) -> AppResult<u64> {
    let mut freed = 0u64;
    for (rel, _) in effective_catalogue(kind, extra) {
        let target = base.join(&rel);
        if target.is_dir() {
            freed += remove_within(base, &target)?;
        }
    }
    Ok(freed)
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

// =============================================================================
// Background auto-clean scheduler
// =============================================================================

/// Tauri event channel carrying the bytes reclaimed by an automatic pass, so
/// the frontend can raise a "Freed N" toast.
pub const AUTO_CLEAN_CHANNEL: &str = "portbay://artifacts-auto-cleaned";

const WEEK_SECS: u64 = 7 * 86_400;
const MONTH_SECS: u64 = 30 * 86_400;
/// How often the running app re-checks whether a pass is due. The cadence gate
/// inside [`run_auto_clean_if_due`] is the real throttle, so checking often is
/// cheap — it mostly returns immediately.
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

/// Whether an auto-clean is due. Pure, so "advance the clock" is unit-testable.
/// `off` — and any unrecognised cadence — is never due.
fn due_for_auto_clean(schedule: &str, last: u64, now: u64) -> bool {
    let cadence = match schedule {
        "weekly" => WEEK_SECS,
        "monthly" => MONTH_SECS,
        _ => return false,
    };
    now.saturating_sub(last) >= cadence
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Run an auto-clean across every registered project when the schedule is due.
/// Stamps `last_auto_clean` whenever a pass runs (even on 0 bytes) so it can't
/// re-fire every check interval, and emits a "freed N" event only when
/// something was actually reclaimed.
async fn run_auto_clean_if_due(app: &AppHandle) {
    let state = app.state::<AppState>();
    let prefs = state.preferences_snapshot();
    let now = now_secs();
    if !due_for_auto_clean(&prefs.auto_clean_schedule, prefs.last_auto_clean, now) {
        return;
    }

    let Ok(registry) = load_registry(&state) else {
        return;
    };
    let extra = prefs.auto_clean_extra_dirs.clone();
    let projects: Vec<(PathBuf, ProjectType)> = registry
        .projects
        .iter()
        .map(|p| (p.path.clone(), p.kind))
        .collect();

    let freed = tokio::task::spawn_blocking(move || {
        let mut total = 0u64;
        for (base, kind) in projects {
            // One project's clean error must not abort the whole pass.
            total += clean_project_artifacts(&base, kind, &extra).unwrap_or(0);
        }
        total
    })
    .await
    .unwrap_or(0);

    // Stamp the pass time regardless of bytes freed, then mirror into state.
    let mut updated = prefs;
    updated.last_auto_clean = now;
    if let Err(e) = updated.save() {
        tracing::warn!(error = %e, "auto-clean: failed to persist last_auto_clean");
    }
    if let Ok(mut guard) = state.preferences.lock() {
        *guard = updated;
    }

    if freed > 0 {
        tracing::info!(freed_bytes = freed, "auto-clean pass reclaimed disk");
        let _ = app.emit(AUTO_CLEAN_CHANNEL, freed);
    }
}

/// Spawn the background auto-clean scheduler for the app's lifetime. Checks once
/// shortly after boot (so a long-overdue clean lands on cold start) and then
/// every [`CHECK_INTERVAL`]. Default cadence is `off`, so this is a no-op until
/// the user opts in from Settings.
pub fn spawn_auto_clean_scheduler(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        // Let the rest of boot settle before walking project trees.
        tokio::time::sleep(Duration::from_secs(30)).await;
        loop {
            run_auto_clean_if_due(&app).await;
            tokio::time::sleep(CHECK_INTERVAL).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn catalogue_matches_known_dirs_only() {
        assert!(is_known_artifact(ProjectType::Next, ".next", &[]));
        assert!(is_known_artifact(ProjectType::Next, "node_modules", &[]));
        assert!(is_known_artifact(ProjectType::Php, "public/build", &[]));
        // Not in the Next catalogue.
        assert!(!is_known_artifact(ProjectType::Next, "vendor", &[]));
        // Never a valid key — guards against path injection.
        assert!(!is_known_artifact(ProjectType::Custom, "../../etc", &[]));
        assert!(!is_known_artifact(ProjectType::Vite, "", &[]));
    }

    #[test]
    fn due_for_auto_clean_respects_cadence_and_off() {
        let now = 1_700_000_000u64;
        // Off / unknown is never due, however long since the last pass.
        assert!(!due_for_auto_clean("off", 0, now));
        assert!(!due_for_auto_clean("nonsense", 0, now));
        // Weekly: due at exactly 7d, not a second before.
        assert!(due_for_auto_clean("weekly", now - WEEK_SECS, now));
        assert!(!due_for_auto_clean("weekly", now - WEEK_SECS + 1, now));
        // Monthly: a fortnight isn't enough; 30d is.
        assert!(!due_for_auto_clean("monthly", now - WEEK_SECS * 2, now));
        assert!(due_for_auto_clean("monthly", now - MONTH_SECS, now));
    }

    #[test]
    fn sanitize_extra_dirs_rejects_escapes_and_blanks() {
        let got = sanitize_extra_dirs(&[
            ".turbo".into(),
            "  .cache/ ".into(), // trimmed + trailing slash stripped
            "".into(),
            "../escape".into(),
            "/abs".into(),
            "a\\b".into(),
        ]);
        assert_eq!(got, vec![".turbo".to_string(), ".cache".to_string()]);
    }

    #[test]
    fn effective_catalogue_appends_custom_without_duplicating() {
        // `node_modules` is already a Next built-in; the custom entry must not
        // duplicate it, while `.turbo` is genuinely added.
        let extra = vec![".turbo".to_string(), "node_modules".to_string()];
        let rels: Vec<String> = effective_catalogue(ProjectType::Next, &extra)
            .into_iter()
            .map(|(r, _)| r)
            .collect();
        assert!(rels.contains(&".turbo".to_string()));
        assert_eq!(rels.iter().filter(|r| *r == "node_modules").count(), 1);
    }

    #[test]
    fn is_known_artifact_honours_custom_dirs() {
        assert!(is_known_artifact(
            ProjectType::Next,
            ".turbo",
            &[".turbo".to_string()]
        ));
        assert!(!is_known_artifact(ProjectType::Next, ".turbo", &[]));
    }

    #[test]
    fn clean_project_artifacts_removes_known_and_custom_only() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().to_path_buf();
        // `.next` is a built-in, `.turbo` a custom extra, `untracked` neither.
        for d in [".next", ".turbo", "untracked"] {
            fs::create_dir_all(base.join(d)).unwrap();
            fs::write(base.join(d).join("f"), b"0123456789").unwrap(); // 10 bytes
        }
        let freed =
            clean_project_artifacts(&base, ProjectType::Next, &[".turbo".to_string()]).unwrap();
        assert_eq!(freed, 20); // .next + .turbo, not untracked
        assert!(!base.join(".next").exists());
        assert!(!base.join(".turbo").exists());
        assert!(base.join("untracked").exists());
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
