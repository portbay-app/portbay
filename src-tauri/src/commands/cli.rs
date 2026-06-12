//! Tauri commands for installing the bundled `portbay` CLI onto the user's
//! PATH. Backs the "Command-line tool" row in Advanced settings. The frontend
//! passes the desired install path (the `cliPath` preference, default
//! `/usr/local/bin/portbay`).

use std::path::PathBuf;

use crate::cli_install::{self, CliStatus};
use crate::error::{AppError, AppResult};

/// Report whether the CLI is installed at `install_path`, whether it points at
/// this app's bundled binary, and whether its directory is on `$PATH`.
#[tauri::command]
pub async fn cli_status(install_path: String) -> AppResult<CliStatus> {
    let path = PathBuf::from(install_path);
    tokio::task::spawn_blocking(move || cli_install::status(&path))
        .await
        .map_err(|e| AppError::Internal(format!("cli status join: {e}")))
}

/// Symlink the bundled CLI to `install_path`. May show one OS authorization
/// prompt if the directory isn't user-writable. Returns the bundle path linked.
#[tauri::command]
pub async fn cli_install_tool(install_path: String) -> AppResult<String> {
    let path = PathBuf::from(install_path);
    tokio::task::spawn_blocking(move || cli_install::install(&path))
        .await
        .map_err(|e| AppError::Internal(format!("cli install join: {e}")))?
        .map_err(AppError::Internal)
}

/// Remove the installed CLI symlink at `install_path`. Idempotent.
#[tauri::command]
pub async fn cli_uninstall_tool(install_path: String) -> AppResult<()> {
    let path = PathBuf::from(install_path);
    tokio::task::spawn_blocking(move || cli_install::uninstall(&path))
        .await
        .map_err(|e| AppError::Internal(format!("cli uninstall join: {e}")))?
        .map_err(AppError::Internal)
}
