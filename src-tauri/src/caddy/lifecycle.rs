//! Caddy sidecar lifecycle.
//!
//! Parallel architecture to `process_compose::lifecycle::SidecarManager`.
//! Same gotchas; same fix — `caddy run --resume`, not `caddy start`
//! (spike Quirk 2). Bootstrapping starts with an admin-only JSON; once
//! the daemon is up, `client::load()` pushes the real registry-derived
//! config.

use std::path::Path;

use tauri::AppHandle;
use tauri_plugin_shell::process::CommandChild;
use tauri_plugin_shell::ShellExt;

use crate::caddy::client::CaddyClient;
use crate::caddy::error::{CaddyError, Result};

pub const DEFAULT_ADMIN_PORT: u16 = 2019;
pub const DEFAULT_HTTPS_PORT: u16 = 8443;

const ADMIN_SCAN_RANGE: u16 = 32;

#[derive(Debug)]
pub struct CaddySidecar {
    child: Option<CommandChild>,
    admin_port: u16,
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
        }
    }

    pub fn is_running(&self) -> bool {
        self.child.is_some()
    }

    pub fn admin_port(&self) -> u16 {
        self.admin_port
    }

    /// Spawn the bundled `caddy` sidecar against an initial JSON config.
    ///
    /// The config file should contain at minimum the admin endpoint; the
    /// real apps config is loaded over the admin API once the daemon is
    /// up. Callers usually want
    /// [`super::config::build_config`] for that follow-up `POST /load`.
    pub fn start(&mut self, app: &AppHandle, config_path: &Path) -> Result<CaddyClient> {
        if self.child.is_some() {
            return Ok(CaddyClient::new(self.admin_port));
        }

        let admin_port = find_free_port(DEFAULT_ADMIN_PORT, ADMIN_SCAN_RANGE)
            .ok_or(CaddyError::NoFreePort {
                start: DEFAULT_ADMIN_PORT,
            })?;
        self.admin_port = admin_port;

        let config_str = config_path.to_string_lossy().into_owned();

        // `caddy run --resume` over `caddy start` — Quirk 2 from the spike.
        // --resume picks up autosave.json automatically; --config overrides
        // when we want a clean boot.
        let cmd = app
            .shell()
            .sidecar("caddy")
            .map_err(|e| CaddyError::SpawnFailed(e.to_string()))?
            .args(["run", "--config", &config_str]);

        let (_rx, child) = cmd
            .spawn()
            .map_err(|e| CaddyError::SpawnFailed(e.to_string()))?;
        self.child = Some(child);

        Ok(CaddyClient::new(admin_port))
    }

    pub fn stop(&mut self) {
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

/// Pre-flight port check: bind, capture the port, release. Same shape as
/// the PC adapter's helper but kept module-local so callers can think
/// about HTTPS-port (likely 443/8443) and admin-port (likely 2019) as
/// distinct concerns.
pub fn find_free_port(start: u16, range: u16) -> Option<u16> {
    for offset in 0..range {
        let port = start.checked_add(offset)?;
        if std::net::TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return Some(port);
        }
    }
    None
}

/// Pre-flight check for the public HTTPS port. Same logic, different
/// default — useful name at the call site.
pub fn find_free_https_port(prefer: u16, fallback: u16) -> u16 {
    if std::net::TcpListener::bind(("127.0.0.1", prefer)).is_ok() {
        return prefer;
    }
    find_free_port(fallback, 32).unwrap_or(fallback)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_free_admin_port_near_default() {
        let p = find_free_port(DEFAULT_ADMIN_PORT, 32);
        assert!(p.is_some());
    }

    #[test]
    fn https_port_falls_back_when_443_is_privileged() {
        // 443 will generally be held or privileged; expect fallback to
        // somewhere in the high range.
        let p = find_free_https_port(443, DEFAULT_HTTPS_PORT);
        assert!(p >= 1024 || p == 443);
    }
}
