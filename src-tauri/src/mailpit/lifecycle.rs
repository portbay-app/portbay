//! Mailpit sidecar lifecycle.
//!
//! Mailpit ships as a single Go binary with no daemon mode — running
//! it in the foreground is the only mode. We bind both listeners
//! (SMTP and web UI) to `127.0.0.1` and let it persist its message
//! store at `<data_dir>/PortBay/mailpit.db`.

use std::path::{Path, PathBuf};

use tauri::AppHandle;
use tauri_plugin_shell::process::CommandChild;
use tauri_plugin_shell::ShellExt;

use crate::mailpit::error::{MailpitError, Result};

/// Default SMTP listen port. Matches Mailpit's own default and the
/// `MAIL_PORT` value most Laravel / Symfony / Rails defaults expect.
pub const DEFAULT_SMTP_PORT: u16 = 1025;

/// Default web UI port. Matches Mailpit's upstream default.
pub const DEFAULT_UI_PORT: u16 = 8025;

/// Number of ports to scan upward if the defaults are taken.
pub const PORT_SCAN_RANGE: u16 = 16;

#[derive(Debug)]
pub struct MailpitSidecar {
    child: Option<CommandChild>,
    smtp_port: u16,
    ui_port: u16,
}

impl Default for MailpitSidecar {
    fn default() -> Self {
        Self::new()
    }
}

impl MailpitSidecar {
    pub fn new() -> Self {
        Self {
            child: None,
            smtp_port: DEFAULT_SMTP_PORT,
            ui_port: DEFAULT_UI_PORT,
        }
    }

    pub fn is_running(&self) -> bool {
        self.child.is_some()
    }

    pub fn smtp_port(&self) -> u16 {
        self.smtp_port
    }

    pub fn ui_port(&self) -> u16 {
        self.ui_port
    }

    /// Spawn Mailpit. Caller picks the ports + persistent DB path; the
    /// helper records them so the sidecar status surface can render
    /// "listening on smtp :1025 / ui :8025" without re-querying.
    pub fn start(
        &mut self,
        app: &AppHandle,
        smtp_port: u16,
        ui_port: u16,
        db_path: &Path,
    ) -> Result<()> {
        if self.child.is_some() {
            return Ok(());
        }
        self.smtp_port = smtp_port;
        self.ui_port = ui_port;

        let db = db_path.to_string_lossy().into_owned();
        let smtp = format!("127.0.0.1:{smtp_port}");
        let ui = format!("127.0.0.1:{ui_port}");

        let cmd = resolve_command(app, &smtp, &ui, &db)?;

        let (_rx, child) = cmd
            .spawn()
            .map_err(|e| MailpitError::SpawnFailed(e.to_string()))?;
        self.child = Some(child);
        Ok(())
    }

    pub fn stop(&mut self) {
        if let Some(child) = self.child.take() {
            let _ = child.kill();
        }
    }
}

impl Drop for MailpitSidecar {
    fn drop(&mut self) {
        self.stop();
    }
}

fn resolve_command(
    app: &AppHandle,
    smtp: &str,
    ui: &str,
    db: &str,
) -> Result<tauri_plugin_shell::process::Command> {
    let args = [
        "--smtp",
        smtp,
        "--listen",
        ui,
        "--db-file",
        db,
        // Friendly defaults: 1000 message cap with auto-rotation.
        "--max",
        "1000",
    ];

    if let Ok(sidecar) = app.shell().sidecar("mailpit") {
        return Ok(sidecar.args(args));
    }

    let path = which::which("mailpit").map_err(|_| MailpitError::BinaryMissing)?;
    Ok(app
        .shell()
        .command(path.to_string_lossy().into_owned())
        .args(args))
}

/// Scan loopback for a free TCP port. Mailpit binds both SMTP and the
/// web UI; we run the scan for each port independently so a collision
/// on one doesn't force the other to a non-standard value.
pub fn find_free_port(start: u16, range: u16) -> Option<u16> {
    for offset in 0..range {
        let port = start.checked_add(offset)?;
        if std::net::TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return Some(port);
        }
    }
    None
}

/// True iff a `mailpit` binary can be found via PATH. Used by the
/// sidecar-status helper to flag NotInstalled cleanly without trying
/// to spawn.
pub fn binary_available(_app: &AppHandle) -> bool {
    which::which("mailpit").is_ok()
}

/// Default persistent DB path. `<data_dir>/PortBay/mailpit.db`.
pub fn default_db_path() -> std::io::Result<PathBuf> {
    let mut dir = dirs::data_dir()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no data dir"))?;
    dir.push("PortBay");
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("mailpit.db"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_free_port_near_default() {
        let port = find_free_port(DEFAULT_SMTP_PORT, 16);
        assert!(port.is_some());
    }
}
