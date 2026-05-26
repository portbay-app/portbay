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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tauri::AppHandle;
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
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
    /// True only while the spawned dnsmasq is actually alive. The Tauri
    /// `CommandChild` handle exists the instant `.spawn()` returns even if
    /// the process exits a millisecond later (e.g. a bad config or arg), so
    /// `child.is_some()` alone is a liar. A background task watching the
    /// sidecar's event stream flips this to `false` on `Terminated`.
    alive: Arc<AtomicBool>,
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
            alive: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn is_running(&self) -> bool {
        self.child.is_some() && self.alive.load(Ordering::Relaxed)
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Spawn the dnsmasq daemon against `config_path`. The port must
    /// match `address=` directive's bound port inside the config — the
    /// caller (`state::boot_dnsmasq`) writes the config with the same
    /// port it passes here.
    pub fn start(&mut self, app: &AppHandle, config_path: &Path, port: u16) -> Result<()> {
        if self.is_running() {
            return Ok(());
        }
        self.port = port;

        let config_str = config_path.to_string_lossy().into_owned();
        let cmd = resolve_command(app, &config_str)?;

        let (mut rx, child) = cmd
            .spawn()
            .map_err(|e| DnsmasqError::SpawnFailed(e.to_string()))?;
        self.alive.store(true, Ordering::Relaxed);
        self.child = Some(child);

        // Drain the sidecar's event stream so dnsmasq's own diagnostics are
        // visible (they were dropped on the floor before — which is exactly
        // why a fatal `junk found in command line` exit went unnoticed), and
        // flip `alive` the moment the process exits so `is_running` can't keep
        // claiming a dead daemon is up.
        let alive = self.alive.clone();
        tauri::async_runtime::spawn(async move {
            while let Some(event) = rx.recv().await {
                match event {
                    CommandEvent::Stderr(bytes) => {
                        let line = String::from_utf8_lossy(&bytes);
                        let line = line.trim_end();
                        if !line.is_empty() {
                            tracing::debug!(target: "dnsmasq", "{line}");
                        }
                    }
                    CommandEvent::Error(err) => {
                        tracing::warn!(target: "dnsmasq", error = %err, "dnsmasq sidecar error");
                    }
                    CommandEvent::Terminated(payload) => {
                        tracing::warn!(target: "dnsmasq", code = ?payload.code, "dnsmasq sidecar terminated");
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

impl Drop for DnsmasqSidecar {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Locate PortBay's bundled dnsmasq next to the running executable. Tauri
/// copies sidecars beside the binary for `cargo run` / `tauri dev` and into
/// the app bundle's MacOS dir for a packaged build, so this finds our OWN
/// dnsmasq in both. Returns `None` if it isn't there.
fn bundled_binary_beside_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    for name in [
        "dnsmasq",
        "dnsmasq-aarch64-apple-darwin",
        "dnsmasq-x86_64-apple-darwin",
    ] {
        let candidate = dir.join(name);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

fn resolve_command(
    app: &AppHandle,
    config_str: &str,
) -> Result<tauri_plugin_shell::process::Command> {
    // dnsmasq is GNU-getopt and only accepts the config flag in its
    // `-C <file>` / `--conf-file=<file>` forms; the space-separated
    // `--conf-file <file>` is parsed as a stray positional argument and
    // aborts startup with "junk found in command line". We pass `-C` as
    // two argv entries (no shell involved, so a path with spaces stays one
    // argument).
    const CONF_FLAG: &str = "-C";

    // 1. The Tauri sidecar slot (works once `binaries/dnsmasq` is in the
    //    shell allow-spawn capability).
    if let Ok(sidecar) = app.shell().sidecar("dnsmasq") {
        return Ok(sidecar.args([CONF_FLAG, config_str]));
    }

    // 2. Our bundled binary, by absolute path, via the general
    //    `shell:allow-execute` permission. This keeps PortBay on its OWN
    //    dnsmasq even when the sidecar scope isn't wired — and never reaches
    //    for a foreign binary (ServBay/Homebrew).
    if let Some(path) = bundled_binary_beside_exe() {
        return Ok(app
            .shell()
            .command(path.to_string_lossy().into_owned())
            .args([CONF_FLAG, config_str]));
    }

    // 3. Last resort: a `dnsmasq` on PATH (dev machines with Homebrew, etc.).
    let path = which::which("dnsmasq").map_err(|_| DnsmasqError::BinaryMissing)?;
    Ok(app
        .shell()
        .command(path.to_string_lossy().into_owned())
        .args([CONF_FLAG, config_str]))
}

/// Resolve a free local port for dnsmasq. Scans `start..start+range`, skipping
/// any port in `avoid` (typically the registered projects' declared ports) and
/// returns the first that's free on **both UDP and TCP** — dnsmasq binds both,
/// and DNS is primarily UDP, so a TCP-only probe (the previous behaviour) would
/// happily hand back a port another DNS daemon already holds on UDP, and our
/// dnsmasq would then fail to bind silently.
pub fn find_free_port(start: u16, range: u16, avoid: &[u16]) -> Option<u16> {
    for offset in 0..range {
        let port = start.checked_add(offset)?;
        if avoid.contains(&port) {
            continue;
        }
        let udp_ok = std::net::UdpSocket::bind(("127.0.0.1", port)).is_ok();
        let tcp_ok = std::net::TcpListener::bind(("127.0.0.1", port)).is_ok();
        if udp_ok && tcp_ok {
            return Some(port);
        }
    }
    None
}

/// True iff a `dnsmasq` binary can be found. Prefers the **bundled** sidecar
/// — that's what `start` uses and what makes PortBay self-contained — and
/// only falls back to a PATH lookup as a dev convenience.
///
/// The previous PATH-only check was wrong on two counts: a packaged GUI app's
/// `PATH` usually omits Homebrew/ServBay dirs, so it returned `false` and the
/// daemon never booted on a clean machine; and depending on a foreign
/// `dnsmasq` on `PATH` (e.g. ServBay's) is exactly what we must not do.
pub fn binary_available(app: &AppHandle) -> bool {
    app.shell().sidecar("dnsmasq").is_ok()
        || bundled_binary_beside_exe().is_some()
        || which::which("dnsmasq").is_ok()
}

#[allow(dead_code)]
fn _typecheck_pathbuf_export() -> PathBuf {
    PathBuf::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_free_port_near_default() {
        let port = find_free_port(DEFAULT_PORT, 32, &[]);
        assert!(port.is_some());
    }
}
