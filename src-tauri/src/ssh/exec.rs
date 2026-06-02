//! Remote command execution + deploy sequences over a connection's russh session.
//!
//! Each call opens one authenticated session (key or password) and runs one or
//! more commands on their own `exec` channels, capturing stdout/stderr + exit
//! code. A deploy runs an ordered list and **stops on the first non-zero exit**,
//! so a failed `npm ci` doesn't proceed to `npm run build`.
//!
//! Output is captured per command (not streamed). For the typical
//! sync-then-build flow each step's full output lands when it finishes; live
//! streaming of long builds is a future refinement.

use russh::ChannelMsg;
use serde::Serialize;

use std::sync::Arc;

use crate::registry::SshConnection;
use crate::ssh::backend::{Result, SshError};
use crate::ssh::interaction::SshInteractor;
use crate::ssh::session::{connect_session, SshSessionHandle};

// The destination handle is reached via `Deref` on the returned `SshSession`,
// so a jump chain is transparent to the exec/deploy channel plumbing below.

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecResult {
    pub stdout: String,
    pub stderr: String,
    /// Process exit code; `-1` if the server never reported one.
    pub exit_code: i32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepResult {
    pub command: String,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// Run a single command, optionally from a working directory. `interactor`
/// (when present) surfaces an untrusted host-key decision to the user on a cold
/// connect; `None` keeps the legacy silent TOFU for headless callers.
pub async fn run_command(
    conn: &SshConnection,
    password: Option<&str>,
    proxy_password: Option<&str>,
    passphrase: Option<&str>,
    command: &str,
    cwd: Option<&str>,
    interactor: Option<Arc<dyn SshInteractor>>,
) -> Result<ExecResult> {
    let session = connect_session(conn, password, proxy_password, passphrase, interactor).await?;
    exec_on(&session, command, cwd).await
}

/// Run an ordered list of commands over a single session. Stops at the first
/// step whose exit code is non-zero (that step is included in the results).
pub async fn run_deploy(
    conn: &SshConnection,
    password: Option<&str>,
    proxy_password: Option<&str>,
    passphrase: Option<&str>,
    steps: &[String],
    cwd: Option<&str>,
) -> Result<Vec<StepResult>> {
    // No live UI caller reaches this one-shot deploy path; keep the legacy
    // silent TOFU. The interactive deploy goes through `ExecManager` +
    // `run_deploy_on`.
    let session = connect_session(conn, password, proxy_password, passphrase, None).await?;
    let mut results = Vec::with_capacity(steps.len());
    for step in steps {
        let r = exec_on(&session, step, cwd).await?;
        let failed = r.exit_code != 0;
        results.push(StepResult {
            command: step.clone(),
            stdout: r.stdout,
            stderr: r.stderr,
            exit_code: r.exit_code,
        });
        if failed {
            break;
        }
    }
    Ok(results)
}

/// Like [`run_deploy`], but runs over an already-open handle (the cached exec
/// session from [`crate::ssh::ExecManager`]) so a deploy reuses the connection
/// instead of re-authenticating. Stops at the first non-zero exit.
pub async fn run_deploy_on(
    handle: &SshSessionHandle,
    steps: &[String],
    cwd: Option<&str>,
) -> Result<Vec<StepResult>> {
    let mut results = Vec::with_capacity(steps.len());
    for step in steps {
        let r = exec_on(handle, step, cwd).await?;
        let failed = r.exit_code != 0;
        results.push(StepResult {
            command: step.clone(),
            stdout: r.stdout,
            stderr: r.stderr,
            exit_code: r.exit_code,
        });
        if failed {
            break;
        }
    }
    Ok(results)
}

/// Open an exec channel, run `command` (prefixed with `cd <cwd> &&` when set),
/// and drain stdout/stderr + the exit status.
pub async fn exec_on(
    handle: &SshSessionHandle,
    command: &str,
    cwd: Option<&str>,
) -> Result<ExecResult> {
    let full = match cwd.map(str::trim).filter(|d| !d.is_empty()) {
        Some(dir) => format!("cd {} && {command}", shell_quote(dir)),
        None => command.to_string(),
    };

    let channel = handle
        .channel_open_session()
        .await
        .map_err(|e| SshError::Russh(format!("couldn't open exec channel: {e}")))?;
    channel
        .exec(true, full.as_bytes())
        .await
        .map_err(|e| SshError::Russh(format!("couldn't start remote command: {e}")))?;

    let mut stdout: Vec<u8> = Vec::new();
    let mut stderr: Vec<u8> = Vec::new();
    let mut code: Option<u32> = None;
    let mut channel = channel;
    while let Some(msg) = channel.wait().await {
        match msg {
            ChannelMsg::Data { ref data } => stdout.extend_from_slice(data),
            // ext type 1 is stderr (SSH_EXTENDED_DATA_STDERR).
            ChannelMsg::ExtendedData { ref data, ext: 1 } => stderr.extend_from_slice(data),
            ChannelMsg::ExitStatus { exit_status } => code = Some(exit_status),
            _ => {}
        }
    }

    Ok(ExecResult {
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        exit_code: code.map(|c| c as i32).unwrap_or(-1),
    })
}

/// Minimal POSIX single-quote escaping for a path used in `cd <dir>`.
fn shell_quote(arg: &str) -> String {
    if arg
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-'))
    {
        return arg.to_string();
    }
    format!("'{}'", arg.replace('\'', "'\\''"))
}
