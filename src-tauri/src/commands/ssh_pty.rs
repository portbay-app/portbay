//! Interactive PTY shell commands.
//!
//! `ssh_pty_open` connects, opens a pty + shell, and spawns an I/O task that
//! streams output to the frontend over a `Channel<PtyEvent>`. The connect runs
//! **synchronously** inside the command so an auth gap surfaces as
//! `SSH_NEEDS_PASSWORD` / `SSH_NEEDS_PASSPHRASE` for the frontend's
//! credential-prompt retry loop (same contract as exec / sftp). Once open, the
//! returned pty id addresses `ssh_pty_input` / `ssh_pty_resize` /
//! `ssh_pty_close`.
//!
//! Output bytes ride the channel as a `Vec<u8>` (JSON number array). That's
//! byte-exact and simple; a binary/base64 framing is a future throughput
//! refinement for very chatty sessions.

use std::sync::Arc;

use russh::client::Msg;
use russh::{Channel, ChannelMsg};
use serde::{Deserialize, Serialize};
use tauri::ipc::Channel as IpcChannel;
use tauri::{AppHandle, State};
use tokio::sync::mpsc;

use crate::commands::projects::load_registry;
use crate::commands::ssh_tunnels::{
    load_stored_key_passphrase, load_stored_password, load_stored_proxy_password,
};
use crate::error::{AppError, AppResult};
use crate::registry::SshConnectionId;
use crate::ssh::pty::{open_shell_channel, PtyControl};
use crate::ssh::session::SshSession;
use crate::state::AppState;

/// An event streamed from a live pty to the frontend terminal.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PtyEvent {
    /// Raw output bytes from the remote pty (stdout + stderr merged, as a real
    /// terminal delivers them).
    Data { bytes: Vec<u8> },
    /// The shell exited; `code` is its status when the server reported one.
    Exit { code: Option<i32> },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PtyOpenInput {
    pub connection_id: String,
    /// Initial terminal size. Defaults keep a sane shell if the UI hasn't
    /// measured yet; the first resize corrects it.
    #[serde(default = "default_cols")]
    pub cols: u32,
    #[serde(default = "default_rows")]
    pub rows: u32,
    /// One-shot password from the credential prompt; this connect only, never
    /// stored. Blank/absent falls back to a keychain-saved password.
    #[serde(default)]
    pub password: Option<String>,
    /// One-shot key passphrase from the credential prompt; one-connect only.
    #[serde(default)]
    pub passphrase: Option<String>,
    /// Optional program to run under the pty instead of a login shell — used by
    /// the Logs tab (`tail -F …`). Blank/absent opens an interactive shell.
    #[serde(default)]
    pub command: Option<String>,
}

fn default_cols() -> u32 {
    80
}
fn default_rows() -> u32 {
    24
}

