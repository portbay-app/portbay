//! User-preference IPC surface.
//!
//! Three commands:
//! - `get_preferences()` — return the current snapshot to the frontend
//!   on mount of the Settings page.
//! - `set_preferences(prefs)` — overwrite the persisted prefs and apply
//!   any side effects (toggle tray visibility live).
//! - `mark_close_toast_seen()` — set the first-run "still running"
//!   toast flag so it doesn't fire again.

use tauri::{AppHandle, State};

use crate::domain::{migrate_registry_suffix, DomainMigration};
use crate::error::{AppError, AppResult};
use crate::preferences::Preferences;
use crate::registry::store;
use crate::state::AppState;
use crate::tray;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainSettings {
    pub domain_suffix: String,
    pub project_count: usize,
}

#[tauri::command]
pub async fn get_preferences(state: State<'_, AppState>) -> AppResult<Preferences> {
    Ok(state.preferences_snapshot())
}

/// Replace the persisted preferences and reconcile any UI side effects.
///
/// Side effects, applied in order:
/// 1. Persist to disk (fails-loudly so the frontend can show a toast).
/// 2. If the tray visibility toggled, install or uninstall it now —
///    no app restart required.
#[tauri::command]
pub async fn set_preferences(
    app: AppHandle,
    state: State<'_, AppState>,
    prefs: Preferences,
) -> AppResult<Preferences> {
    let previous = state.preferences_snapshot();

    let mut prefs = prefs;
    // Starting (or restarting) the auto-clean clock: when the cadence flips on
    // from "off" — or was never stamped — anchor `last_auto_clean` to now so
    // the first automatic pass is one full cadence away, never an immediate
    // surprise wipe the moment the toggle is enabled.
    if prefs.auto_clean_schedule != "off"
        && (previous.auto_clean_schedule == "off" || prefs.last_auto_clean == 0)
    {
        prefs.last_auto_clean = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
    }

    // Persist first; only then commit to in-memory state so a disk
    // failure leaves the running app coherent with what's on disk.
    prefs
        .save()
        .map_err(|e| AppError::Internal(format!("failed to save preferences: {e}")))?;

    {
        let mut guard = state
            .preferences
            .lock()
            .expect("preferences mutex poisoned");
        *guard = prefs.clone();
    }

    if previous.show_tray_icon != prefs.show_tray_icon {
        if prefs.show_tray_icon {
            if let Err(e) = tray::install(&app) {
                tracing::warn!(error = %e, "tray install failed");
            }
        } else {
            tray::uninstall(&app);
        }
    }

    Ok(prefs)
}

#[tauri::command]
pub async fn get_domain_settings(state: State<'_, AppState>) -> AppResult<DomainSettings> {
    let registry = store::load_or_default(&state.registry_path, &state.domain_suffix)?;
    Ok(DomainSettings {
        domain_suffix: registry.domain_suffix,
        project_count: registry.projects.len(),
    })
}

#[tauri::command]
pub async fn update_domain_suffix(
    state: State<'_, AppState>,
    domain_suffix: String,
) -> AppResult<DomainMigration> {
    let mut registry = store::load_or_default(&state.registry_path, &state.domain_suffix)?;
    let certs_root = certs_root();
    let migration = migrate_registry_suffix(&mut registry, &domain_suffix, certs_root)
        .map_err(|e| AppError::BadInput(e.to_string()))?;
    store::save_to(&registry, &state.registry_path)?;
    state.reconciler.mark_dirty();
    Ok(migration)
}

#[tauri::command]
pub async fn mark_close_toast_seen(state: State<'_, AppState>) -> AppResult<()> {
    let mut updated = state.preferences_snapshot();
    if updated.close_to_menu_bar_toast_seen {
        return Ok(());
    }
    updated.close_to_menu_bar_toast_seen = true;
    updated
        .save()
        .map_err(|e| AppError::Internal(format!("failed to save preferences: {e}")))?;
    *state
        .preferences
        .lock()
        .expect("preferences mutex poisoned") = updated;
    Ok(())
}

fn certs_root() -> Option<std::path::PathBuf> {
    let mut dir = dirs::data_dir()?;
    dir.push("PortBay");
    dir.push("certs");
    Some(dir)
}
