//! dnsmasq sidecar lifecycle.
//!
//! Parallel structure to `caddy::lifecycle::CaddySidecar`. The dnsmasq
//! daemon answers DNS queries on a non-privileged loopback port; the
//! `/etc/resolver/<suffix>` file (written separately by the resolver-
//! install card) tells macOS to route only `.<suffix>` queries here.
//!
//! Binary resolution at spawn time mirrors `mkcert`'s pattern:
//!
//! 1. Bundled sidecar at `binaries/dnsmasq-<triple>` (production path).
//! 2. Next to the running executable.
//! 3. `which::which("dnsmasq")` on PATH (the dev fallback that the
//!    Homebrew or ServBay install already satisfies).
//!
//! If none of the above resolves, `start` returns `BinaryMissing` and
//! the GUI surfaces the missing-binary state via the sidecar slot.

use std::path::{Path, PathBuf};

use tauri::AppHandle;
use tauri_plugin_shell::process::CommandChild;
use tauri_plugin_shell::ShellExt;

use crate::dnsmasq::error::{DnsmasqError, Result};

/// Default UDP port for dnsmasq. Picked high enough that any process
/// running as the user can bind without sudo; low enough to stay clear
/// of the ephemeral-port range.
pub const DEFAULT_PORT: u16 = 53053;

/// Number of ports to scan if the default is taken.
pub const PORT_SCAN_RANGE: u16 = 32;

#[derive(Debug)]
pub struct DnsmasqSidecar {
    child: Option<CommandChild>,
    port: u16,
}

impl Default for DnsmasqSidecar {
    fn default() -> Self {
        Self::new()
    }
}

impl DnsmasqSidecar {
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

    /// Spawn the dnsmasq daemon against `config_path`. The port must
    /// match `address=` directive's bound port inside the config — the
    /// caller (`state::boot_dnsmasq`) writes the config with the same
    /// port it passes here.
    pub fn start(&mut self, app: &AppHandle, config_path: &Path, port: u16) -> Result<()> {
        if self.child.is_some() {
            return Ok(());
        }
        self.port = port;

        let config_str = config_path.to_string_lossy().into_owned();
        let cmd = resolve_command(app, &config_str)?;

        let (_rx, child) = cmd
            .spawn()
            .map_err(|e| DnsmasqError::SpawnFailed(e.to_string()))?;
        self.child = Some(child);
        Ok(())
    }

    pub fn stop(&mut self) {
        if let Some(child) = self.child.take() {
            let _ = child.kill();
        }
    }
}

impl Drop for DnsmasqSidecar {
    fn drop(&mut self) {
        self.stop();
    }
}

fn resolve_command(
    app: &AppHandle,
    config_str: &str,
) -> Result<tauri_plugin_shell::process::Command> {
    // Try the bundled sidecar first. The shell plugin only surfaces a
    // success once it has resolved a real binary on disk, so we can
    // fall through to PATH cleanly on Err.
    if let Ok(sidecar) = app.shell().sidecar("dnsmasq") {
        return Ok(sidecar.args(["--conf-file", config_str]));
    }

    // Fall back to the system binary on PATH (Homebrew, ServBay, etc.).
    let path = which::which("dnsmasq").map_err(|_| DnsmasqError::BinaryMissing)?;
    Ok(app
        .shell()
        .command(path.to_string_lossy().into_owned())
        .args(["--conf-file", config_str]))
}

/// Resolve a free local port for dnsmasq. Scans `start..start+range`
/// and returns the first that binds. UDP-only here matches the daemon's
/// actual listening protocol; we attempt a TCP bind as a cheap
/// availability proxy (most daemons that hold a port hold both).
pub fn find_free_port(start: u16, range: u16) -> Option<u16> {
    for offset in 0..range {
        let port = start.checked_add(offset)?;
        if std::net::TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return Some(port);
        }
    }
    None
}

/// True iff a `dnsmasq` binary can be found via the sidecar slot or PATH.
/// Cheap; used by the sidecar-status command to flag NotInstalled.
pub fn binary_available(_app: &AppHandle) -> bool {
    // Sidecar resolution requires a live AppHandle; the shell plugin's
    // sidecar lookup at startup is not stable enough to use cheaply
    // here, so we lean on PATH only for the "is anything available?"
    // status check. The actual `start` still tries sidecar first.
    which::which("dnsmasq").is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_free_port_near_default() {
        let port = find_free_port(DEFAULT_PORT, 32);
        assert!(port.is_some());
    }
}

#[allow(dead_code)]
fn _typecheck_pathbuf_export() -> PathBuf {
    PathBuf::new()
}
