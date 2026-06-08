//! Remote command execution + deploy sequences over a connection's russh session.
//!
//! Each call opens one authenticated session (key or password) and runs one or
//! more commands on their own `exec` channels, capturing stdout/stderr + exit
//! code. A deploy runs an ordered list and **stops on the first non-zero exit**,
//! so a failed `npm ci` doesn't proceed to `npm run build`.
//!
//! Output is both captured per command (the returned `StepResult`s stay the
//! source of truth) and, via the `_streaming` variants, forwarded chunk-by-chunk
//! to a callback so the UI can show a long build live. A shared cancel flag
//! stops a run between steps and best-effort kills the in-flight one (SIGTERM
//! over the channel, then close — servers that ignore `signal` requests still
//! free the channel, though the remote process may run on).

use russh::ChannelMsg;
use serde::Serialize;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

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

/// Live progress from a streaming deploy run, forwarded to a caller-supplied
/// callback (the command layer maps these onto Tauri events).
#[derive(Debug, Clone)]
pub enum DeployProgress {
    /// Step `index` (0-based) has started executing.
    StepStarted { index: usize, command: String },
    /// A chunk of output from the running step. `stderr` flags the stream it
    /// arrived on; chunks are lossy-UTF-8 and split on valid boundaries.
    Output {
        index: usize,
        stderr: bool,
        chunk: String,
    },
    /// Step `index` finished (or was cancelled mid-flight with exit code -1).
    StepDone {
        index: usize,
        exit_code: i32,
        duration_ms: u64,
    },
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
    run_deploy_on_streaming(handle, steps, cwd, None, |_| {}).await
}

/// Like [`run_deploy_on`], but forwards [`DeployProgress`] to `progress` as the
/// run executes and honours `cancel`: a set flag skips queued steps and
/// best-effort kills the in-flight one (its partial output is still included,
/// with exit code -1). The returned `StepResult`s stay the source of truth.
pub async fn run_deploy_on_streaming(
    handle: &SshSessionHandle,
    steps: &[String],
    cwd: Option<&str>,
    cancel: Option<Arc<AtomicBool>>,
    mut progress: impl FnMut(DeployProgress),
) -> Result<Vec<StepResult>> {
    let cancelled = |c: &Option<Arc<AtomicBool>>| c.as_ref().is_some_and(|f| f.load(Ordering::SeqCst));
    let mut results = Vec::with_capacity(steps.len());
    for (index, step) in steps.iter().enumerate() {
        if cancelled(&cancel) {
            break;
        }
        progress(DeployProgress::StepStarted {
            index,
            command: step.clone(),
        });
        let started = Instant::now();
        let r = exec_on_streaming(handle, step, cwd, cancel.clone(), |stderr, chunk| {
            progress(DeployProgress::Output {
                index,
                stderr,
                chunk,
            });
        })
        .await?;
        let duration_ms = started.elapsed().as_millis() as u64;
        progress(DeployProgress::StepDone {
            index,
            exit_code: r.exit_code,
            duration_ms,
        });
        let failed = r.exit_code != 0;
        results.push(StepResult {
            command: step.clone(),
            stdout: r.stdout,
            stderr: r.stderr,
            exit_code: r.exit_code,
        });
        if failed || cancelled(&cancel) {
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
    exec_on_streaming(handle, command, cwd, None, |_, _| {}).await
}

/// Streaming core under [`exec_on`]: drains the channel while forwarding
/// output chunks to `on_output` (flushed every ~100 ms or 16 KiB, split on
/// valid UTF-8 boundaries so a multi-byte char never tears across chunks).
/// When `cancel` flips mid-run the remote process gets a best-effort SIGTERM
/// and the channel is closed; whatever output arrived is returned with exit
/// code -1 (servers don't report a status for a killed channel).
pub async fn exec_on_streaming(
    handle: &SshSessionHandle,
    command: &str,
    cwd: Option<&str>,
    cancel: Option<Arc<AtomicBool>>,
    mut on_output: impl FnMut(bool, String),
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
    // Bytes accumulated since the last flush to `on_output`, per stream.
    let mut pend_out: Vec<u8> = Vec::new();
    let mut pend_err: Vec<u8> = Vec::new();
    let mut code: Option<u32> = None;
    let mut closing = false;
    let mut channel = channel;
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    // Flush the valid-UTF-8 prefix of `pend`, keeping any trailing incomplete
    // multi-byte sequence buffered for the next flush.
    fn flush(pend: &mut Vec<u8>, is_err: bool, on_output: &mut impl FnMut(bool, String)) {
        if pend.is_empty() {
            return;
        }
        let valid = match std::str::from_utf8(pend) {
            Ok(_) => pend.len(),
            Err(e) => e.valid_up_to(),
        };
        if valid == 0 {
            return;
        }
        let chunk = String::from_utf8_lossy(&pend[..valid]).into_owned();
        pend.drain(..valid);
        on_output(is_err, chunk);
    }

    loop {
        tokio::select! {
            msg = channel.wait() => {
                let Some(msg) = msg else { break };
                match msg {
                    ChannelMsg::Data { ref data } => {
                        stdout.extend_from_slice(data);
                        pend_out.extend_from_slice(data);
                        if pend_out.len() >= 16 * 1024 {
                            flush(&mut pend_out, false, &mut on_output);
                        }
                    }
                    // ext type 1 is stderr (SSH_EXTENDED_DATA_STDERR).
                    ChannelMsg::ExtendedData { ref data, ext: 1 } => {
                        stderr.extend_from_slice(data);
                        pend_err.extend_from_slice(data);
                        if pend_err.len() >= 16 * 1024 {
                            flush(&mut pend_err, true, &mut on_output);
                        }
                    }
                    ChannelMsg::ExitStatus { exit_status } => code = Some(exit_status),
                    _ => {}
                }
            }
            _ = tick.tick() => {
                flush(&mut pend_out, false, &mut on_output);
                flush(&mut pend_err, true, &mut on_output);
                if closing {
                    // Grace tick after the close went out — the server didn't
                    // wrap up the channel, bail with what we have.
                    break;
                }
                if cancel.as_ref().is_some_and(|c| c.load(Ordering::SeqCst)) {
                    closing = true;
                    let _ = channel.signal(russh::Sig::TERM).await;
                    let _ = channel.close().await;
                }
            }
        }
    }
    flush(&mut pend_out, false, &mut on_output);
    flush(&mut pend_err, true, &mut on_output);

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
