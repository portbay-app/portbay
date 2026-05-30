//! `cloudflared` child-process lifecycle + per-project tunnel state.
//!
//! Each call to [`TunnelManager::start`] spawns one cloudflared child
//! pointed at the project's local URL. A background task tails the
//! child's stdout/stderr, parses the assigned `trycloudflare.com` URL,
//! and stores it on the `Tunnel` record so subsequent `status` calls
//! return it immediately.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tauri_plugin_shell::process::CommandChild;
use tauri_plugin_shell::ShellExt;

use crate::tunnel::error::{Result, TunnelError};

/// How long the start path waits for cloudflared to announce a public
/// URL on stdout before giving up. Real-world: tunnels usually appear
/// within 2–6 s; 20 s leaves headroom for slow connections.
pub const TUNNEL_URL_TIMEOUT: Duration = Duration::from_secs(20);

/// Public view of one running tunnel — what the GUI / list command
/// renders. Cheap to clone. `Deserialize` too, so a separate process (the
/// CLI / MCP server) can read the state file the app mirrors tunnels into
/// (see [`write_state`] / [`read_state`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TunnelStatus {
    pub project_id: String,
    pub upstream_url: String,
    pub public_url: Option<String>,
    /// True iff the child process is still alive (we don't reap until
    /// the next `start` / `stop` call observes the exit).
    pub running: bool,
    /// Whether the local origin the tunnel points at is actually reachable.
    /// `None` until probed (the manager leaves it unset; the async command
    /// layer fills it). `Some(false)` means the cloudflared process is alive
    /// but visitors would get errors — an honest "degraded" signal instead of
    /// a misleading green "running".
    pub origin_reachable: Option<bool>,
    /// Wall-clock ms when the tunnel started.
    pub started_at_ms: u64,
    /// `true` for a bring-your-own **named** tunnel (stable custom hostname),
    /// `false` for a quick ephemeral `*.trycloudflare.com` share. Serde-default
    /// so an older state file (written before custom tunnels) reads as quick.
    #[serde(default)]
    pub custom: bool,
}

/// One live tunnel. The `child` keeps cloudflared alive for the life
/// of the `Tunnel`; `Drop` kills the child so a poisoned mutex or app
/// shutdown never leaks the process.
#[derive(Debug)]
pub struct Tunnel {
    pub project_id: String,
    pub upstream_url: String,
    pub public_url: Arc<Mutex<Option<String>>>,
    pub started_at_ms: u64,
    custom: bool,
    child: Option<CommandChild>,
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        if let Some(child) = self.child.take() {
            let _ = child.kill();
        }
    }
}

impl Tunnel {
    fn status(&self) -> TunnelStatus {
        TunnelStatus {
            project_id: self.project_id.clone(),
            upstream_url: self.upstream_url.clone(),
            public_url: self
                .public_url
                .lock()
                .expect("public_url mutex poisoned")
                .clone(),
            running: self.child.is_some(),
            origin_reachable: None,
            started_at_ms: self.started_at_ms,
            custom: self.custom,
        }
    }
}

/// One manager per app instance. Owns the per-project tunnel map and
/// dispatches start/stop/list operations.
#[derive(Debug, Default)]
pub struct TunnelManager {
    tunnels: HashMap<String, Tunnel>,
}

