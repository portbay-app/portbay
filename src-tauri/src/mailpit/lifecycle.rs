//! Mailpit sidecar lifecycle.
//!
//! Mailpit ships as a single Go binary with no daemon mode — running
//! it in the foreground is the only mode. We bind both listeners
//! (SMTP and web UI) to `127.0.0.1` and let it persist its message
//! store at `<data_dir>/PortBay/mailpit.db`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tauri::AppHandle;
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
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
    /// True only while the spawned Mailpit is actually alive. `.spawn()`
    /// returns a handle even if Mailpit exits immediately (e.g. a port already
    /// bound), so `child.is_some()` alone is a liar. A background task watching
    /// the event stream flips this to `false` on `Terminated` — and also lets
    /// Mailpit's own stderr surface instead of being dropped on the floor.
    alive: Arc<AtomicBool>,
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
            alive: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn is_running(&self) -> bool {
        self.child.is_some() && self.alive.load(Ordering::Relaxed)
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
        // is_running() (not child.is_some()) so a crashed Mailpit is respawned
        // rather than wrongly treated as still up.
        if self.is_running() {
            return Ok(());
        }
        self.smtp_port = smtp_port;
        self.ui_port = ui_port;

        let db = db_path.to_string_lossy().into_owned();
        let smtp = format!("127.0.0.1:{smtp_port}");
        let ui = format!("127.0.0.1:{ui_port}");

        let cmd = resolve_command(app, &smtp, &ui, &db)?;

        let (mut rx, child) = cmd
            .spawn()
            .map_err(|e| MailpitError::SpawnFailed(e.to_string()))?;
        self.alive.store(true, Ordering::Relaxed);
        self.child = Some(child);

        // Drain the event stream so Mailpit's diagnostics surface, and flip
        // `alive` the moment it exits so `is_running` can't keep claiming a
        // dead mailer is up.
        let alive = self.alive.clone();
        tauri::async_runtime::spawn(async move {
            while let Some(event) = rx.recv().await {
                match event {
                    CommandEvent::Stderr(bytes) => {
                        let line = String::from_utf8_lossy(&bytes);
                        let line = line.trim_end();
                        if !line.is_empty() {
                            tracing::debug!(target: "mailpit", "{line}");
                        }
                    }
                    CommandEvent::Error(err) => {
                        tracing::warn!(target: "mailpit", error = %err, "mailpit sidecar error");
                    }
                    CommandEvent::Terminated(payload) => {
                        tracing::warn!(target: "mailpit", code = ?payload.code, "mailpit sidecar terminated");
                        break;
                    }
                    _ => {}
                }
            }
            alive.store(false, Ordering::Relaxed);
        });
        Ok(())
    }

    pub fn stop(&mut self) {
        self.alive.store(false, Ordering::Relaxed);
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

/// Scan loopback for a free TCP port, skipping any in `avoid`. Mailpit binds
/// both SMTP and the web UI; we run the scan for each port independently so a
/// collision on one doesn't force the other to a non-standard value. `avoid`
/// carries the registered projects' ports so Mailpit never claims a port a
/// user's dev server expects — its default ranges (1025–1040 SMTP, 8025–8040
/// UI) sit squarely in dev-server territory.
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

/// True iff a usable `mailpit` binary exists. Checks the **bundled sidecar
/// first** (the shipped binary lives in the app bundle, never on `PATH`), then
/// falls back to `PATH`. The previous PATH-only check made the bundled binary
/// invisible: `boot_mailpit` short-circuited and the UI told users to
/// `brew install mailpit` even though Mailpit was sitting right there in the
/// bundle. Mirrors `dnsmasq::binary_available`.
pub fn binary_available(app: &AppHandle) -> bool {
    app.shell().sidecar("mailpit").is_ok() || which::which("mailpit").is_ok()
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
        let port = find_free_port(DEFAULT_SMTP_PORT, 16, &[]);
        assert!(port.is_some());
    }

    #[test]
    fn skips_avoided_ports() {
        // With the first two candidates in the avoid set, the scan must return
        // a port at or beyond start+2 — never one a project already claims.
        let avoid = [DEFAULT_SMTP_PORT, DEFAULT_SMTP_PORT + 1];
        let port = find_free_port(DEFAULT_SMTP_PORT, 16, &avoid).expect("a free port");
        assert!(
            port >= DEFAULT_SMTP_PORT + 2,
            "must skip avoided ports, got {port}"
        );
    }
}
