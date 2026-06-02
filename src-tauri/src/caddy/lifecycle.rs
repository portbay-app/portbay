//! Caddy sidecar lifecycle.
//!
//! Parallel architecture to `process_compose::lifecycle::SidecarManager`.
//! Same gotchas; same fix — `caddy run --resume`, not `caddy start`
//! (spike Quirk 2). Bootstrapping starts with an admin-only JSON; once
//! the daemon is up, `client::load()` pushes the real registry-derived
//! config.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tauri::AppHandle;
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
use tauri_plugin_shell::ShellExt;

use crate::caddy::client::CaddyClient;
use crate::caddy::error::{CaddyError, Result};

pub const DEFAULT_ADMIN_PORT: u16 = 2019;
pub const DEFAULT_HTTPS_PORT: u16 = 8443;

pub const ADMIN_SCAN_RANGE: u16 = 32;

#[derive(Debug)]
pub struct CaddySidecar {
    child: Option<CommandChild>,
    admin_port: u16,
    /// True only while the spawned caddy is actually alive. The Tauri
    /// `CommandChild` handle exists the instant `.spawn()` returns even if the
    /// process exits a millisecond later (a rejected config, a bound admin
    /// port), so `child.is_some()` alone is a liar — and because Caddy is the
    /// reverse proxy, a false "up" silently breaks all routing. A background
    /// task watching the event stream flips this to `false` on `Terminated`.
    alive: Arc<AtomicBool>,
}

impl Default for CaddySidecar {
    fn default() -> Self {
        Self::new()
    }
}

impl CaddySidecar {
    pub fn new() -> Self {
        Self {
            child: None,
            admin_port: DEFAULT_ADMIN_PORT,
            alive: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn is_running(&self) -> bool {
        self.child.is_some() && self.alive.load(Ordering::Relaxed)
    }

    pub fn admin_port(&self) -> u16 {
        self.admin_port
    }

    /// PID of the running caddy child, if any. Used by the reconciler's
    /// :80 conflict check to recognise our own daemon (so it isn't reported
    /// as an external port-80 holder).
    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().map(|c| c.pid())
    }

    /// Spawn the bundled `caddy` sidecar against an initial JSON config.
    ///
    /// `admin_port` MUST match the `admin.listen` port baked into
    /// `config_path` — the returned `CaddyClient` is wired to talk to that
    /// exact port. Pre-scan for a free port via [`find_free_port`] before
    /// calling, then write the config with that port using
    /// [`super::config::bootstrap_config`].
    ///
    /// The config file should contain at minimum the admin endpoint; the
    /// real apps config is loaded over the admin API once the daemon is
    /// up. Callers usually want [`super::config::build_config`] for that
    /// follow-up `POST /load`.
    pub fn start(
        &mut self,
        app: &AppHandle,
        config_path: &Path,
        admin_port: u16,
    ) -> Result<CaddyClient> {
        // is_running() (not child.is_some()) so a crashed-but-not-reaped caddy
        // is respawned rather than wrongly treated as still up.
        if self.is_running() {
            return Ok(CaddyClient::new(self.admin_port));
        }

        self.admin_port = admin_port;

        let config_str = config_path.to_string_lossy().into_owned();

        // `caddy run --config ...` is the launch shape — Quirk 2 from the
        // spike. `caddy start` has hanging-shell behaviour and forks into
        // a backgrounded daemon that we can't manage as a child process.
        let cmd = app
            .shell()
            .sidecar("caddy")
            .map_err(|e| CaddyError::SpawnFailed(e.to_string()))?
            .args(["run", "--config", &config_str]);

        let (mut rx, child) = cmd
            .spawn()
            .map_err(|e| CaddyError::SpawnFailed(e.to_string()))?;
        self.alive.store(true, Ordering::Relaxed);
        self.child = Some(child);

        // Drain the event stream so caddy's own diagnostics surface, and flip
        // `alive` the moment the process exits so `is_running` can't keep
        // claiming a dead reverse proxy is up.
        let alive = self.alive.clone();
        tauri::async_runtime::spawn(async move {
            while let Some(event) = rx.recv().await {
                match event {
                    CommandEvent::Stderr(bytes) => {
                        let line = String::from_utf8_lossy(&bytes);
                        let line = line.trim_end();
                        if !line.is_empty() {
                            tracing::debug!(target: "caddy", "{line}");
                        }
                    }
                    CommandEvent::Error(err) => {
                        tracing::warn!(target: "caddy", error = %err, "caddy sidecar error");
                    }
                    CommandEvent::Terminated(payload) => {
                        tracing::warn!(target: "caddy", code = ?payload.code, "caddy sidecar terminated");
                        break;
                    }
                    _ => {}
                }
            }
            alive.store(false, Ordering::Relaxed);
        });

        Ok(CaddyClient::new(admin_port))
    }

    pub fn stop(&mut self) {
        self.alive.store(false, Ordering::Relaxed);
        if let Some(child) = self.child.take() {
            let _ = child.kill();
        }
    }
}

impl Drop for CaddySidecar {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Pre-flight port check: bind, capture the port, release. Skips any port in
/// `avoid` (typically the registered projects' declared ports) so a Caddy admin
/// port never lands on a port a dev server expects. Same shape as the PC
/// adapter's helper but kept module-local so callers can think about
/// HTTPS-port (likely 443/8443) and admin-port (likely 2019) as distinct concerns.
pub fn find_free_port(start: u16, range: u16, avoid: &[u16]) -> Option<u16> {
    for offset in 0..range {
        let port = start.checked_add(offset)?;
        if avoid.contains(&port) {
            continue;
        }
        if std::net::TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return Some(port);
        }
    }
    None
}

/// Pre-flight check for the public HTTPS port. Prefers `prefer` (normally 443)
/// if it's free and not in `avoid`; otherwise falls back to a scan from
/// `fallback`.
///
/// The bind test uses the **wildcard** address, not `127.0.0.1`, on purpose:
/// macOS denies a non-root process a privileged-port (<1024) bind on the
/// loopback-specific address (`127.0.0.1:443` → EACCES) but *allows* it on the
/// wildcard (`0.0.0.0:443`) — and Caddy binds the wildcard (`:443`). Testing
/// `127.0.0.1:443` therefore wrongly fails and forces a fallback to 8443, so a
/// browser hitting `https://<host>` (port 443) finds nothing. Matching Caddy's
/// bind makes the pre-flight honest.
pub fn find_free_https_port(prefer: u16, fallback: u16, avoid: &[u16]) -> u16 {
    if !avoid.contains(&prefer) && std::net::TcpListener::bind(("0.0.0.0", prefer)).is_ok() {
        return prefer;
    }
    find_free_port(fallback, 32, avoid).unwrap_or(fallback)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_free_admin_port_near_default() {
        let p = find_free_port(DEFAULT_ADMIN_PORT, 32, &[]);
        assert!(p.is_some());
    }

    #[test]
    fn https_port_falls_back_when_443_is_privileged() {
        // 443 will generally be held or privileged; expect fallback to
        // somewhere in the high range.
        let p = find_free_https_port(443, DEFAULT_HTTPS_PORT, &[]);
        assert!(p >= 1024 || p == 443);
    }
}
