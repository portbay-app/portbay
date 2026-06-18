//! Build-artifact scanning and cleanup for the project detail panel.
//!
//! Surfaces the disk weight of common build-output directories (`.next`,
//! `dist`, `node_modules`, `vendor`, …) per project type and lets the user
//! reclaim space with a one-click clean — the local-dev equivalent of
//! `rm -rf .next node_modules`.
//!
//! ## Two cleanup surfaces, two safety levels
//!
//! 1. **Manual clean** (the user clicks a row, or "Clean all" in the detail
//!    panel) may remove *anything* in the catalogue — including dependency
//!    stores (`node_modules`, `vendor`, `.venv`) and build output (`dist`,
//!    `.next`). That's a deliberate, eyes-on action: the user sees the size and
//!    chooses, the way `npkill`/CleanMyMac surface reclaimable space.
//!
//! 2. **Scheduled auto-clean** (the unattended background pass) is far more
//!    conservative. It only ever deletes dirs flagged [`auto_safe`] — pure,
//!    regenerable caches that (a) rebuild locally with **no network**, and
//!    (b) cannot break a running process or the project's integrity if removed.
//!    Dependency stores and build *output* are never touched by the scheduler,
//!    because deleting them forces a reinstall/rebuild (network) or can break a
//!    live dev server. This split is why a weekly pass can no longer wipe
//!    `node_modules` out from under a project.
//!
//! Safety (both surfaces): the clean commands take a *relative* dir key, not a
//! frontend-supplied absolute path. The key must match this module's
//! hardcoded catalogue for the project's type, and the resolved target is
//! re-checked to live inside the project folder before any deletion. Symlinks
//! are never followed while measuring, so a measure/clean can't escape the
//! project tree.
//!
//! [`auto_safe`]: artifact_catalogue

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
    /// Whether the background scheduler will auto-delete this dir. `true` only
    /// for regenerable caches (rebuild locally, no reinstall, no integrity
    /// risk); `false` for dependency stores and build output, which stay
    /// manual-only. Lets the UI mark a row "won't be auto-cleaned".
    pub auto_clean: bool,
}

/// One catalogue entry: `(project-relative path, label, auto_safe)`.
///
/// `auto_safe` is the entire safety contract of the background scheduler. It is
/// `true` **only** for a regenerable cache that:
///   * rebuilds locally with no network access, and
///   * cannot break a running process or the project's integrity if deleted.
///
/// Everything a project needs reinstalled (`node_modules`, `vendor`, `.venv`,
/// `.gradle`, `.pub-cache`) or rebuilt and possibly served (`dist`, `build`,
/// `.next`, `public/build`, `*/build`) is `auto_safe = false`: still offered
/// for a deliberate manual clean, but never swept by the unattended pass. This
/// is the line that keeps a scheduled clean from forcing a `pnpm install`.
type Artifact = (&'static str, &'static str, bool);

const AUTO_SAFE: bool = true;
const MANUAL_ONLY: bool = false;

