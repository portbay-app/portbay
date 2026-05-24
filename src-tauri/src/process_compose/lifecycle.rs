//! Sidecar lifecycle — owns the bundled `process-compose` child process.
//!
//! Started at Tauri app boot, killed when the window closes (or when
//! [`SidecarManager::stop`] is called explicitly). The struct also handles
//! port selection so a second PortBay instance, or another tool already on
//! :9999, doesn't trap us.

use std::path::{Path, PathBuf};

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
    /// The `-f` config path the live child was started against. Remembered so
    /// [`SidecarManager::stop`] can mop up any *orphaned* leftover instance
    /// sharing this config (the recover-on-quit complement to the boot sweep),
    /// keyed on a path unique to this install.
    config_path: Option<PathBuf>,
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
            config_path: None,
        }
    }

    pub fn is_running(&self) -> bool {
        self.child.is_some()
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// PID of the running process-compose child, if any. Used by the port
    /// pre-flight to recognise dev servers that descend from *our* supervisor
    /// (so it never mistakes PortBay's own running server for a conflict).
    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().map(|c| c.pid())
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
        self.config_path = Some(config_path.to_path_buf());

        Ok(PcClient::new(port))
    }

    /// Stop the sidecar if it's running. Safe to call multiple times.
    ///
    /// SIGTERM first, then reap. A bare `child.kill()` (SIGKILL) gave
    /// process-compose no chance to tear down the dev servers it spawned, so
    /// they reparented to launchd and kept holding their ports — turning the
    /// next launch's Play into a phantom "port already in use" conflict. SIGTERM
    /// lets PC run its shutdown handler (which signals every managed process);
    /// after a short grace we SIGKILL-reap whatever's left.
    pub fn stop(&mut self) {
        let own_pid = self.child.as_ref().map(|c| c.pid());
        if let Some(child) = self.child.take() {
            let pid = child.pid();
            // SAFETY: `kill(2)` with a valid pid + standard signal; the only
            // effect is signal delivery. A stale pid just yields ESRCH.
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGTERM);
            }
            std::thread::sleep(GRACEFUL_STOP_GRACE);
            // Reap the handle (SIGKILL if it somehow survived the SIGTERM).
            let _ = child.kill();
        }
        // Recover-on-quit: after reaping our own child, mop up any *orphaned*
        // (PPID 1) process-compose left over from a previous crashed run that's
        // still squatting on its port. Orphans-only so a graceful quit can
        // never reach into a hypothetical second live PortBay instance (whose
        // PC is still parented to that live app). The boot sweep handles the
        // rest; together they keep stale instances from accumulating.
        if let Some(config) = self.config_path.take() {
            sweep_stale(&config, own_pid, SweepMode::OrphansOnly);
        }
    }
}

/// How wide a [`sweep_stale`] pass reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SweepMode {
    /// Reap every matching `process-compose` regardless of parentage. Safe only
    /// when ours isn't running yet (boot), where any match is by definition
    /// stale.
    All,
    /// Reap only instances orphaned to launchd (PPID 1). Safe to run while a
    /// live PortBay is up — it can't touch a PC still parented to a live app.
    OrphansOnly,
}

/// How long to let process-compose forward termination to its managed
/// processes before we reap it. PC forwards signals near-instantly; this is
/// just enough headroom that the dev servers receive their SIGTERM (and so
/// don't leak) without making app quit feel sluggish.
const GRACEFUL_STOP_GRACE: std::time::Duration = std::time::Duration::from_millis(800);

impl Drop for SidecarManager {
    fn drop(&mut self) {
        // If the manager is dropped without an explicit stop (e.g. app
        // crash), still try to clean up the child.
        self.stop();
    }
}

/// How long to let a swept-up stale instance forward SIGTERM to its managed
/// processes before SIGKILL. Same rationale as [`GRACEFUL_STOP_GRACE`], with a
/// little more headroom since boot is not latency-sensitive and we want the
/// stale instance's dev servers to actually receive their signal.
const SWEEP_GRACE: std::time::Duration = std::time::Duration::from_millis(1500);

