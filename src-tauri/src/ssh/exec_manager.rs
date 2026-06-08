//! Exec session manager: one cached, authenticated SSH session per connection,
//! reused across exec/deploy calls so navigating the host workspace (Terminal →
//! Logs → Processes → Deploy …) doesn't re-authenticate on every visit.
//!
//! Mirrors [`crate::ssh::SftpManager`]: it caches one live [`SshSession`] per
//! connection and hands out cheap `Arc` clones so the manager lock is released
//! before a possibly-long command runs (a deploy mustn't block Logs/Processes).
//! The `Arc` keeps the session — and any jump chain — alive while a caller runs
//! commands on it; exec channels reach the target handle through `SshSession`'s
//! `Deref`. A session whose SSH handle has closed is transparently re-opened,
//! and idle sessions are reaped on a timer.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::registry::SshConnection;
use crate::ssh::backend::Result;
use crate::ssh::interaction::SshInteractor;
use crate::ssh::session::{connect_session, SshSession};

/// A live authenticated session plus when it was last handed out (for idle
/// reaping). Liveness and `is_closed()` reach the target handle via `Deref`.
struct CachedExec {
    session: Arc<SshSession>,
    last_used: Instant,
}

#[derive(Default)]
pub struct ExecManager {
    sessions: HashMap<String, CachedExec>,
}

impl ExecManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Return a live session for `conn`, reusing the cached one while its SSH
    /// handle is still open, otherwise (re)connecting. The returned `Arc` lets
    /// the caller drop the manager lock before running commands on it.
    /// `password`/`passphrase` are only consulted when a new session must be
    /// opened. `interactor` (when present) surfaces an untrusted host-key
    /// decision to the user on that cold connect.
    pub async fn session_for(
        &mut self,
        conn: &SshConnection,
        password: Option<&str>,
        proxy_password: Option<&str>,
        passphrase: Option<&str>,
        interactor: Option<Arc<dyn SshInteractor>>,
    ) -> Result<Arc<SshSession>> {
        if let Some(cached) = self.sessions.get_mut(conn.id.as_str()) {
            if !cached.session.is_closed() {
                cached.last_used = Instant::now();
                return Ok(cached.session.clone());
            }
            // Dead session — drop it and reconnect below.
            self.sessions.remove(conn.id.as_str());
        }

        let session = Arc::new(
            connect_session(conn, password, proxy_password, passphrase, interactor).await?,
        );
        self.sessions.insert(
            conn.id.as_str().to_string(),
            CachedExec {
                session: session.clone(),
                last_used: Instant::now(),
            },
        );
        Ok(session)
    }

    /// Drop sessions idle longer than `max_idle`, plus any whose handle has
    /// already closed. Called periodically by the background reaper so a host
    /// doesn't hold an authenticated connection open forever.
    pub fn reap_idle(&mut self, max_idle: Duration) {
        self.sessions
            .retain(|_, c| !c.session.is_closed() && c.last_used.elapsed() < max_idle);
    }

    /// Drop the cached session for a connection (e.g. the user disconnected).
    pub fn disconnect(&mut self, conn_id: &str) {
        self.sessions.remove(conn_id);
    }

    /// Whether a still-open session is cached for this connection. Read-only —
    /// doesn't bump `last_used`, so a status poll never keeps a session alive.
    pub fn has_session(&self, conn_id: &str) -> bool {
        self.sessions
            .get(conn_id)
            .is_some_and(|c| !c.session.is_closed())
    }

    /// Drop every cached session (app shutdown / state reset).
    pub fn disconnect_all(&mut self) {
        self.sessions.clear();
    }
}