/// Known artifact dirs per project type: `(relative path, label, auto_safe)`.
/// Hardcoded so a clean target can never be an arbitrary user string.
fn artifact_catalogue(kind: ProjectType) -> &'static [Artifact] {
    match kind {
        // Node family: the package store and primary build output are
        // manual-only; the tool/build *caches* under them are auto-safe.
        ProjectType::Next => &[
            (".next/cache", "Next.js build cache", AUTO_SAFE),
            ("node_modules/.cache", "Tooling cache", AUTO_SAFE),
            (".turbo", "Turbo cache", AUTO_SAFE),
            (".next", "Next.js build", MANUAL_ONLY),
            ("node_modules", "Dependencies", MANUAL_ONLY),
        ],
        ProjectType::Vite => &[
            ("node_modules/.vite", "Vite dep cache", AUTO_SAFE),
            ("node_modules/.cache", "Tooling cache", AUTO_SAFE),
            (".turbo", "Turbo cache", AUTO_SAFE),
            ("dist", "Vite build", MANUAL_ONLY),
            ("node_modules", "Dependencies", MANUAL_ONLY),
        ],
        ProjectType::Node => &[
            ("node_modules/.cache", "Tooling cache", AUTO_SAFE),
            (".turbo", "Turbo cache", AUTO_SAFE),
            ("dist", "Build output", MANUAL_ONLY),
            ("node_modules", "Dependencies", MANUAL_ONLY),
        ],
        // JS meta-frameworks: each one's framework cache regenerates on the next
        // dev/build (auto-safe); the package store and shipped build output are
        // manual-only.
        ProjectType::Astro => &[
            (".astro", "Astro cache", AUTO_SAFE),
            ("node_modules/.vite", "Vite dep cache", AUTO_SAFE),
            ("dist", "Astro build", MANUAL_ONLY),
            ("node_modules", "Dependencies", MANUAL_ONLY),
        ],
        ProjectType::SvelteKit => &[
            (".svelte-kit", "SvelteKit cache", AUTO_SAFE),
            ("node_modules/.vite", "Vite dep cache", AUTO_SAFE),
            ("build", "SvelteKit build", MANUAL_ONLY),
            ("node_modules", "Dependencies", MANUAL_ONLY),
        ],
        ProjectType::Nuxt => &[
            (".nuxt", "Nuxt dev build", AUTO_SAFE),
            ("node_modules/.cache", "Tooling cache", AUTO_SAFE),
            (".output", "Nuxt build", MANUAL_ONLY),
            ("node_modules", "Dependencies", MANUAL_ONLY),
        ],
        ProjectType::Remix => &[
            ("node_modules/.cache", "Tooling cache", AUTO_SAFE),
            ("build", "Remix build", MANUAL_ONLY),
            ("public/build", "Client build", MANUAL_ONLY),
            ("node_modules", "Dependencies", MANUAL_ONLY),
        ],
        ProjectType::Gatsby => &[
            (".cache", "Gatsby cache", AUTO_SAFE),
            ("public", "Gatsby build", MANUAL_ONLY),
            ("node_modules", "Dependencies", MANUAL_ONLY),
        ],
        ProjectType::Angular => &[
            (".angular/cache", "Angular cache", AUTO_SAFE),
            ("node_modules/.cache", "Tooling cache", AUTO_SAFE),
            ("dist", "Angular build", MANUAL_ONLY),
            ("node_modules", "Dependencies", MANUAL_ONLY),
        ],
        ProjectType::SolidStart => &[
            (".vinxi", "Vinxi cache", AUTO_SAFE),
            ("node_modules/.vite", "Vite dep cache", AUTO_SAFE),
            (".output", "SolidStart build", MANUAL_ONLY),
            ("node_modules", "Dependencies", MANUAL_ONLY),
        ],
        ProjectType::Qwik => &[
            ("node_modules/.vite", "Vite dep cache", AUTO_SAFE),
            ("dist", "Qwik build", MANUAL_ONLY),
            ("server", "Qwik SSR build", MANUAL_ONLY),
            ("node_modules", "Dependencies", MANUAL_ONLY),
        ],
        ProjectType::VueCli => &[
            ("node_modules/.cache", "Tooling cache", AUTO_SAFE),
            ("dist", "Vue build", MANUAL_ONLY),
            ("node_modules", "Dependencies", MANUAL_ONLY),
        ],
        ProjectType::Preact => &[
            ("node_modules/.cache", "Tooling cache", AUTO_SAFE),
            ("build", "Preact build", MANUAL_ONLY),
            ("node_modules", "Dependencies", MANUAL_ONLY),
        ],
        // Go (+ Hugo, which runs under the Go kind): tool caches regenerate;
        // vendored deps and build outputs are manual.
        ProjectType::Go => &[
            ("resources/_gen", "Hugo cache", AUTO_SAFE),
            ("public", "Build output", MANUAL_ONLY),
            ("vendor", "Vendored modules", MANUAL_ONLY),
            ("bin", "Compiled binary", MANUAL_ONLY),
        ],
        // Ruby (Rails/Jekyll): the framework cache is auto-safe; bundled gems
        // and the generated site need a `bundle install` / rebuild to restore.
        ProjectType::Ruby => &[
            ("tmp/cache", "Rails cache", AUTO_SAFE),
            ("_site", "Jekyll build", MANUAL_ONLY),
            ("vendor/bundle", "Bundled gems", MANUAL_ONLY),
        ],
        // Rust: `target` is the build cache + output; rebuild recompiles, so
        // manual-only.
        ProjectType::Rust => &[("target", "Cargo build", MANUAL_ONLY)],
        // Deno caches modules globally; only the Fresh build output is local.
        ProjectType::Deno => &[("_fresh", "Fresh build", MANUAL_ONLY)],
        // Elixir: `_build` recompiles with no network (auto-safe); `deps` needs
        // a `mix deps.get`.
        ProjectType::Elixir => &[
            ("_build", "Mix build", AUTO_SAFE),
            ("deps", "Mix deps", MANUAL_ONLY),
        ],
        // .NET: `obj` intermediates rebuild; `bin` is the build output.
        ProjectType::DotNet => &[
            ("obj", "Build intermediates", AUTO_SAFE),
            ("bin", "Build output", MANUAL_ONLY),
        ],
        // JVM (Maven `target/`, Gradle `build/` + `.gradle/`): all rebuild, but
        // a clean re-downloads Gradle's dependency cache, so manual-only.
        ProjectType::Java | ProjectType::Kotlin => &[
            ("target", "Maven build", MANUAL_ONLY),
            ("build", "Gradle build", MANUAL_ONLY),
            (".gradle", "Gradle cache", MANUAL_ONLY),
        ],
        ProjectType::Scala => &[
            ("target", "sbt build", MANUAL_ONLY),
            ("project/target", "sbt project cache", MANUAL_ONLY),
        ],
        ProjectType::Clojure => &[
            (".cpcache", "deps.edn cache", AUTO_SAFE),
            ("target", "Build output", MANUAL_ONLY),
        ],
        ProjectType::Crystal => &[
            ("lib", "Shards", MANUAL_ONLY),
            ("bin", "Compiled binary", MANUAL_ONLY),
        ],
        ProjectType::Dart => &[
            (".dart_tool", "Dart tool cache", AUTO_SAFE),
            ("build", "Build output", MANUAL_ONLY),
        ],
        // Swift's `.build` holds both the resolved deps and the build products.
        ProjectType::Swift => &[(".build", "SwiftPM build", MANUAL_ONLY)],
        ProjectType::Zig => &[
            ("zig-cache", "Zig cache", AUTO_SAFE),
            (".zig-cache", "Zig cache", AUTO_SAFE),
            ("zig-out", "Build output", MANUAL_ONLY),
        ],
        ProjectType::Nim => &[("nimcache", "Nim cache", AUTO_SAFE)],
        ProjectType::Haskell => &[
            (".stack-work", "Stack build", MANUAL_ONLY),
            ("dist-newstyle", "Cabal build", MANUAL_ONLY),
        ],
        ProjectType::OCaml => &[("_build", "Dune build", MANUAL_ONLY)],
        // Laravel/PHP: the framework regenerates its own caches on demand, so
        // they're auto-safe; Composer packages and the compiled front-end are
        // manual-only (a `composer install` / `npm run build` to restore).
        ProjectType::Php => &[
            ("bootstrap/cache", "Bootstrap cache", AUTO_SAFE),
            ("storage/framework/cache", "Framework cache", AUTO_SAFE),
            ("storage/framework/views", "Compiled views", AUTO_SAFE),
            ("vendor", "Composer packages", MANUAL_ONLY),
            ("public/build", "Front-end build", MANUAL_ONLY),
        ],
        ProjectType::Python => &[
            ("__pycache__", "Bytecode cache", AUTO_SAFE),
            (".pytest_cache", "pytest cache", AUTO_SAFE),
            (".mypy_cache", "mypy cache", AUTO_SAFE),
            (".ruff_cache", "ruff cache", AUTO_SAFE),
            (".venv", "Virtualenv", MANUAL_ONLY),
            ("dist", "Build output", MANUAL_ONLY),
        ],
        // A static site's `dist`/`build` is what's being served — never sweep
        // it automatically; only a generic build cache is auto-safe.
        ProjectType::Static => &[
            (".cache", "Build cache", AUTO_SAFE),
            ("dist", "Build output", MANUAL_ONLY),
            ("build", "Build output", MANUAL_ONLY),
        ],
        // Flutter/Dart: every dir here needs a network `pub get` or a rebuild to
        // restore, so none are auto-safe.
        ProjectType::Flutter => &[
            ("build", "Flutter build", MANUAL_ONLY),
            (".dart_tool", "Dart tool cache", MANUAL_ONLY),
            (".pub-cache", "Pub cache", MANUAL_ONLY),
        ],
        // Xcode's DerivedData is the canonical safe-to-delete cache (Xcode
        // rebuilds it with no network); the build product is manual-only.
        ProjectType::Xcode => &[
            ("DerivedData", "Derived data", AUTO_SAFE),
            ("build", "Xcode build", MANUAL_ONLY),
        ],
        // Gradle re-downloads dependencies when its caches are cleared, so
        // `.gradle` is manual-only; build outputs likewise.
        ProjectType::Android => &[
            ("build", "Gradle build", MANUAL_ONLY),
            ("app/build", "Android app build", MANUAL_ONLY),
            (".gradle", "Gradle cache", MANUAL_ONLY),
        ],
        ProjectType::Expo => &[
            (".expo", "Expo cache", AUTO_SAFE),
            ("node_modules/.cache", "Tooling cache", AUTO_SAFE),
            ("node_modules", "Dependencies", MANUAL_ONLY),
            ("ios/build", "iOS build", MANUAL_ONLY),
            ("android/build", "Android build", MANUAL_ONLY),
        ],
        // Custom projects have an unknown shape: only the universally-safe
        // tooling caches are auto-cleaned; everything reinstall-or-rebuild stays
        // manual-only.
        ProjectType::Custom => &[
            (".turbo", "Turbo cache", AUTO_SAFE),
            (".cache", "Build cache", AUTO_SAFE),
            ("node_modules/.cache", "Tooling cache", AUTO_SAFE),
            ("node_modules", "Dependencies", MANUAL_ONLY),
            ("dist", "Build output", MANUAL_ONLY),
            ("build", "Build output", MANUAL_ONLY),
            (".next", "Next.js build", MANUAL_ONLY),
            ("vendor", "Composer packages", MANUAL_ONLY),
        ],
    }
}