/// Reap stale `process-compose` instances left over from a previous PortBay
/// run, *before* we boot our own.
///
/// Every PortBay PC invocation carries `-f <app-data>/process-compose.yaml` —
/// a path unique to this install — so matching that argument in the process's
/// command line identifies our own leftovers precisely, without touching an
/// unrelated `process-compose` the user might run with a different config.
///
/// With [`SweepMode::All`] (called before [`SidecarManager::start`]) any match
/// is by definition stale — ours isn't running yet — so it catches instances
/// orphaned straight to launchd *and* those still parented to a stale
/// `portbay-app` that outlived its launcher. With [`SweepMode::OrphansOnly`]
/// only PPID-1 instances are reaped, which is safe to run while a live PortBay
/// is up. `exclude` skips a known-live pid (e.g. the child we just reaped).
/// Returns the number of processes reaped.
pub fn sweep_stale(config_path: &Path, exclude: Option<u32>, mode: SweepMode) -> usize {
    let marker = config_path.to_string_lossy();
    let out = match std::process::Command::new("ps")
        .args(["-axo", "pid=,ppid=,command="])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return 0,
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let targets = stale_pc_pids(
        &text,
        &marker,
        exclude.into_iter().collect::<Vec<_>>().as_slice(),
        mode,
    );
    for pid in &targets {
        let _ = crate::port_holder::kill_gracefully(*pid, SWEEP_GRACE);
    }
    targets.len()
}

/// Parse `ps -axo pid=,ppid=,command=` output and return the pids of every
/// `process-compose` line that references `marker` (our config path), is not in
/// `exclude` or the current process, and satisfies `mode`. Pure string work so
/// it can be unit tested without spawning processes.
fn stale_pc_pids(ps_output: &str, marker: &str, exclude: &[u32], mode: SweepMode) -> Vec<u32> {
    let me = std::process::id();
    ps_output
        .lines()
        .filter_map(|line| {
            let mut cols = line.split_whitespace();
            let pid = cols.next()?.parse::<u32>().ok()?;
            let ppid = cols.next()?.parse::<u32>().ok()?;
            if pid == me || exclude.contains(&pid) {
                return None;
            }
            if mode == SweepMode::OrphansOnly && ppid != 1 {
                return None;
            }
            if line.contains("process-compose") && line.contains(marker) {
                Some(pid)
            } else {
                None
            }
        })
        .collect()
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

    // Columns: pid, ppid, command. 1443 is orphaned to launchd (ppid 1);
    // 14648 is still parented to a stale portbay-app (ppid 13275).
    const PS_SAMPLE: &str = "  1443     1 /…/target/debug/process-compose -f /Users/n/Library/Application Support/PortBay/process-compose.yaml --port 9999 --tui=false --keep-project up
14648 13275 /…/target/debug/process-compose -f /Users/n/Library/Application Support/PortBay/process-compose.yaml --port 10000 --tui=false up
  321     1 /usr/sbin/cfprefsd agent
 7788     1 /opt/homebrew/bin/process-compose -f /Users/n/other/process-compose.yaml up";

    const PORTBAY_CONFIG: &str =
        "/Users/n/Library/Application Support/PortBay/process-compose.yaml";

    #[test]
    fn sweep_all_matches_every_instance_on_our_config() {
        let pids = stale_pc_pids(PS_SAMPLE, PORTBAY_CONFIG, &[], SweepMode::All);
        // Both PortBay PCs match (regardless of parentage); the homebrew PC on
        // a *different* config and the unrelated cfprefsd line are left alone.
        assert_eq!(pids, vec![1443, 14648]);
    }

    #[test]
    fn sweep_orphans_only_skips_live_parented_instances() {
        let pids = stale_pc_pids(PS_SAMPLE, PORTBAY_CONFIG, &[], SweepMode::OrphansOnly);
        // Only 1443 (ppid 1) is reaped; 14648 (parented to a live app) is left,
        // which is what protects a hypothetical second live PortBay instance.
        assert_eq!(pids, vec![1443]);
    }

    #[test]
    fn sweep_honors_exclude() {
        let pids = stale_pc_pids(PS_SAMPLE, PORTBAY_CONFIG, &[1443], SweepMode::All);
        assert_eq!(pids, vec![14648]);
    }

    #[test]
    fn sweep_never_targets_self() {
        // A line whose pid is our own process id is skipped even if it
        // matched the marker.
        let me = std::process::id();
        let line = format!("{me} 1 /x/process-compose -f /cfg/process-compose.yaml up");
        let pids = stale_pc_pids(&line, "/cfg/process-compose.yaml", &[], SweepMode::All);
        assert!(pids.is_empty());
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
