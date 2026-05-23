//! IPC surface for the Languages container.
//!
//! Single command for v1: `list_runtimes()` returns every language
//! PortBay knows about with each detected version and its config
//! tabs in one round-trip. The frontend renders the whole panel from
//! that one payload — no per-language re-fetch on selection change.
//!
//! Follow-up commits on the same kanban card add:
//!   - `install_runtime(lang, version)` — delegates to brew/asdf/mise
//!   - `set_runtime_default(lang, version)` — for the Settings card
//!   - `open_runtime_config(lang, version)` — Reveal in Finder

use crate::error::AppResult;
use crate::runtimes::{self, LanguageView};

#[tauri::command]
pub async fn list_runtimes() -> AppResult<Vec<LanguageView>> {
    Ok(runtimes::list_all())
}