/// Open an interactive shell. Returns the pty id used by the input/resize/close
/// commands. Output streams over `on_event` until the shell exits.
#[tauri::command]
pub async fn ssh_pty_open(
    state: State<'_, AppState>,
    app: AppHandle,
    input: PtyOpenInput,
    on_event: IpcChannel<PtyEvent>,
) -> AppResult<String> {
    // Resolve the connection + its secrets exactly like exec / sftp.
    let conn = {
        let registry = load_registry(&state)?;
        let raw = registry
            .get_ssh_connection(&SshConnectionId::new(&input.connection_id))
            .ok_or_else(|| {
                AppError::BadInput(format!(
                    "SSH connection `{}` not found",
                    input.connection_id
                ))
            })?;
        // Fold in a borrowed identity (user / key / auth) before connecting.
        registry.effective_ssh_connection(raw)
    };
    let nonblank = |s: Option<String>| s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    let password = match nonblank(input.password) {
        Some(p) => Some(p),
        None => load_stored_password(&conn.id)?,
    };
    let passphrase = match nonblank(input.passphrase) {
        Some(p) => Some(p),
        None => load_stored_key_passphrase(&conn.id)?,
    };
    let proxy_password = load_stored_proxy_password(&conn.id)?;

    // Reuse the host's warm, already-authenticated exec session (established by
    // the workspace's host-snapshot/exec calls) and open the shell as one more
    // multiplexed channel on it — milliseconds, not a fresh TCP+SSH handshake +
    // auth pipeline (and ProxyJump chain) per terminal tab. Opening the session
    // here runs synchronously so an auth gap on a *cold* host still surfaces as
    // SSH_NEEDS_* for the frontend's credential-prompt retry loop. The returned
    // `Arc` is moved into the I/O task to keep the session (and any jump chain)
    // alive for the channel's lifetime even if the exec manager reaps its entry.
    let session = {
        let mut mgr = state.exec.lock().await;
        mgr.session_for(
            &conn,
            password.as_deref(),
            proxy_password.as_deref(),
            passphrase.as_deref(),
            Some(crate::ssh::EventInteractor::new(app)),
        )
        .await
        .map_err(AppError::Ssh)?
    };
    let channel = open_shell_channel(&session, input.cols, input.rows, input.command.as_deref())
        .await
        .map_err(AppError::Ssh)?;

    let (tx, rx) = mpsc::unbounded_channel::<PtyControl>();
    let id = {
        let mut mgr = state.pty.lock().await;
        let id = mgr.next_id();
        mgr.register(id.clone(), tx);
        id
    };

    // The I/O task owns the channel + session; it exits on shell EOF/Close, a
    // Close control, or when the control sender drops. The frontend calls
    // ssh_pty_close to reap the registry entry; a natural exit emits Exit so the
    // UI can reflect it.
    tokio::spawn(async move {
        run_pty(channel, session, rx, on_event).await;
    });

    Ok(id)
}

/// Drive one pty: forward remote output to the frontend and apply control
/// messages. Owns the channel for its lifetime (see module docs for why).
async fn run_pty(
    mut channel: Channel<Msg>,
    _session: Arc<SshSession>,
    mut rx: mpsc::UnboundedReceiver<PtyControl>,
    out: IpcChannel<PtyEvent>,
) {
    let mut exit_code: Option<i32> = None;
    loop {
        tokio::select! {
            msg = channel.wait() => {
                match msg {
                    Some(ChannelMsg::Data { data }) => {
                        let _ = out.send(PtyEvent::Data { bytes: data.to_vec() });
                    }
                    Some(ChannelMsg::ExtendedData { data, .. }) => {
                        let _ = out.send(PtyEvent::Data { bytes: data.to_vec() });
                    }
                    Some(ChannelMsg::ExitStatus { exit_status }) => {
                        exit_code = Some(exit_status as i32);
                    }
                    // EOF / Close / channel gone — the shell is done.
                    Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => break,
                    _ => {}
                }
            }
            ctrl = rx.recv() => {
                match ctrl {
                    Some(PtyControl::Input(bytes)) => {
                        let _ = channel.data(bytes.as_slice()).await;
                    }
                    Some(PtyControl::Resize { cols, rows }) => {
                        let _ = channel.window_change(cols.max(1), rows.max(1), 0, 0).await;
                    }
                    // Explicit close, or every sender dropped: end the shell.
                    Some(PtyControl::Close) | None => {
                        let _ = channel.eof().await;
                        let _ = channel.close().await;
                        break;
                    }
                }
            }
        }
    }
    let _ = out.send(PtyEvent::Exit { code: exit_code });
}

/// Send typed input to a pty. Best-effort: a closed session silently drops it.
#[tauri::command]
pub async fn ssh_pty_input(state: State<'_, AppState>, id: String, data: String) -> AppResult<()> {
    state
        .pty
        .lock()
        .await
        .send(&id, PtyControl::Input(data.into_bytes()));
    Ok(())
}

/// Inform a pty its terminal was resized (columns × rows).
#[tauri::command]
pub async fn ssh_pty_resize(
    state: State<'_, AppState>,
    id: String,
    cols: u32,
    rows: u32,
) -> AppResult<()> {
    state
        .pty
        .lock()
        .await
        .send(&id, PtyControl::Resize { cols, rows });
    Ok(())
}

/// Close a pty and forget it.
#[tauri::command]
pub async fn ssh_pty_close(state: State<'_, AppState>, id: String) -> AppResult<()> {
    let mut mgr = state.pty.lock().await;
    mgr.send(&id, PtyControl::Close);
    mgr.remove(&id);
    Ok(())
}
