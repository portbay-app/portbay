//! dnsmasq-related commands: resolver-file install / uninstall /
//! status, plus sidecar restart.
//!
//! Resolver-install is the gate that makes dnsmasq actually answer
//! real queries. Until the user clicks the Settings → DNS button (or
//! invokes this from the CLI), the daemon runs harmlessly on
//! loopback and macOS never routes anything to it.

use serde::Serialize;
use tauri::{AppHandle, State};

use crate::dnsmasq::resolver;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolverStatus {
    /// The domain suffix this status reflects (matches
    /// `AppState::domain_suffix`).
    pub suffix: String,
    /// True iff `/etc/resolver/<suffix>` exists *and* references the
    /// current dnsmasq port. A stale file from an older boot (port
    /// mismatch) reads as `false`.
    pub installed: bool,
    /// Path of the resolver file we'd read or write.
    pub path: String,
    /// Whatever is currently in the file (for diagnostic display).
    /// `None` when the file is missing entirely.
    pub current_contents: Option<String>,
    /// Port the daemon is currently listening on, exposed so the
    /// settings UI can render "expected port: …" without re-querying.
    pub current_port: u16,
}

#[tauri::command]
pub async fn dnsmasq_resolver_status(state: State<'_, AppState>) -> AppResult<ResolverStatus> {
    let suffix = state.domain_suffix.clone();
    let port = state.dnsmasq.lock().expect("dnsmasq mutex poisoned").port();
    Ok(ResolverStatus {
        path: resolver::resolver_file_path(&suffix)
            .to_string_lossy()
            .into_owned(),
        installed: resolver::is_installed(&suffix, port),
        current_contents: resolver::read_installed(&suffix),
        current_port: port,
        suffix,
    })
}

#[tauri::command]
pub async fn dnsmasq_install_resolver(state: State<'_, AppState>) -> AppResult<()> {
    let suffix = state.domain_suffix.clone();
    let port = state.dnsmasq.lock().expect("dnsmasq mutex poisoned").port();

    // Run the osascript prompt off the async runtime — it blocks on
    // the macOS auth dialog and can take seconds (or never resolve if
    // the user walks away).
    let result =
        tokio::task::spawn_blocking(move || resolver::install_via_osascript(&suffix, port))
            .await
            .map_err(|e| AppError::Internal(format!("install join: {e}")))?;

    result.map_err(AppError::from)
}

#[tauri::command]
pub async fn dnsmasq_uninstall_resolver(state: State<'_, AppState>) -> AppResult<()> {
    let suffix = state.domain_suffix.clone();
    let result = tokio::task::spawn_blocking(move || resolver::uninstall_via_osascript(&suffix))
        .await
        .map_err(|e| AppError::Internal(format!("uninstall join: {e}")))?;
    result.map_err(AppError::from)
}

/// `restart_dnsmasq()` — stop the bundled dnsmasq sidecar and start
/// it again against a fresh config. Picked up by the dnsmasq card's
/// action button.
#[tauri::command]
pub async fn restart_dnsmasq(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    state.shutdown_dnsmasq();
    state.boot_dnsmasq(&app)
}
