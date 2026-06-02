//! SFTP session manager: one cached, multiplexed SFTP subsystem per connection.
//!
//! Opening an SSH session + sftp subsystem costs a full handshake (~100–500 ms),
//! so a file manager that re-handshakes on every click would feel sluggish. We
//! cache one live [`SftpSession`] per connection id and hand out cheap `Arc`
//! clones; russh-sftp multiplexes concurrent requests over the single channel,
//! so callers run ops in parallel without locking each other out. A session
//! whose underlying SSH handle has closed is transparently re-opened.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use russh_sftp::client::SftpSession;

use crate::registry::SshConnection;
use crate::ssh::backend::{Result, SshError};
use crate::ssh::interaction::SshInteractor;
use crate::ssh::session::{connect_session, SshSession};

/// A live SFTP subsystem plus the SSH session it rides on (kept to probe
/// liveness and to keep the session — and any jump chain — alive). Channel ops
/// and `is_closed()` reach the target handle through `SshSession`'s `Deref`.
struct CachedSftp {
    session: SshSession,
    sftp: Arc<SftpSession>,
    last_used: Instant,
}

#[derive(Default)]
pub struct SftpManager {
    sessions: HashMap<String, CachedSftp>,
}

impl SftpManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Return a live SFTP session for `conn`, reusing the cached one when its
    /// SSH handle is still open, otherwise (re)connecting. `password` is only
    /// consulted when a new session must be opened for a password-auth host.
    pub async fn session_for(
        &mut self,
        conn: &SshConnection,
        password: Option<&str>,
        proxy_password: Option<&str>,
        passphrase: Option<&str>,
        interactor: Option<Arc<dyn SshInteractor>>,
    ) -> Result<Arc<SftpSession>> {
        if let Some(cached) = self.sessions.get_mut(conn.id.as_str()) {
            if !cached.session.is_closed() {
                cached.last_used = Instant::now();
                return Ok(cached.sftp.clone());
            }
            // Dead session — drop it and reconnect below.
            self.sessions.remove(conn.id.as_str());
        }

        let session = connect_session(conn, password, proxy_password, passphrase, interactor).await?;
        let channel = session
            .channel_open_session()
            .await
            .map_err(|e| SshError::Russh(format!("couldn't open SSH channel: {e}")))?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|e| SshError::Russh(format!("couldn't start the sftp subsystem: {e}")))?;
        let sftp = SftpSession::new(channel.into_stream())
            .await
            .map_err(|e| SshError::Russh(format!("sftp handshake failed: {e}")))?;
        let sftp = Arc::new(sftp);

        self.sessions.insert(
            conn.id.as_str().to_string(),
            CachedSftp {
                session,
                sftp: sftp.clone(),
                last_used: Instant::now(),
            },
        );
        Ok(sftp)
    }

    /// Drop sessions idle longer than `max_idle`, plus any whose SSH handle has
    /// already closed. Called periodically by the background reaper.
    pub fn reap_idle(&mut self, max_idle: Duration) {
        self.sessions
            .retain(|_, c| !c.session.is_closed() && c.last_used.elapsed() < max_idle);
    }

    /// Drop the cached session for a connection (e.g. the user disconnected, or
    /// an op hit a dead session). Best-effort — the russh task closes when the
    /// handle drops.
    pub fn disconnect(&mut self, conn_id: &str) {
        self.sessions.remove(conn_id);
    }

    /// Drop every cached session (app shutdown).
    pub fn disconnect_all(&mut self) {
        self.sessions.clear();
    }
}
