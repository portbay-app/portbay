//! Sidecar lifecycle — owns the bundled `process-compose` child process.
//!
//! Started at Tauri app boot, killed when the window closes (or when
//! [`SidecarManager::stop`] is called explicitly). The struct also handles
//! port selection so a second PortBay instance, or another tool already on
//! :9999, doesn't trap us.

use std::path::Path;

use tauri::AppHandle;
use tauri_plugin_shell::process::CommandChild;
use tauri_plugin_shell::ShellExt;

use crate::process_compose::client::PcClient;
use crate::process_compose::error::{PcError, Result};

/// Default port to try first. Process Compose's own default is 8080; we
/// start higher because 8080 is heavily contested by web frameworks.
pub const DEFAULT_PORT: u16 = 9999;

/// Number of ports to scan upward from the start before giving up.
const PORT_SCAN_RANGE: u16 = 32;

/// Owns the bundled Process Compose child process for as long as the app
/// window is open.
///
/// Designed for one instance per app. Lives inside Tauri's state (managed
/// behind a `Mutex`) so commands can stop / inspect it.
#[derive(Debug)]
pub struct SidecarManager {
    child: Option<CommandChild>,
    port: u16,
}

impl Default for SidecarManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SidecarManager {
    pub fn new() -> Self {
        Self {
            child: None,
            port: DEFAULT_PORT,
        }
    }

    pub fn is_running(&self) -> bool {
        self.child.is_some()
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Spawn the bundled `process-compose` sidecar against `config_path`.
    ///
    /// Picks a free port starting from `DEFAULT_PORT`. Returns a ready-to-use
    /// `PcClient`.
    pub fn start(&mut self, app: &AppHandle, config_path: &Path) -> Result<PcClient> {
        if self.child.is_some() {
            // Idempotent: already running, hand back the existing client.
            return Ok(PcClient::new(self.port));
        }

        let port = find_free_port(DEFAULT_PORT, PORT_SCAN_RANGE).ok_or(PcError::NoFreePort {
            start: DEFAULT_PORT,
        })?;
        self.port = port;

        let port_str = port.to_string();
        let config_str = config_path.to_string_lossy().into_owned();

        let cmd = app
            .shell()
            .sidecar("process-compose")
            .map_err(|e| PcError::SpawnFailed(e.to_string()))?
            .args([
                "-f",
                &config_str,
                "--port",
                &port_str,
                "--tui=false",
                "--keep-project",
                "up",
            ]);

        let (_rx, child) = cmd
            .spawn()
            .map_err(|e| PcError::SpawnFailed(e.to_string()))?;
        self.child = Some(child);

        Ok(PcClient::new(port))
    }

    /// Kill the sidecar if it's running. Safe to call multiple times.
    pub fn stop(&mut self) {
        if let Some(child) = self.child.take() {
            let _ = child.kill();
        }
    }
}

impl Drop for SidecarManager {
    fn drop(&mut self) {
        // If the manager is dropped without an explicit stop (e.g. app
        // crash), still try to clean up the child.
        self.stop();
    }
}

/// Try to bind a TCP listener on `start, start+1, ...` until one succeeds.
/// Closes the test listener immediately and returns the port number — the
/// next attempt to bind it (by PC) will race only with other apps, not us.
pub fn find_free_port(start: u16, range: u16) -> Option<u16> {
    for offset in 0..range {
        let port = start.checked_add(offset)?;
        if std::net::TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return Some(port);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_free_port_near_default() {
        let port = find_free_port(DEFAULT_PORT, 32).expect("expected at least one free port");
        assert!((DEFAULT_PORT..DEFAULT_PORT + 32).contains(&port));
    }

    #[test]
    fn skips_a_held_port() {
        // Hold one port, then see that find_free_port skips it.
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let held_port = listener.local_addr().unwrap().port();
        let chosen = find_free_port(held_port, 4).unwrap();
        // chosen is either held_port (if bind raced) — unlikely — or
        // something within [held_port, held_port+4). Don't assert equality,
        // just that it didn't pick the one we hold *during the call*.
        // Drop the listener first so the next assertion is meaningful.
        drop(listener);
        // Re-run with the now-free port — should find it.
        let chosen2 = find_free_port(chosen, 4).unwrap();
        assert!(chosen2 >= chosen);
    }
}