/// The built-in catalogue plus the user's custom extra dirs (from
/// preferences), applied to every project type. Custom entries that duplicate a
/// built-in key are skipped so a row never appears twice.
///
/// User-added extra dirs are treated as `auto_safe`: the whole purpose of the
/// "extra dirs to clean" setting is to opt those paths into the auto pass, so
/// honouring that is the user's explicit choice (and they default to caches
/// like `.turbo`/`.cache`).
fn effective_catalogue(kind: ProjectType, extra: &[String]) -> Vec<(String, String, bool)> {
    let mut out: Vec<(String, String, bool)> = artifact_catalogue(kind)
        .iter()
        .map(|(r, l, safe)| (r.to_string(), l.to_string(), *safe))
        .collect();
    for rel in sanitize_extra_dirs(extra) {
        if !out.iter().any(|(r, _, _)| *r == rel) {
            out.push((rel, "Custom".to_string(), AUTO_SAFE));
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
        .any(|(r, _, _)| r == rel)
}

/// Which cleanup surface a clean pass is running on. A manual clean removes the
/// full catalogue; the unattended scheduler removes only `auto_safe` caches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CleanScope {
    /// Manual, eyes-on clean — may delete dependency stores and build output.
    Manual,
    /// Background scheduler — regenerable caches only.
    Scheduled,
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

    let present: Vec<(String, String, bool, PathBuf)> = effective_catalogue(kind, &extra)
        .into_iter()
        .filter_map(|(rel, label, auto_safe)| {
            let p = base.join(&rel);
            p.is_dir().then_some((rel, label, auto_safe, p))
        })
        .collect();

    let result = tokio::task::spawn_blocking(move || {
        present
            .into_iter()
            .map(|(rel, label, auto_clean, p)| {
                let (size_bytes, file_count, last_modified) = measure_dir(&p);
                ArtifactDir {
                    rel,
                    label,
                    path: p.to_string_lossy().into_owned(),
                    size_bytes,
                    file_count,
                    last_modified,
                    auto_clean,
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
    // A manual "Clean all" is an eyes-on action, so it may remove the full
    // catalogue (dependency stores and build output included).
    tokio::task::spawn_blocking(move || {
        clean_project_artifacts(&base, kind, &extra, CleanScope::Manual)
    })
    .await
    .map_err(|e| AppError::Internal(format!("artifact clean task failed: {e}")))?
}

/// Delete catalogued (+ custom) artifact dirs present under `base`, returning
/// total bytes reclaimed. Blocking — call inside `spawn_blocking`.
///
/// `scope` gates *what* is eligible: [`CleanScope::Manual`] removes the whole
/// catalogue; [`CleanScope::Scheduled`] removes only `auto_safe` caches, so the
/// background pass can never delete a dependency store or build output.
fn clean_project_artifacts(
    base: &Path,
    kind: ProjectType,
    extra: &[String],
    scope: CleanScope,
) -> AppResult<u64> {
    let mut freed = 0u64;
    for (rel, _, auto_safe) in effective_catalogue(kind, extra) {
        if scope == CleanScope::Scheduled && !auto_safe {
            continue;
        }
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
            // Scheduled scope: regenerable caches only — never a dependency
            // store or build output. One project's error must not abort the pass.
            total +=
                clean_project_artifacts(&base, kind, &extra, CleanScope::Scheduled).unwrap_or(0);
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

/// Delete `.log` files under `logs_dir` (recursively) whose mtime is older than
/// `retention_days`. `0` means "keep forever" (no-op). Best-effort; returns how
/// many files were removed. Only stale logs (older than the window) are touched,
/// so the current session's fresh, open log files are never affected.
pub fn prune_logs(logs_dir: &std::path::Path, retention_days: u32) -> usize {
    if retention_days == 0 {
        return 0;
    }
    let Some(cutoff) = std::time::SystemTime::now()
        .checked_sub(Duration::from_secs(u64::from(retention_days) * 86_400))
    else {
        return 0;
    };
    fn walk(dir: &std::path::Path, cutoff: std::time::SystemTime, removed: &mut usize) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if meta.is_dir() {
                walk(&path, cutoff, removed);
            } else if path.extension().and_then(|e| e.to_str()) == Some("log")
                && meta.modified().map(|m| m < cutoff).unwrap_or(false)
                && std::fs::remove_file(&path).is_ok()
            {
                *removed += 1;
            }
        }
    }
    let mut removed = 0;
    walk(logs_dir, cutoff, &mut removed);
    removed
}

/// Log-retention pass: prune stale per-process logs per the user's
/// `log_retention_days` setting. Independent of the auto-clean *schedule* (it
/// has its own control), so it runs every scheduler tick — cheap: a dir walk +
/// mtime check.
async fn run_log_retention(app: &AppHandle) {
    let state = app.state::<AppState>();
    let retention = state.preferences_snapshot().log_retention_days;
    if retention == 0 {
        return;
    }
    let logs_dir = state.logs_dir.clone();
    let removed = tokio::task::spawn_blocking(move || prune_logs(&logs_dir, retention))
        .await
        .unwrap_or(0);
    if removed > 0 {
        tracing::info!(
            removed,
            retention_days = retention,
            "log retention: pruned stale logs"
        );
    }
}

/// Scan `folder`'s immediate subdirectories for project-like dirs not already
/// registered. "Project-like" = a recognised framework dev command, or a PHP
/// app — plain/empty folders are ignored so we don't surface noise.
fn scan_workspace(folder: &Path, registered: &std::collections::HashSet<PathBuf>) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let canon = path.canonicalize().unwrap_or(path);
        if registered.contains(&canon) {
            continue;
        }
        let det = crate::commands::projects::detect_kind(&canon);
        let looks_like_project = det.start_command.is_some()
            || matches!(det.kind, crate::registry::ProjectType::Php)
            || det.php_version.is_some();
        if looks_like_project {
            out.push(canon);
        }
    }
    out
}

/// Auto-detect pass: when enabled, scan the user's default workspace folder for
/// unregistered project dirs and fire ONE desktop notification for any newly
/// seen since last time (tracked in `detected-projects.json` so the user isn't
/// re-pinged about the same dirs). Independent of the auto-clean schedule.
async fn run_auto_detect(app: &AppHandle) {
    let state = app.state::<AppState>();
    let prefs = state.preferences_snapshot();
    if !prefs.auto_detect_projects {
        return;
    }
    let folder = prefs.default_workspace_folder.trim().to_string();
    if folder.is_empty() {
        return;
    }
    let Ok(registry) = load_registry(&state) else {
        return;
    };
    let registered: std::collections::HashSet<PathBuf> = registry
        .list_projects()
        .iter()
        .filter_map(|p| p.path.canonicalize().ok())
        .collect();
    let data_dir = state
        .logs_dir
        .parent()
        .unwrap_or(&state.logs_dir)
        .to_path_buf();

    let found =
        tokio::task::spawn_blocking(move || scan_workspace(Path::new(&folder), &registered))
            .await
            .unwrap_or_default();
    if found.is_empty() {
        return;
    }

    // Only notify about dirs we haven't surfaced before.
    let seen_path = data_dir.join("detected-projects.json");
    let mut seen: std::collections::HashSet<String> = std::fs::read(&seen_path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let new_count = found
        .iter()
        .filter(|p| seen.insert(p.display().to_string()))
        .count();
    if new_count == 0 {
        return;
    }
    if let Ok(json) = serde_json::to_vec(&seen) {
        let _ = std::fs::write(&seen_path, json);
    }
    notify_detected(new_count);
}

/// Fire-and-forget desktop notification about newly detected projects.
fn notify_detected(count: usize) {
    crate::notifications::desktop_banner(
        "PortBay",
        &format!("Found {count} new project(s) in your workspace - open PortBay to add them."),
    );
}

/// Spawn the background auto-clean + log-retention + auto-detect scheduler for
/// the app's lifetime. Checks once shortly after boot (so a long-overdue clean
/// lands on cold start) and then every [`CHECK_INTERVAL`]. Auto-clean defaults
/// to `off`; log retention defaults to 7 days; auto-detect defaults to off.
pub fn spawn_auto_clean_scheduler(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        // Let the rest of boot settle before walking project trees.
        tokio::time::sleep(Duration::from_secs(30)).await;
        loop {
            run_auto_clean_if_due(&app).await;
            run_log_retention(&app).await;
            run_auto_detect(&app).await;
            tokio::time::sleep(CHECK_INTERVAL).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn prune_logs_removes_only_stale_log_files() {
        let dir = tempfile::tempdir().unwrap();
        let logs = dir.path();
        let stale = logs.join("old.log");
        let fresh = logs.join("new.log");
        let other = logs.join("keep.txt"); // non-.log, also stale
        fs::write(&stale, b"x").unwrap();
        fs::write(&fresh, b"x").unwrap();
        fs::write(&other, b"x").unwrap();
        let old = std::time::SystemTime::now() - Duration::from_secs(100 * 86_400);
        fs::OpenOptions::new()
            .write(true)
            .open(&stale)
            .unwrap()
            .set_modified(old)
            .unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&other)
            .unwrap()
            .set_modified(old)
            .unwrap();

        // 0 = keep forever → no-op.
        assert_eq!(prune_logs(logs, 0), 0);
        assert!(stale.exists());

        // 7-day window → deletes the stale .log only.
        assert_eq!(prune_logs(logs, 7), 1);
        assert!(!stale.exists(), "stale .log should be pruned");
        assert!(fresh.exists(), "fresh .log must be kept");
        assert!(other.exists(), "non-.log must be kept");
    }

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
        let cat = effective_catalogue(ProjectType::Next, &extra);
        let rels: Vec<String> = cat.iter().map(|(r, _, _)| r.clone()).collect();
        assert!(rels.contains(&".turbo".to_string()));
        assert_eq!(rels.iter().filter(|r| *r == "node_modules").count(), 1);
        // node_modules stays manual-only even though `.turbo` was added as a
        // user extra; an extra-supplied dir is treated as auto-safe.
        let auto_safe = |rel: &str| cat.iter().find(|(r, _, _)| r == rel).map(|(_, _, s)| *s);
        assert_eq!(auto_safe("node_modules"), Some(false));
        assert_eq!(auto_safe(".turbo"), Some(true));
    }

    /// The core safety invariant: no dependency store or rebuildable output dir
    /// is ever `auto_safe`, for ANY project type. This is the guard that stops a
    /// scheduled pass from wiping `node_modules`/`.venv`/`vendor`.
    #[test]
    fn scheduled_clean_never_targets_dependency_or_output_dirs() {
        const NEVER_AUTO: &[&str] = &[
            "node_modules",
            ".venv",
            "vendor",
            ".gradle",
            ".pub-cache",
            ".dart_tool",
            "dist",
            "build",
            ".next",
            "public/build",
            "app/build",
            "ios/build",
            "android/build",
        ];
        let kinds = [
            ProjectType::Next,
            ProjectType::Vite,
            ProjectType::Node,
            ProjectType::Php,
            ProjectType::Python,
            ProjectType::Static,
            ProjectType::Flutter,
            ProjectType::Xcode,
            ProjectType::Android,
            ProjectType::Expo,
            ProjectType::Custom,
        ];
        for kind in kinds {
            for (rel, _, auto_safe) in artifact_catalogue(kind) {
                if NEVER_AUTO.contains(rel) {
                    assert!(!auto_safe, "{rel} must never be auto-safe (kind {kind:?})");
                }
            }
        }
    }

    #[test]
    fn is_known_artifact_honours_custom_dirs() {
        // `coverage` is in no built-in catalogue, so it's only a valid clean
        // target when the user has added it as an extra dir.
        assert!(is_known_artifact(
            ProjectType::Next,
            "coverage",
            &["coverage".to_string()]
        ));
        assert!(!is_known_artifact(ProjectType::Next, "coverage", &[]));
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
        let freed = clean_project_artifacts(
            &base,
            ProjectType::Next,
            &[".turbo".to_string()],
            CleanScope::Manual,
        )
        .unwrap();
        assert_eq!(freed, 20); // .next + .turbo, not untracked
        assert!(!base.join(".next").exists());
        assert!(!base.join(".turbo").exists());
        assert!(base.join("untracked").exists());
    }

    #[test]
    fn scheduled_clean_keeps_node_modules_and_build_but_clears_caches() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().to_path_buf();
        // Manual-only: a dependency store and the build output.
        // Auto-safe: the Next build cache, a tooling cache, and a user extra.
        for d in [
            "node_modules",
            ".next",
            ".next/cache",
            "node_modules/.cache",
            ".turbo",
        ] {
            fs::create_dir_all(base.join(d)).unwrap();
            fs::write(base.join(d).join("f"), b"0123456789").unwrap(); // 10 bytes each
        }
        let freed = clean_project_artifacts(
            &base,
            ProjectType::Next,
            &[".turbo".to_string()],
            CleanScope::Scheduled,
        )
        .unwrap();

        // The dependency store and build output survive a scheduled pass...
        assert!(
            base.join("node_modules").exists(),
            "scheduled clean must never remove node_modules"
        );
        assert!(
            base.join(".next").exists(),
            "scheduled clean must keep build output"
        );
        // ...while the regenerable caches are reclaimed.
        assert!(!base.join(".next/cache").exists());
        assert!(!base.join("node_modules/.cache").exists());
        assert!(!base.join(".turbo").exists());
        // 3 cache dirs × 10 bytes. (node_modules still holds its own 10-byte
        // file plus the now-removed .cache child.)
        assert_eq!(freed, 30);
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
