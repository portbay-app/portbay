//! Project → host deploy: sync a project's files to its configured SSH host
//! over the cached SFTP session, then run the configured build/release steps
//! over the cached exec session.
//!
//! The upload is whole-file (read into memory, then write) like the rest of the
//! SFTP layer — simple and robust for source/build trees. Both the SFTP and
//! exec sessions are the same per-connection cached sessions the workspace
//! already uses, so a deploy launched from inside the IDE doesn't
//! re-authenticate. The steps run from `remote_path` and stop at the first
//! non-zero exit, exactly like an ad-hoc deploy.

use std::collections::BTreeSet;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::commands::localfs::walk_files;
use crate::commands::projects::load_registry;
use crate::commands::ssh_exec::{
    emit_deploy_progress, DeployCancelGuard, DeployEvent, DEPLOY_CHANNEL,
};
use crate::commands::ssh_tunnels::{
    load_stored_key_passphrase, load_stored_password, load_stored_proxy_password,
};
use crate::error::{AppError, AppResult};
use crate::registry::{ProjectId, SshConnection, SshConnectionId};
use crate::ssh::exec::{run_deploy_on_streaming, StepResult};
use crate::ssh::secret::{nonblank_secret, secret_str, SecretString};
use crate::state::AppState;
use russh_sftp::client::SftpSession;
use tokio::io::AsyncWriteExt;

/// Per-file upload ceiling (1 GiB), matching the SFTP file-manager limit. A
/// single file above this is skipped with a note rather than buffered whole.
const MAX_FILE_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDeployRunInput {
    pub project_id: String,
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

/// Outcome of a deploy run: what was synced plus the per-step results.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployRunResult {
    /// Number of files uploaded.
    pub uploaded: u32,
    /// Total bytes uploaded.
    pub bytes: u64,
    /// Files skipped because they exceeded the per-file size ceiling.
    pub skipped: Vec<String>,
    /// The remote directory files were synced into.
    pub remote_path: String,
    /// Per-step output from the configured build/release commands.
    pub steps: Vec<StepResult>,
    /// True when the run was cancelled (sync stopped early and/or steps were
    /// skipped); whatever completed is still reported above.
    pub cancelled: bool,
}

/// Join a POSIX directory + relative child path (the remote side is POSIX).
fn posix_join(dir: &str, rel: &str) -> String {
    let dir = dir.trim_end_matches('/');
    if dir.is_empty() {
        format!("/{rel}")
    } else {
        format!("{dir}/{rel}")
    }
}

/// Resolve the secrets for a connection, mirroring `ssh_exec`'s resolver:
/// non-blank overrides win and are never stored; an explicit empty passphrase
/// is forwarded as "declined"; blank/absent falls back to the keychain.
fn resolve_secrets(
    conn: &SshConnection,
    password_override: Option<String>,
    passphrase_override: Option<String>,
) -> AppResult<(
    Option<SecretString>,
    Option<SecretString>,
    Option<SecretString>,
)> {
    let password = match nonblank_secret(password_override) {
        Some(p) => Some(p),
        None => load_stored_password(&conn.id)?,
    };
    let passphrase = match passphrase_override {
        Some(ref s) if !s.trim().is_empty() => Some(SecretString::new(s.trim().to_string())),
        Some(_) => Some(SecretString::new(String::new())),
        None => load_stored_key_passphrase(&conn.id)?,
    };
    let proxy_password = load_stored_proxy_password(&conn.id)?;
    Ok((password, proxy_password, passphrase))
}

