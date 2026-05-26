//! Auto-update commands — thin wrappers over `tauri-plugin-updater`'s Rust API.
//!
//! The frontend stays declarative: `check_for_update` reports whether a newer
//! release is published (reading the GitHub-hosted `latest.json` configured in
//! `tauri.conf.json::plugins.updater`), and `install_update` downloads +
//! verifies the minisign signature + installs it, then relaunches into the new
//! version. Both flow through the standard `AppError` envelope so the existing
//! toast / ErrorEnvelope path renders failures with no special-casing.
//!
//! Keeping the flow Rust-side means the "Update now" toast action is just a
//! `safeInvoke("install_update")` like every other command button.

use serde::Serialize;
use tauri_plugin_updater::UpdaterExt;

use crate::error::{AppError, AppResult};

/// What the frontend needs to render the "update available" toast and the
/// Settings → Updates row. A `None` return from [`check_for_update`] means the
/// running build is already current.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    /// Version offered by the manifest (e.g. `"0.2.0"`).
    pub version: String,
    /// Version currently running.
    pub current_version: String,
    /// Release notes from the manifest, if the producer set them.
    pub notes: Option<String>,
    /// Publish date from the manifest, if present.
    pub pub_date: Option<String>,
}

/// Check the configured endpoint for a newer signed release. Returns `None`
/// when up to date. Network / parse / signature errors surface as an envelope.
#[tauri::command]
pub async fn check_for_update(app: tauri::AppHandle) -> AppResult<Option<UpdateInfo>> {
    let updater = app
        .updater()
        .map_err(|e| AppError::Internal(format!("updater unavailable: {e}")))?;

    match updater.check().await {
        Ok(Some(update)) => Ok(Some(UpdateInfo {
            version: update.version.clone(),
            current_version: update.current_version.clone(),
            notes: update.body.clone(),
            pub_date: update.date.map(|d| d.to_string()),
        })),
        Ok(None) => Ok(None),
        Err(e) => Err(AppError::Internal(format!("update check failed: {e}"))),
    }
}

/// Download, verify, and install the latest update, then relaunch. The plugin
/// rejects any package whose signature doesn't match the configured pubkey, so
/// a tampered binary fails here rather than running. Never returns on success —
/// `app.restart()` replaces the process.
#[tauri::command]
pub async fn install_update(app: tauri::AppHandle) -> AppResult<()> {
    let updater = app
        .updater()
        .map_err(|e| AppError::Internal(format!("updater unavailable: {e}")))?;

    let update = updater
        .check()
        .await
        .map_err(|e| AppError::Internal(format!("update check failed: {e}")))?
        .ok_or_else(|| AppError::Internal("no update available to install".into()))?;

    update
        .download_and_install(|_chunk_len, _content_len| {}, || {})
        .await
        .map_err(|e| AppError::Internal(format!("update install failed: {e}")))?;

    app.restart()
}