impl TunnelManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_running(&self, project_id: &str) -> bool {
        self.tunnels.contains_key(project_id)
    }

    pub fn count(&self) -> usize {
        self.tunnels.len()
    }

    pub fn list(&self) -> Vec<TunnelStatus> {
        let mut out: Vec<TunnelStatus> = self.tunnels.values().map(|t| t.status()).collect();
        out.sort_by(|a, b| a.project_id.cmp(&b.project_id));
        out
    }

    pub fn status(&self, project_id: &str) -> Option<TunnelStatus> {
        self.tunnels.get(project_id).map(|t| t.status())
    }

    /// Start a **quick** (ephemeral) tunnel, routing traffic through Caddy so the
    /// per-project Origin/Host normalisation applies.
    ///
    /// `hostname` is the project's Caddy hostname (e.g. `myapp.test`); it is
    /// passed as `--http-host-header` so Caddy matches the correct route.
    /// `upstream_url` is the local URL cloudflared points at — `start_tunnel`
    /// passes Caddy's plain-HTTP `:80` listener (`http://127.0.0.1:80`), which
    /// while a share is active serves that project with normalisation instead
    /// of redirecting to https; reaching Caddy's TLS port by IP can't carry SNI,
    /// so Caddy would have no cert to present and the handshake would 502.
    ///
    /// The returned status reflects the just-started state — `public_url`
    /// is initially `None`. Callers poll `status` until the URL is
    /// populated (the stdout-tail task fills it in).
    pub fn start(
        &mut self,
        app: &AppHandle,
        project_id: &str,
        hostname: &str,
        upstream_url: &str,
    ) -> Result<TunnelStatus> {
        let cmd = resolve_command(app, upstream_url, hostname)?;
        self.spawn(project_id, upstream_url, cmd, None, false)
    }

    /// Start a bring-your-own **named** tunnel from a PortBay-owned config
    /// (`config_path`). The public URL is the user's stable hostname — known
    /// up-front, so it's pre-populated rather than parsed from stdout.
    /// `upstream_url` is the local origin the tunnel's ingress points at (for the
    /// reachability probe + status display).
    pub fn start_custom(
        &mut self,
        app: &AppHandle,
        project_id: &str,
        config_path: &std::path::Path,
        upstream_url: &str,
        public_url: String,
    ) -> Result<TunnelStatus> {
        let cmd = resolve_custom_command(app, config_path)?;
        self.spawn(project_id, upstream_url, cmd, Some(public_url), true)
    }

    /// Shared spawn path for both tunnel kinds: register the cloudflared child,
    /// tail its output (parsing the quick-share URL when `preset_url` is `None`),
    /// and record the `Tunnel`. `custom` flags the kind for status/state.
    fn spawn(
        &mut self,
        project_id: &str,
        upstream_url: &str,
        cmd: tauri_plugin_shell::process::Command,
        preset_url: Option<String>,
        custom: bool,
    ) -> Result<TunnelStatus> {
        if self.tunnels.contains_key(project_id) {
            return Err(TunnelError::AlreadyRunning(project_id.to_string()));
        }

        let (mut rx, child) = cmd
            .spawn()
            .map_err(|e| TunnelError::SpawnFailed(e.to_string()))?;

        let public_url = Arc::new(Mutex::new(preset_url));
        let public_url_for_task = public_url.clone();

        // Tail the child's output and fill in the public URL once cloudflared
        // announces it (quick share only — a named tunnel's URL is preset, and
        // `parse_public_url` won't match a custom domain). Closes naturally when
        // the child exits and the receiver is drained.
        tauri::async_runtime::spawn(async move {
            use tauri_plugin_shell::process::CommandEvent;
            while let Some(event) = rx.recv().await {
                let line = match event {
                    CommandEvent::Stdout(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                    CommandEvent::Stderr(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                    _ => continue,
                };
                if let Some(url) = parse_public_url(&line) {
                    let mut guard = public_url_for_task
                        .lock()
                        .expect("public_url mutex poisoned");
                    if guard.is_none() {
                        *guard = Some(url);
                    }
                }
            }
        });

        let started_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let tunnel = Tunnel {
            project_id: project_id.to_string(),
            upstream_url: upstream_url.to_string(),
            public_url,
            started_at_ms,
            custom,
            child: Some(child),
        };
        let status = tunnel.status();
        self.tunnels.insert(project_id.to_string(), tunnel);
        Ok(status)
    }

    /// Kill the tunnel for `project_id`. Returns `NotRunning` if no
    /// such tunnel exists.
    pub fn stop(&mut self, project_id: &str) -> Result<()> {
        let tunnel = self
            .tunnels
            .remove(project_id)
            .ok_or_else(|| TunnelError::NotRunning(project_id.to_string()))?;
        // The `Drop` on Tunnel kills the child.
        drop(tunnel);
        Ok(())
    }

    /// Kill every running tunnel (Stop All / shutdown). Returns how many were
    /// stopped. Clearing the map drops each `Tunnel`, whose `Drop` kills its
    /// cloudflared child — so no share outlives a Stop All.
    pub fn stop_all(&mut self) -> usize {
        let n = self.tunnels.len();
        self.tunnels.clear();
        n
    }

    /// Return the inner `Arc<Mutex<Option<String>>>` so callers can
    /// poll for the public URL *without* holding the `TunnelManager`
    /// lock across the await — which would deadlock since the
    /// stdout-tail task also needs the inner mutex to write the URL,
    /// and `MutexGuard` isn't `Send`.
    pub fn url_handle(&self, project_id: &str) -> Result<Arc<Mutex<Option<String>>>> {
        Ok(self
            .tunnels
            .get(project_id)
            .ok_or_else(|| TunnelError::NotRunning(project_id.to_string()))?
            .public_url
            .clone())
    }
}

/// Block until the public URL inside `handle` is populated, polling
/// at 200 ms intervals up to [`TUNNEL_URL_TIMEOUT`]. Returns
/// `UrlTimeout` if cloudflared never announces. Lives outside
/// [`TunnelManager`] so the caller doesn't have to hold the
/// manager-level lock across the await.
pub async fn wait_for_url(handle: Arc<Mutex<Option<String>>>) -> Result<String> {
    let deadline = Instant::now() + TUNNEL_URL_TIMEOUT;
    loop {
        if let Some(url) = handle.lock().expect("public_url mutex poisoned").clone() {
            return Ok(url);
        }
        if Instant::now() >= deadline {
            return Err(TunnelError::UrlTimeout);
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// Write a minimal cloudflared config file that intentionally contains only
/// a comment, so cloudflared's own config loading is satisfied (it won't
/// complain about an empty file) while inheriting nothing from the user's
/// `~/.cloudflared/config.yml`. This preserves the isolation invariant
/// established for quick tunnels: PortBay-spawned cloudflared processes never
/// pick up the user's named-tunnel credentials or ingress rules.
/// Path to the isolated quick-tunnel config — pure computation, no I/O. Every
/// PortBay-spawned cloudflared passes this via `--config`, so it doubles as the
/// unique marker for spotting our leftovers in `ps` output ([`sweep_stale_cloudflared`]).
/// `None` only when there's no platform data dir.
fn quick_tunnel_config_path() -> Option<PathBuf> {
    let mut dir = dirs::data_dir()?;
    dir.push("PortBay");
    dir.push("cloudflared");
    Some(dir.join("tunnel-quick.yml"))
}

fn isolated_config_path() -> Result<PathBuf> {
    let path = quick_tunnel_config_path()
        .ok_or_else(|| TunnelError::SpawnFailed("no data dir".to_string()))?;
    let dir = path
        .parent()
        .expect("config path always has a cloudflared parent dir");
    std::fs::create_dir_all(dir)
        .map_err(|e| TunnelError::SpawnFailed(format!("mkdir cloudflared dir: {e}")))?;
    // PortBay quick tunnel: intentionally minimal so the user's ~/.cloudflared/config.yml
    // is not inherited.
    std::fs::write(&path, b"# PortBay quick tunnel: intentionally minimal so the user's ~/.cloudflared/config.yml is not inherited\n")
        .map_err(|e| TunnelError::SpawnFailed(format!("write cloudflared config: {e}")))?;
    Ok(path)
}

/// Grace period before a stale cloudflared gets SIGKILL during the boot sweep.
const SWEEP_GRACE: Duration = Duration::from_millis(500);

/// Reap any leftover PortBay-spawned `cloudflared` quick tunnels from a previous
/// run. A normal quit tears tunnels down via [`TunnelManager`]'s `Drop`, but a
/// crash / `SIGKILL` runs no destructor — cloudflared reparents to launchd and
/// keeps tunneling a now-dead origin. Run this once at boot, before we spawn
/// anything: at that point any cloudflared whose command line references our
/// isolated `--config` path is, by definition, our orphan. Returns the count
/// reaped. Mirrors `process_compose::lifecycle::sweep_stale`.
pub fn sweep_stale_cloudflared() -> usize {
    let Some(marker_path) = quick_tunnel_config_path() else {
        return 0;
    };
    let marker = marker_path.to_string_lossy();
    let out = match std::process::Command::new("ps")
        .args(["-axo", "pid=,ppid=,command="])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return 0,
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let targets = stale_cloudflared_pids(&text, &marker, std::process::id());
    for pid in &targets {
        let _ = crate::port_holder::kill_gracefully(*pid, SWEEP_GRACE);
    }
    targets.len()
}

/// Parse `ps -axo pid=,ppid=,command=` output and return the pids of every
/// `cloudflared` line that references `marker` (our isolated config path) and
/// isn't `me`. Pure string work, so it's unit-testable without spawning a thing.
fn stale_cloudflared_pids(ps_output: &str, marker: &str, me: u32) -> Vec<u32> {
    ps_output
        .lines()
        .filter_map(|line| {
            let pid = line.split_whitespace().next()?.parse::<u32>().ok()?;
            if pid == me {
                return None;
            }
            (line.contains("cloudflared") && line.contains(marker)).then_some(pid)
        })
        .collect()
}

// --- Cross-process tunnel state mirror -------------------------------------
//
// Tunnels are managed in the app's in-memory `TunnelManager`, which a separate
// process (the CLI / MCP server) can't reach. So the app mirrors the live
// tunnel list to a JSON file on every change; out-of-process readers read that
// file. Read-only for them — the app is the sole writer. The file is stale by
// design when the app isn't running, so boot clears it and shutdown empties it.

/// Filename (under the PortBay data dir) the app mirrors live tunnel state to.
pub const STATE_FILE: &str = "tunnels-state.json";

/// Absolute path to the tunnel state file under `data_dir`.
pub fn state_file_path(data_dir: &std::path::Path) -> PathBuf {
    data_dir.join(STATE_FILE)
}

/// Mirror the current tunnel list to the state file (write-then-rename so a
/// reader never sees a half-written file). Best-effort; the caller logs on Err.
pub fn write_state(data_dir: &std::path::Path, tunnels: &[TunnelStatus]) -> std::io::Result<()> {
    let path = state_file_path(data_dir);
    let json = serde_json::to_vec_pretty(tunnels)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, &path)
}

/// Read the mirrored tunnel state. Empty vec when the file is missing or
/// unreadable/corrupt — an absent/just-booted app means "no tunnels", the
/// honest read for an out-of-process caller.
pub fn read_state(data_dir: &std::path::Path) -> Vec<TunnelStatus> {
    let Ok(bytes) = std::fs::read(state_file_path(data_dir)) else {
        return Vec::new();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

/// Build the cloudflared command for a quick (ephemeral) tunnel.
///
/// Traffic is routed through Caddy (`upstream_url`, normally its plain-HTTP
/// `:80` listener) rather than directly to the dev server, so Caddy's
/// Origin/Host normalisation applies for the duration of the share.
/// `--http-host-header` tells cloudflared to send a `Host` header matching the
/// project's Caddy hostname so Caddy routes the request to the correct project.
///
/// `--config` points at an isolated minimal YAML (a comment, not empty) so
/// cloudflared never inherits the user's `~/.cloudflared/config.yml`.
/// `--no-tls-verify` is a harmless no-op for the plain-HTTP `:80` origin we use
/// today; it is retained so an https origin (should we route to Caddy's TLS
/// port in future) would accept the self-signed mkcert cert without a flag flip.
fn resolve_command(
    app: &AppHandle,
    upstream_url: &str,
    hostname: &str,
) -> Result<tauri_plugin_shell::process::Command> {
    let config_path = isolated_config_path()?;
    let config_str = config_path.to_string_lossy().into_owned();

    let args = [
        "tunnel",
        "--config",
        &config_str,
        "--url",
        upstream_url,
        "--http-host-header",
        hostname,
        "--no-autoupdate",
        "--no-tls-verify",
    ];

    if let Ok(sidecar) = app.shell().sidecar("cloudflared") {
        return Ok(sidecar.args(args));
    }
    let path = which::which("cloudflared").map_err(|_| TunnelError::BinaryMissing)?;
    Ok(app
        .shell()
        .command(path.to_string_lossy().into_owned())
        .args(args))
}

/// Build the cloudflared command for a **named** tunnel run from a PortBay-owned
/// config. Unlike the quick path there is no `--url`: the config's `ingress`
/// defines routing, and its `tunnel:`/`credentials-file:` select the user's
/// named tunnel. We pass our generated config (never `~/.cloudflared/config.yml`),
/// preserving the same isolation invariant — pointed at the user's creds on
/// purpose this time.
fn resolve_custom_command(
    app: &AppHandle,
    config_path: &std::path::Path,
) -> Result<tauri_plugin_shell::process::Command> {
    let config_str = config_path.to_string_lossy().into_owned();
    let args = ["tunnel", "--config", &config_str, "--no-autoupdate", "run"];

    if let Ok(sidecar) = app.shell().sidecar("cloudflared") {
        return Ok(sidecar.args(args));
    }
    let path = which::which("cloudflared").map_err(|_| TunnelError::BinaryMissing)?;
    Ok(app
        .shell()
        .command(path.to_string_lossy().into_owned())
        .args(args))
}

/// Parse the public `trycloudflare.com` URL from a cloudflared log
/// line. The format has been stable for several years:
///
/// ```text
/// 2025-11-07T12:34:56Z INF |  https://random-name.trycloudflare.com  |
/// ```
///
/// We accept any line containing `https://<host>.trycloudflare.com`
/// without trying to match the surrounding ASCII-table formatting.
pub(crate) fn parse_public_url(line: &str) -> Option<String> {
    let needle = "https://";
    let suffix = ".trycloudflare.com";
    let start = line.find(needle)?;
    let rest = &line[start..];
    let end = rest.find(suffix)?;
    let host_end = end + suffix.len();
    // The host might be followed by punctuation/whitespace; truncate
    // at the first non-URL-safe char after the suffix.
    let url = &rest[..host_end];
    // Validate: the path is well-formed (`https://[a-z0-9-]+.trycloudflare.com`).
    let host = &url[needle.len()..url.len() - suffix.len()];
    if host.is_empty() || !host.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return None;
    }
    Some(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_url_from_cloudflared_log_line() {
        let line = "2025-11-07T12:34:56Z INF |  https://random-name.trycloudflare.com  | someone";
        assert_eq!(
            parse_public_url(line).as_deref(),
            Some("https://random-name.trycloudflare.com"),
        );
    }

    #[test]
    fn parses_url_from_table_border_lines() {
        let line = "| https://abc-def-ghi.trycloudflare.com |";
        assert_eq!(
            parse_public_url(line).as_deref(),
            Some("https://abc-def-ghi.trycloudflare.com"),
        );
    }

    #[test]
    fn rejects_lines_without_trycloudflare() {
        assert!(parse_public_url("INF starting tunnel").is_none());
        assert!(parse_public_url("https://example.com").is_none());
        assert!(parse_public_url("").is_none());
    }

    #[test]
    fn rejects_malformed_host_segment() {
        // empty host
        assert!(parse_public_url("https://.trycloudflare.com").is_none());
        // host with invalid characters
        assert!(parse_public_url("https://bad host!.trycloudflare.com").is_none());
    }

    #[test]
    fn manager_starts_empty_and_counts_zero() {
        let m = TunnelManager::new();
        assert!(m.list().is_empty());
        assert_eq!(m.count(), 0);
        assert!(!m.is_running("anything"));
        assert!(m.status("anything").is_none());
    }

    #[test]
    fn stop_all_on_empty_manager_is_zero() {
        let mut m = TunnelManager::new();
        assert_eq!(m.stop_all(), 0);
        assert_eq!(m.count(), 0);
    }

    #[test]
    fn stale_sweep_matches_only_our_cloudflared() {
        // ppid 1 = orphaned to launchd after a crash; that's the real-world
        // leftover the boot sweep targets. The user's own cloudflared (a
        // *different* --config) and unrelated processes are left untouched.
        let marker = "/Users/n/Library/Application Support/PortBay/cloudflared/tunnel-quick.yml";
        let ps = format!(
            "  4242     1 /…/target/debug/cloudflared tunnel --config {marker} --url http://127.0.0.1:80 --http-host-header app.portbay.test\n\
             54321     1 /opt/homebrew/bin/cloudflared tunnel --config /Users/n/.cloudflared/config.yml run my-named-tunnel\n\
               321     1 /usr/sbin/cfprefsd agent"
        );
        let pids = stale_cloudflared_pids(&ps, marker, 999_999);
        assert_eq!(pids, vec![4242], "only PortBay's quick tunnel matches");
    }

    #[test]
    fn stale_sweep_never_targets_self() {
        let marker = "/data/PortBay/cloudflared/tunnel-quick.yml";
        let ps =
            format!("  777     1 cloudflared tunnel --config {marker} --url http://127.0.0.1:80");
        // When the matching pid is us, it must be excluded (we'd never sweep the
        // live process driving the sweep).
        assert!(stale_cloudflared_pids(&ps, marker, 777).is_empty());
    }
}