/// Run the configured deploy for a project: sync files, then run steps.
#[tauri::command]
pub async fn project_deploy_run(
    state: State<'_, AppState>,
    app: AppHandle,
    input: ProjectDeployRunInput,
) -> AppResult<DeployRunResult> {
    let registry = load_registry(&state)?;
    let project = registry
        .get_project(&ProjectId::new(input.project_id.clone()))
        .ok_or_else(|| AppError::NotFound(input.project_id.clone()))?;
    let deploy = project
        .deploy
        .clone()
        .filter(|d| d.is_active())
        .ok_or_else(|| {
            AppError::BadInput("this project has no deploy host + remote path set".into())
        })?;

    // Resolve the deploy's target connection (folding in any borrowed identity).
    let raw = registry
        .get_ssh_connection(&SshConnectionId::new(deploy.connection_id.as_str()))
        .ok_or_else(|| {
            AppError::BadInput(format!(
                "deploy host `{}` no longer exists",
                deploy.connection_id
            ))
        })?;
    let conn = registry.effective_ssh_connection(raw);
    let (password, proxy_password, passphrase) =
        resolve_secrets(&conn, input.password, input.passphrase)?;

    // Local source root: the project folder, optionally narrowed to a subdir.
    let mut local_root = project.path.clone();
    if let Some(sub) = deploy
        .local_subdir
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        local_root = local_root.join(sub);
    }
    let files = walk_files(&local_root, &deploy.exclude)?;

    let remote_root = deploy.remote_path.trim().to_string();

    // One host-key interactor for both legs of the deploy (SFTP upload + exec
    // steps); only the first cold connect actually prompts.
    let interactor = Some(crate::ssh::EventInteractor::shared(app.clone()));

    // Open (or reuse) the cached SFTP session for the upload.
    let sftp = {
        let mut mgr = state.sftp.lock().await;
        mgr.session_for(
            &conn,
            secret_str(&password),
            secret_str(&proxy_password),
            secret_str(&passphrase),
            interactor.clone(),
        )
        .await
        .map_err(AppError::Ssh)?
    };

    // Collect every remote directory the upload needs (the root + each file's
    // intermediate dirs), then create them parents-first, ignoring "already
    // exists" errors. A genuinely un-creatable dir surfaces later as a failed
    // file write with a clear path. `BTreeSet` dedups; sorting by component
    // depth guarantees a parent precedes its child regardless of name.
    let mut dir_set: BTreeSet<String> = BTreeSet::new();
    dir_set.insert(remote_root.clone());
    for f in &files {
        if let Some(idx) = f.rel.rfind('/') {
            let mut acc = remote_root.clone();
            for seg in f.rel[..idx].split('/') {
                acc = posix_join(&acc, seg);
                dir_set.insert(acc.clone());
            }
        }
    }
    let mut ordered: Vec<String> = dir_set.into_iter().collect();
    ordered.sort_by_key(|d| d.matches('/').count());
    for dir in &ordered {
        let _ = sftp.create_dir(dir.clone()).await;
    }

    // Live progress plumbing: with a run id the sync leg emits throttled
    // upload-progress events and the steps stream their output; the cancel
    // flag stops between files / steps. The guard deregisters the flag on any
    // exit path; callers without a run id pay nothing.
    let run_id = input.run_id.clone();
    let guard = run_id.as_deref().map(DeployCancelGuard::new);
    let is_cancelled = || guard.as_ref().is_some_and(|g| g.is_cancelled());
    let total = files.len() as u32;
    let emit_sync = |uploaded: u32, bytes: u64| {
        if let Some(id) = run_id.as_deref() {
            let _ = crate::commands::events::emit_to_main(
                &app,
                DEPLOY_CHANNEL,
                DeployEvent::Sync {
                    run_id: id.to_string(),
                    uploaded,
                    total,
                    bytes,
                },
            );
        }
    };

    // Upload each file whole.
    emit_sync(0, 0);
    let mut uploaded = 0u32;
    let mut bytes = 0u64;
    let mut skipped = Vec::new();
    let mut cancelled = false;
    let mut last_emit = std::time::Instant::now();
    for f in &files {
        if is_cancelled() {
            cancelled = true;
            break;
        }
        let meta = std::fs::metadata(&f.abs)?;
        if meta.len() > MAX_FILE_BYTES {
            skipped.push(f.rel.clone());
            continue;
        }
        let data = std::fs::read(&f.abs)?;
        let remote = posix_join(&remote_root, &f.rel);
        write_remote(&sftp, remote, &data).await?;
        uploaded += 1;
        bytes += data.len() as u64;
        // Throttle progress to ~10 Hz so a tree of tiny files doesn't flood IPC.
        if last_emit.elapsed().as_millis() >= 100 {
            emit_sync(uploaded, bytes);
            last_emit = std::time::Instant::now();
        }
    }
    emit_sync(uploaded, bytes);

    // Run the build/release steps over the cached exec session, from the remote
    // root. No steps configured → sync-only deploy.
    let steps: Vec<String> = deploy
        .steps
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let step_results = if steps.is_empty() || cancelled {
        Vec::new()
    } else {
        let session = {
            let mut mgr = state.exec.lock().await;
            mgr.session_for(
                &conn,
                secret_str(&password),
                secret_str(&proxy_password),
                secret_str(&passphrase),
                interactor,
            )
            .await
            .map_err(AppError::Ssh)?
        };
        run_deploy_on_streaming(
            &session,
            &steps,
            Some(&remote_root),
            guard.as_ref().map(|g| g.flag()),
            |p| {
                if let Some(id) = run_id.as_deref() {
                    emit_deploy_progress(&app, id, p);
                }
            },
        )
        .await
        .map_err(AppError::Ssh)?
    };
    cancelled = cancelled || is_cancelled();

    Ok(DeployRunResult {
        uploaded,
        bytes,
        skipped,
        remote_path: remote_root,
        steps: step_results,
        cancelled,
    })
}

/// Whole-file remote write with create+truncate semantics (mirrors the SFTP
/// file-manager's `write_remote`).
async fn write_remote(sftp: &Arc<SftpSession>, path: String, bytes: &[u8]) -> AppResult<()> {
    let mut file = sftp
        .create(path.clone())
        .await
        .map_err(|e| AppError::Internal(format!("couldn't create `{path}`: {e}")))?;
    file.write_all(bytes)
        .await
        .map_err(|e| AppError::Internal(format!("couldn't write `{path}`: {e}")))?;
    file.shutdown()
        .await
        .map_err(|e| AppError::Internal(format!("couldn't finalise `{path}`: {e}")))?;
    Ok(())
}
