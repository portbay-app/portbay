//! Interactive PTY shell sessions over a connection's russh session.
//!
//! Where [`exec`](crate::ssh::exec) runs one command and captures its output,
//! this opens a real pseudo-terminal + login shell and streams it bidirectionally
//! — the interactive remote shell for the researcher / cluster use case (vim,
//! htop, `nvidia-smi -l`, REPLs, …).
//!
//! Each live shell is driven by a single I/O task that **owns** the russh
//! channel: russh's send methods (`data`, `window_change`) borrow `&self` while
//! `wait()` borrows `&mut self`, so one task with a `select!` loop — output on
//! one arm, control messages on the other — is the only borrow-safe shape.
//! [`PtyManager`] keeps the control-channel sender for each session so the
//! input / resize / close commands can reach the task by id.

use std::collections::HashMap;

use russh::client::Msg;
use russh::Channel;
use tokio::sync::mpsc::UnboundedSender;

use crate::registry::SshConnection;
use crate::ssh::backend::{Result, SshError};
use crate::ssh::session::{connect_session, SshSession};

/// Terminal type advertised to the remote pty. `xterm-256color` matches the
/// xterm.js emulator on the frontend, so colours and key encodings line up.
const DEFAULT_TERM: &str = "xterm-256color";

/// A control message to a running pty session's I/O task.
pub enum PtyControl {
    /// Bytes typed at the terminal (already terminal-encoded by xterm.js).
    Input(Vec<u8>),
    /// The terminal was resized — informs the remote pty so full-screen apps
    /// (vim, htop) redraw at the right dimensions.
    Resize { cols: u32, rows: u32 },
    /// Tear the session down (EOF + close the channel, end the task).
    Close,
}

/// Registry of live pty sessions, keyed by an opaque pty id. Each entry is the
/// sender half of its I/O task's control channel; the task owns the russh
/// channel + session and exits when a [`PtyControl::Close`] arrives, the shell
/// ends, or the sender drops.
#[derive(Default)]
pub struct PtyManager {
    next: u64,
    sessions: HashMap<String, UnboundedSender<PtyControl>>,
}

impl PtyManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mint a fresh, process-unique pty id.
    pub fn next_id(&mut self) -> String {
        self.next += 1;
        format!("pty-{}", self.next)
    }

    /// Record a session's control sender under `id`.
    pub fn register(&mut self, id: String, control: UnboundedSender<PtyControl>) {
        self.sessions.insert(id, control);
    }

    /// Send a control message to a session. Returns `false` if the id is unknown
    /// or its task has already ended (best-effort — a closed shell simply drops
    /// further keystrokes).
    pub fn send(&self, id: &str, ctrl: PtyControl) -> bool {
        match self.sessions.get(id) {
            Some(tx) => tx.send(ctrl).is_ok(),
            None => false,
        }
    }

    /// Forget a session (its task tears down when the sender drops).
    pub fn remove(&mut self, id: &str) {
        self.sessions.remove(id);
    }

    /// Drop every session (app shutdown / window destroy).
    pub fn disconnect_all(&mut self) {
        self.sessions.clear();
    }
}

/// Connect + authenticate, open a session channel, request a pty of the given
/// size, and start either a login shell (`command` = `None`) or a single
/// program under the pty (`command` = `Some`, e.g. `tail -F …` for the Logs
/// tab). Returns the live channel (for the I/O loop) and the owning
/// [`SshSession`] — kept alive for the channel's lifetime, including any
/// `ProxyJump` chain it tunnels through.
pub async fn open_shell(
    conn: &SshConnection,
    password: Option<&str>,
    proxy_password: Option<&str>,
    passphrase: Option<&str>,
    cols: u32,
    rows: u32,
    command: Option<&str>,
) -> Result<(SshSession, Channel<Msg>)> {
    let session = connect_session(conn, password, proxy_password, passphrase, None).await?;
    let channel = open_shell_channel(&session, cols, rows, command).await?;
    Ok((session, channel))
}

/// Open a pty + shell channel on an **already-connected** session, skipping the
/// connect + auth handshake entirely.
///
/// This is the warm path the terminal takes: the host workspace has already
/// established an authenticated session (host snapshot / exec), and russh
/// multiplexes many channels over one connection, so opening a shell is a single
/// channel request — milliseconds — instead of a fresh TCP + SSH handshake +
/// auth pipeline (and, with a `ProxyJump`, the whole chain again) on every tab.
/// That handshake-per-tab was the 10–20 s "terminal takes forever to open" lag.
///
/// The caller keeps the owning session alive (e.g. an `Arc<SshSession>` clone in
/// the I/O task) for the channel's lifetime.
pub async fn open_shell_channel(
    session: &SshSession,
    cols: u32,
    rows: u32,
    command: Option<&str>,
) -> Result<Channel<Msg>> {
    let channel = session
        .channel_open_session()
        .await
        .map_err(|e| SshError::Russh(format!("couldn't open shell channel: {e}")))?;
    channel
        .request_pty(true, DEFAULT_TERM, cols.max(1), rows.max(1), 0, 0, &[])
        .await
        .map_err(|e| SshError::Russh(format!("couldn't request a pty: {e}")))?;
    match command.map(str::trim).filter(|c| !c.is_empty()) {
        // Run one program under the pty (Logs follow, watch-style commands).
        Some(cmd) => channel
            .exec(true, cmd.as_bytes())
            .await
            .map_err(|e| SshError::Russh(format!("couldn't start remote command: {e}")))?,
        // Interactive login shell.
        None => channel
            .request_shell(true)
            .await
            .map_err(|e| SshError::Russh(format!("couldn't start the remote shell: {e}")))?,
    }
    Ok(channel)
}
