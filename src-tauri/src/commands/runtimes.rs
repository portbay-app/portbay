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
//!   - `install_runtime(lang)` — the on-request convenience that delegates to
//!     Homebrew, streaming `brew install`'s output to the UI. PortBay still
//!     never bundles a runtime; this just runs the command we'd otherwise tell
//!     the user to run by hand.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Serialize;
use tauri::ipc::Channel;
use tauri::{AppHandle, State};
use tauri_plugin_shell::process::CommandEvent;
use tauri_plugin_shell::ShellExt;

use crate::commands::projects::{load_registry, save_registry};
use crate::error::{AppError, AppResult};
use crate::registry::ManualRuntime;
use crate::runtimes::{self, major_minor, runtime_by_id, LanguageView};
use crate::state::AppState;

#[tauri::command]
pub async fn list_runtimes(state: State<'_, AppState>) -> AppResult<Vec<LanguageView>> {
    let reg = load_registry(&state)?;
    Ok(runtimes::list_all(&reg.runtimes))
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
    let exists = reg
        .runtimes
        .manual
        .iter()
        .any(|m| m.binary.canonicalize().unwrap_or_else(|_| m.binary.clone()) == canon);
    if !exists {
        reg.runtimes.manual.push(ManualRuntime {
            lang: lang.clone(),
            version,
            binary,
        });
        save_registry(&state, &reg)?;
    }

    Ok(runtimes::list_all(&reg.runtimes))
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
    Ok(runtimes::list_all(&reg.runtimes))
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
    Ok(runtimes::list_all(&reg.runtimes))
}

/// Progress events for [`install_runtime`], streamed over the `Channel`.
/// `done` carries the final success flag so the UI can settle on a clean exit;
/// on failure the command also returns `Err`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum InstallEvent {
    Log { line: String },
    Done { success: bool },
}

/// Install a runtime by delegating to Homebrew. PortBay never bundles a
/// runtime — this runs the same `brew install <formula>` the sidebar otherwise
/// shows as a hint, streaming brew's stdout/stderr to the UI line by line. The
/// formula comes from the runtime's own [`brew_formula`], so the action stays
/// in sync with the hint. The frontend re-lists runtimes on success so the new
/// version appears without a manual refresh.
///
/// [`brew_formula`]: crate::runtimes::LanguageRuntime::brew_formula
#[tauri::command]
pub async fn install_runtime(
    app: AppHandle,
    lang: String,
    on_event: Channel<InstallEvent>,
) -> AppResult<()> {
    let runtime = runtime_by_id(&lang)
        .ok_or_else(|| AppError::BadInput(format!("unknown language `{lang}`")))?;
    let formula = runtime.brew_formula().ok_or_else(|| {
        AppError::BadInput(format!(
            "{} can't be installed via Homebrew automatically",
            runtime.display_name()
        ))
    })?;
    drop(runtime);

    let _ = on_event.send(InstallEvent::Log {
        line: format!("$ brew install {formula}"),
    });

    let (mut rx, _child) = app
        .shell()
        .command("brew")
        .args(["install", &formula])
        .spawn()
        .map_err(|e| {
            AppError::Internal(format!(
                "couldn't start Homebrew — is `brew` installed and on PATH? ({e})"
            ))
        })?;

    // brew streams progress to stderr and results to stdout; forward both.
    let mut exit_code: Option<i32> = None;
    while let Some(event) = rx.recv().await {
        match event {
            CommandEvent::Stdout(bytes) | CommandEvent::Stderr(bytes) => {
                let line = String::from_utf8_lossy(&bytes).trim_end().to_string();
                if !line.is_empty() {
                    let _ = on_event.send(InstallEvent::Log { line });
                }
            }
            CommandEvent::Terminated(payload) => {
                exit_code = payload.code;
            }
            _ => {}
        }
    }

    let success = exit_code == Some(0);
    let _ = on_event.send(InstallEvent::Done { success });
    if !success {
        return Err(AppError::Internal(format!(
            "`brew install {formula}` exited with code {exit_code:?}"
        )));
    }
    Ok(())
}

/// Apply edits from an editable config tab (e.g. PHP's FPM / php.ini tabs).
/// `patches` maps each dirty row's `key` to its new string value. The change
/// is validated + persisted into the registry, then any services it affects
/// (the version's FPM pool) are restarted best-effort so it takes effect now.
/// Returns the refreshed language list so the panel re-renders saved values.
#[tauri::command]
pub async fn update_runtime_config(
    state: State<'_, AppState>,
    lang: String,
    version: String,
    tab_id: String,
    patches: BTreeMap<String, String>,
) -> AppResult<Vec<LanguageView>> {
    let runtime = runtime_by_id(&lang)
        .ok_or_else(|| AppError::BadInput(format!("unknown language `{lang}`")))?;

    let mut reg = load_registry(&state)?;
    let result = runtime
        .apply_config(&version, &tab_id, &patches, &mut reg.runtimes)
        .map_err(AppError::BadInput)?;
    save_registry(&state, &reg)?;

    // Restart affected services so the new config is live immediately. A
    // process that isn't currently running (no project uses this version yet)
    // simply errors out — ignored; the next reconcile tick will pick the
    // config up when the pool first starts.
    if let Ok(client) = state.pc_client() {
        for pid in &result.restart_processes {
            let _ = client.restart(pid).await;
        }
    }

    Ok(runtimes::list_all(&reg.runtimes))
}
