//! Migration-import commands (thin GUI wrappers).
//!
//! The site→`Project` mapping and the preview/collision logic live in
//! `crate::import` so the GUI, the `portbay import` CLI, and the MCP `Migrate`
//! toolset all share one implementation and can't drift. These commands just
//! load the registry, call the shared functions, and (on a successful import)
//! save + mark the reconciler dirty.
//!
//! - `detect_sources()` lists which source tools (Herd, ServBay, MAMP) are
//!   installed locally and how many sites they expose.
//! - `preview_import(source)` returns each site plus id/path collision flags.
//! - `import_projects(source, ids)` imports the chosen sites into the registry.

use tauri::State;

use crate::commands::projects::{load_registry, save_registry};
use crate::error::{AppError, AppResult};
use crate::import::{self, DetectedSource, ImportPreviewRow, ImportResult, ImportSource};
use crate::state::AppState;

#[tauri::command]
pub async fn detect_sources() -> AppResult<Vec<DetectedSource>> {
    Ok(import::detect_all())
}

#[tauri::command]
pub async fn preview_import(
    state: State<'_, AppState>,
    source: ImportSource,
) -> AppResult<Vec<ImportPreviewRow>> {
    let registry = load_registry(&state)?;
    import::preview(source, &registry).map_err(|e| AppError::Internal(e.to_string()))
}

#[tauri::command]
pub async fn import_projects(
    state: State<'_, AppState>,
    source: ImportSource,
    ids: Vec<String>,
) -> AppResult<ImportResult> {
    let mut registry = load_registry(&state)?;
    let result = import::import_selected(source, &ids, &mut registry)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    if !result.imported.is_empty() {
        save_registry(&state, &registry)?;
        state.reconciler.mark_dirty();
    }
    Ok(result)
}
