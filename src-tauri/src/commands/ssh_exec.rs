//! Remote exec + deploy commands over a saved SSH connection.
//!
//! The frontend shows the exact commands and the user explicitly clicks Run —
//! that click is the approval step for executing on a remote host. Output +
//! per-step exit codes come back for display.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::commands::projects::load_registry;
use crate::commands::ssh_tunnels::{
    load_stored_key_passphrase, load_stored_password, load_stored_proxy_password,
};
use crate::error::{AppError, AppResult};
use crate::registry::{SshConnection, SshConnectionId};
use crate::ssh::exec::{exec_on, run_deploy_on_streaming, DeployProgress, ExecResult, StepResult};
use crate::ssh::secret::{nonblank_secret, secret_str, SecretString};
use crate::state::AppState;

/// Channel for live deploy progress (ad-hoc and project deploys both emit
/// here); payloads are [`DeployEvent`]s keyed by the caller-supplied `runId`.
pub const DEPLOY_CHANNEL: &str = "portbay://deploy";

/// One live-progress event from a running deploy. `kind` tags the variant for
/// the frontend (`sync` / `stepStarted` / `output` / `stepDone`).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DeployEvent {
    /// Project-deploy file sync progress (uploads done so far / total).
    #[serde(rename_all = "camelCase")]
    Sync {
        run_id: String,
        uploaded: u32,
        total: u32,
        bytes: u64,
    },
    #[serde(rename_all = "camelCase")]
    StepStarted {
        run_id: String,
        index: usize,
        command: String,
    },
    #[serde(rename_all = "camelCase")]
    Output {
        run_id: String,
        index: usize,
        stderr: bool,
        chunk: String,
    },
    #[serde(rename_all = "camelCase")]
    StepDone {
        run_id: String,
        index: usize,
        exit_code: i32,
        duration_ms: u64,
    },
}

/// In-flight deploy cancel flags, keyed by run id (same pattern as the SFTP
/// search cancel registry).
fn cancel_registry() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    static REG: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(HashMap::new()))
}

/// RAII registration of a run's cancel flag: created when a streaming deploy
/// starts, deregistered on drop so an early `?` return can't leak a stale flag.
pub struct DeployCancelGuard {
    id: String,
    flag: Arc<AtomicBool>,
}

impl DeployCancelGuard {
    pub fn new(id: &str) -> Self {
        let flag = Arc::new(AtomicBool::new(false));
        if let Ok(mut reg) = cancel_registry().lock() {
            reg.insert(id.to_string(), flag.clone());
        }
        Self {
            id: id.to_string(),
            flag,
        }
    }

    pub fn flag(&self) -> Arc<AtomicBool> {
        self.flag.clone()
    }

    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
}

impl Drop for DeployCancelGuard {
    fn drop(&mut self) {
        if let Ok(mut reg) = cancel_registry().lock() {
            reg.remove(&self.id);
        }
    }
}

/// Map a [`DeployProgress`] from the exec layer onto a [`DeployEvent`] and
/// emit it; shared by the ad-hoc and project deploy commands.
pub fn emit_deploy_progress(app: &AppHandle, run_id: &str, p: DeployProgress) {
    let ev = match p {
        DeployProgress::StepStarted { index, command } => DeployEvent::StepStarted {
            run_id: run_id.to_string(),
            index,
            command,
        },
        DeployProgress::Output {
            index,
            stderr,
            chunk,
        } => DeployEvent::Output {
            run_id: run_id.to_string(),
            index,
            stderr,
            chunk,
        },
        DeployProgress::StepDone {
            index,
            exit_code,
            duration_ms,
        } => DeployEvent::StepDone {
            run_id: run_id.to_string(),
            index,
            exit_code,
            duration_ms,
        },
    };
    // Deploy output is raw remote stdout/stderr — main window only, never
    // broadcast (see `events::emit_to_main`).
    let _ = crate::commands::events::emit_to_main(app, DEPLOY_CHANNEL, ev);
}

/// Flag an in-flight deploy run for cancellation. Queued steps are skipped and
/// the running command gets a best-effort SIGTERM; the run still resolves with
/// the results gathered so far.
#[tauri::command]
pub async fn ssh_deploy_cancel(run_id: String) -> AppResult<()> {
    if let Ok(reg) = cancel_registry().lock() {
        if let Some(flag) = reg.get(&run_id) {
            flag.store(true, Ordering::SeqCst);
        }
    }
    Ok(())
}

