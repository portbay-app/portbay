//! Remote exec + deploy commands over a saved SSH connection.
//!
//! The frontend shows the exact commands and the user explicitly clicks Run —
//! that click is the approval step for executing on a remote host. Output +
//! per-step exit codes come back for display.

use serde::Deserialize;
use tauri::{AppHandle, State};

use crate::commands::projects::load_registry;
use crate::commands::ssh_tunnels::{
    load_stored_key_passphrase, load_stored_password, load_stored_proxy_password,
};
use crate::error::{AppError, AppResult};
use crate::registry::{SshConnection, SshConnectionId};
use crate::ssh::exec::{exec_on, run_deploy_on, ExecResult, StepResult};
use crate::state::AppState;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshExecInput {
    pub connection_id: String,
    pub command: String,
    #[serde(default)]
    pub cwd: Option<String>,
    /// One-shot password from the credential prompt, used for this connect only
    /// and never stored. Blank/absent falls back to a keychain-saved password.
    #[serde(default)]
    pub password: Option<String>,
    /// One-shot key passphrase from the credential prompt; one-connect only.
    #[serde(default)]
    pub passphrase: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshDeployInput {
    pub connection_id: String,
    pub steps: Vec<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    /// One-shot password from the credential prompt; this run only, never stored.
    #[serde(default)]
    pub password: Option<String>,
    /// One-shot key passphrase from the credential prompt; this run only.
    #[serde(default)]
    pub passphrase: Option<String>,
}

struct ConnContext {
    conn: SshConnection,
    password: Option<String>,
    proxy_password: Option<String>,
    passphrase: Option<String>,
}

/// Resolve a connection and its secrets. `password_override` / `passphrase_override`
/// are one-shot secrets from the credential prompt: when present (non-blank)
/// they're used for this connect and never persisted, taking precedence over
/// any keychain-saved value. Blank/absent falls back to the keychain.
fn connection(
    state: &State<'_, AppState>,
    id: &str,
    password_override: Option<String>,
    passphrase_override: Option<String>,
) -> AppResult<ConnContext> {
    // Dev-only diagnostic (compiled out of release builds): did the inline
    // (prompted) password arrive over IPC? Presence only — never the secret.
    #[cfg(debug_assertions)]
    tracing::info!(
        connection = %id,
        password_override_present = password_override.as_deref().map(str::trim).is_some_and(|s| !s.is_empty()),
        passphrase_override_present = passphrase_override.as_deref().map(str::trim).is_some_and(|s| !s.is_empty()),
        "ssh_exec: resolving connection secrets"
    );
    let registry = load_registry(state)?;
    let raw = registry
        .get_ssh_connection(&SshConnectionId::new(id))
        .ok_or_else(|| AppError::BadInput(format!("SSH connection `{id}` not found")))?;
    // Fold in a borrowed identity (user / key / auth) before connecting.
    let conn = registry.effective_ssh_connection(raw);
    let nonblank = |s: Option<String>| s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    let password = match nonblank(password_override) {
        Some(p) => Some(p),
        None => load_stored_password(&conn.id)?,
    };
    let passphrase = match passphrase_override {
        Some(ref s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        // An explicit *empty* override means the user chose "Skip" on the
        // passphrase prompt: forward it as a declined passphrase (`Some("")`)
        // so the backend skips the key and asks for the password instead of
        // re-prompting — and don't silently fall back to a stored passphrase.
        Some(_) => Some(String::new()),
        None => load_stored_key_passphrase(&conn.id)?,
    };
    let proxy_password = load_stored_proxy_password(&conn.id)?;
    Ok(ConnContext {
        conn,
        password,
        proxy_password,
        passphrase,
    })
}

/// Run one command on the remote host. Captures stdout/stderr + exit code.
#[tauri::command]
pub async fn ssh_exec_run(
    state: State<'_, AppState>,
    app: AppHandle,
    input: SshExecInput,
) -> AppResult<ExecResult> {
    let cx = connection(
        &state,
        &input.connection_id,
        input.password,
        input.passphrase,
    )?;
    // Reuse (or open) the host's cached exec session, releasing the manager
    // lock before running the command so a long command doesn't block other
    // exec/deploy calls on other hosts.
    let session = {
        let mut mgr = state.exec.lock().await;
        mgr.session_for(
            &cx.conn,
            cx.password.as_deref(),
            cx.proxy_password.as_deref(),
            cx.passphrase.as_deref(),
            Some(crate::ssh::EventInteractor::new(app)),
        )
        .await
        .map_err(AppError::Ssh)?
    };
    exec_on(&session, &input.command, input.cwd.as_deref())
        .await
        .map_err(AppError::Ssh)
}

/// Run an ordered deploy sequence; stops at the first failing step.
#[tauri::command]
pub async fn ssh_deploy_run(
    state: State<'_, AppState>,
    app: AppHandle,
    input: SshDeployInput,
) -> AppResult<Vec<StepResult>> {
    if input.steps.iter().all(|s| s.trim().is_empty()) {
        return Err(AppError::BadInput("add at least one deploy command".into()));
    }
    let steps: Vec<String> = input
        .steps
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let cx = connection(
        &state,
        &input.connection_id,
        input.password,
        input.passphrase,
    )?;
    let session = {
        let mut mgr = state.exec.lock().await;
        mgr.session_for(
            &cx.conn,
            cx.password.as_deref(),
            cx.proxy_password.as_deref(),
            cx.passphrase.as_deref(),
            Some(crate::ssh::EventInteractor::new(app)),
        )
        .await
        .map_err(AppError::Ssh)?
    };
    run_deploy_on(&session, &steps, input.cwd.as_deref())
        .await
        .map_err(AppError::Ssh)
}
