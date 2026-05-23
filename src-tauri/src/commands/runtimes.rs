//! IPC surface for the Languages container.
//!
//! Detect-first: PortBay reuses runtimes already on the machine and never
//! bundles or copies one. This module adds the user-controlled surface on top
//! of detection:
//!
//!   - `list_runtimes()` — every language, detected + manually-added versions,
//!     with the per-language default marked.
//!   - `add_runtime_by_path(lang, path)` — register an existing binary the
//!     detector didn't find (e.g. a custom-compiled PHP). Reuses it in place.
//!   - `remove_runtime_path(lang, version)` — drop a manual entry.
//!   - `set_default_runtime(lang, version)` — set/clear the default version a
//!     new project inherits for that language.
//!
//! Deferred follow-up (same card): `install_runtime` via `brew` — the
//! on-request convenience that delegates to the user's package manager.

use std::path::PathBuf;

use tauri::State;

use crate::commands::projects::{load_registry, save_registry};
use crate::error::{AppError, AppResult};
use crate::registry::ManualRuntime;
use crate::runtimes::{self, major_minor, runtime_by_id, LanguageView};
use crate::state::AppState;

#[tauri::command]
pub async fn list_runtimes(state: State<'_, AppState>) -> AppResult<Vec<LanguageView>> {
    let reg = load_registry(&state)?;
    Ok(runtimes::list_all(&reg.runtimes.manual, &reg.runtimes.defaults))
}

/// Register an existing binary as a manual install for `lang`. PortBay probes
/// its version and reuses the binary in place — it is never copied.
#[tauri::command]
pub async fn add_runtime_by_path(
    state: State<'_, AppState>,
    lang: String,
    path: String,
) -> AppResult<Vec<LanguageView>> {
    let runtime = runtime_by_id(&lang)
        .ok_or_else(|| AppError::BadInput(format!("unknown language `{lang}`")))?;

    let binary = PathBuf::from(&path);
    if !binary.is_file() {
        return Err(AppError::BadInput(format!("no binary found at {path}")));
    }

    // Probe the version; a binary that doesn't report one isn't the runtime
    // the user thinks it is.
    let version = runtime.probe_version(&binary).ok_or_else(|| {
        AppError::BadInput(format!(
            "{path} didn't report a {lang} version — is it the right binary?"
        ))
    })?;
    let version = major_minor(&version);

    let mut reg = load_registry(&state)?;
    let canon = binary.canonicalize().unwrap_or_else(|_| binary.clone());
    let exists = reg.runtimes.manual.iter().any(|m| {
        m.binary.canonicalize().unwrap_or_else(|_| m.binary.clone()) == canon
    });
    if !exists {
        reg.runtimes.manual.push(ManualRuntime {
            lang: lang.clone(),
            version,
            binary,
        });
        save_registry(&state, &reg)?;
    }

    Ok(runtimes::list_all(&reg.runtimes.manual, &reg.runtimes.defaults))
}

/// Remove a manually-added install. No-op if it wasn't manual / not present.
#[tauri::command]
pub async fn remove_runtime_path(
    state: State<'_, AppState>,
    lang: String,
    version: String,
) -> AppResult<Vec<LanguageView>> {
    let mut reg = load_registry(&state)?;
    let before = reg.runtimes.manual.len();
    reg.runtimes
        .manual
        .retain(|m| !(m.lang == lang && m.version == version));
    if reg.runtimes.manual.len() != before {
        save_registry(&state, &reg)?;
    }
    Ok(runtimes::list_all(&reg.runtimes.manual, &reg.runtimes.defaults))
}

/// Set (or clear, when `version` is empty/None) the default version a new
/// project inherits for `lang`.
#[tauri::command]
pub async fn set_default_runtime(
    state: State<'_, AppState>,
    lang: String,
    version: Option<String>,
) -> AppResult<Vec<LanguageView>> {
    let mut reg = load_registry(&state)?;
    match version {
        Some(v) if !v.trim().is_empty() => {
            reg.runtimes.defaults.insert(lang, v);
        }
        _ => {
            reg.runtimes.defaults.remove(&lang);
        }
    }
    save_registry(&state, &reg)?;
    Ok(runtimes::list_all(&reg.runtimes.manual, &reg.runtimes.defaults))
}