/// One saved deploy snippet (a recallable command sequence). Shape mirrors
/// `DeploySnippet` in `src/lib/stores/deploySnippets.svelte.ts` — the frontend
/// owns the semantics; the backend only persists.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploySnippet {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub cwd: String,
    pub steps: Vec<String>,
}

/// Saved deploy snippets keyed by SSH connection id.
pub type DeploySnippetMap = std::collections::BTreeMap<String, Vec<DeploySnippet>>;

/// On-disk home for saved deploy snippets, next to `preferences.json`.
///
/// These used to live in webview localStorage, but WKWebView keys its storage
/// by bundle identity — a dev binary running unbundled vs. wrapped in a signed
/// .app lands in different containers and the snippets "vanish". A plain file
/// under the PortBay data dir survives all of that.
fn deploy_snippets_path() -> std::io::Result<std::path::PathBuf> {
    let mut dir = dirs::data_dir()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no platform data dir"))?;
    dir.push("PortBay");
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("ssh-deploy-snippets.json"))
}

/// Load all saved deploy snippets. Missing file or parse failure → empty map;
/// the deploy pane must render even if the file is corrupted by a disk fault.
#[tauri::command]
pub async fn ssh_deploy_snippets_get() -> AppResult<DeploySnippetMap> {
    let Ok(path) = deploy_snippets_path() else {
        return Ok(DeploySnippetMap::new());
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Ok(DeploySnippetMap::new());
    };
    match serde_json::from_str::<DeploySnippetMap>(&raw) {
        Ok(map) => Ok(map),
        Err(e) => {
            tracing::warn!(error = %e, path = %path.display(), "deploy snippets file corrupt — starting empty");
            Ok(DeploySnippetMap::new())
        }
    }
}

/// Replace the persisted snippet map. Atomic write (temp + rename) so a crash
/// mid-write can't truncate the user's saved snippets.
#[tauri::command]
pub async fn ssh_deploy_snippets_set(snippets: DeploySnippetMap) -> AppResult<()> {
    let path = deploy_snippets_path()
        .map_err(|e| AppError::Internal(format!("failed to resolve snippets path: {e}")))?;
    let serialised = serde_json::to_vec_pretty(&snippets)
        .map_err(|e| AppError::Internal(format!("failed to serialise snippets: {e}")))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &serialised)
        .and_then(|()| std::fs::rename(&tmp, &path))
        .map_err(|e| AppError::Internal(format!("failed to save deploy snippets: {e}")))?;
    Ok(())
}

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
    /// Caller-generated id for live progress events + cancellation. Absent →
    /// no streaming (legacy await-the-result behaviour).
    #[serde(default)]
    pub run_id: Option<String>,
    /// One-shot password from the credential prompt; this run only, never stored.
    #[serde(default)]
    pub password: Option<String>,
    /// One-shot key passphrase from the credential prompt; this run only.
    #[serde(default)]
    pub passphrase: Option<String>,
}

struct ConnContext {
    conn: SshConnection,
    password: Option<SecretString>,
    proxy_password: Option<SecretString>,
    passphrase: Option<SecretString>,
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
    let password = match nonblank_secret(password_override) {
        Some(p) => Some(p),
        None => load_stored_password(&conn.id)?,
    };
    let passphrase = match passphrase_override {
        Some(ref s) if !s.trim().is_empty() => Some(SecretString::new(s.trim().to_string())),
        // An explicit *empty* override means the user chose "Skip" on the
        // passphrase prompt: forward it as a declined passphrase (`Some("")`)
        // so the backend skips the key and asks for the password instead of
        // re-prompting — and don't silently fall back to a stored passphrase.
        Some(_) => Some(SecretString::new(String::new())),
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
            secret_str(&cx.password),
            secret_str(&cx.proxy_password),
            secret_str(&cx.passphrase),
            Some(crate::ssh::EventInteractor::shared(app)),
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
            secret_str(&cx.password),
            secret_str(&cx.proxy_password),
            secret_str(&cx.passphrase),
            Some(crate::ssh::EventInteractor::shared(app.clone())),
        )
        .await
        .map_err(AppError::Ssh)?
    };
    // With a run id, stream progress events + honour cancellation; the
    // returned results stay authoritative either way.
    let guard = input.run_id.as_deref().map(DeployCancelGuard::new);
    let cancel = guard.as_ref().map(|g| g.flag());
    run_deploy_on_streaming(&session, &steps, input.cwd.as_deref(), cancel, |p| {
        if let Some(id) = input.run_id.as_deref() {
            emit_deploy_progress(&app, id, p);
        }
    })
    .await
    .map_err(AppError::Ssh)
}
