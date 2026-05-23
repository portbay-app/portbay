//! PHP commands — version detection + Xdebug toggle.
//!
//! `list_php_installs` powers the /php route and the detail panel's
//! version picker. `set_xdebug_mode` flips the project's
//! `XDEBUG_MODE` env var (the user toggles enable/disable; the
//! reconciler re-spawns the project's PC entry on save).
//!
//! No installer here — missing versions are surfaced with the
//! `brew install` hint the frontend already shows. A bundled
//! installer is a follow-up card once we sign releases.

use std::collections::BTreeMap;

use tauri::State;

use crate::commands::dto::ProjectView;
use crate::commands::projects::{load_registry, save_registry};
use crate::error::{AppError, AppResult};
use crate::php::{self, PhpInstall};
use crate::registry::ProjectId;
use crate::state::AppState;

#[tauri::command]
pub async fn list_php_installs() -> AppResult<Vec<PhpInstall>> {
    Ok(php::detect_all())
}

/// `set_xdebug_mode(id, mode)` — flip a project's `XDEBUG_MODE` env
/// var. Passing `"off"` deletes the var entirely; any other value
/// sets it. The patch goes through `update_project` so the existing
/// dirty-and-reconcile flow runs.
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

    Ok(crate::commands::dto::ProjectView::from_project(
        &snapshot, None,
    ))
}
