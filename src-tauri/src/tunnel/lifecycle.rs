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

use serde::Serialize;
use tauri::AppHandle;
use tauri_plugin_shell::process::CommandChild;
use tauri_plugin_shell::ShellExt;

use crate::tunnel::error::{Result, TunnelError};

/// How long the start path waits for cloudflared to announce a public
/// URL on stdout before giving up. Real-world: tunnels usually appear
/// within 2–6 s; 20 s leaves headroom for slow connections.
pub const TUNNEL_URL_TIMEOUT: Duration = Duration::from_secs(20);

/// Public view of one running tunnel — what the GUI / list command
/// renders. Cheap to clone.
#[derive(Debug, Clone, Serialize)]
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

    /// Spawn cloudflared for `project_id`, routing traffic through Caddy's
    /// HTTPS listener so Origin/Host normalisation applies.
    ///
    /// `hostname` is the project's Caddy hostname (e.g. `myapp.test`); it is
    /// passed as `--http-host-header` so Caddy matches the correct route.
    /// `caddy_https_port` is the port Caddy is listening on (stored in
    /// `AppState::caddy_https_port` at boot time).
    ///
    /// The returned status reflects the just-started state — `public_url`
    /// is initially `None`. Callers poll `status` until the URL is
    /// populated (the stdout-tail task fills it in).
    pub fn start(
        &mut self,
        app: &AppHandle,
        project_id: &str,
        hostname: &str,
        caddy_https_port: u16,
    ) -> Result<TunnelStatus> {
        if self.tunnels.contains_key(project_id) {
            return Err(TunnelError::AlreadyRunning(project_id.to_string()));
        }

        let upstream_url = format!("https://127.0.0.1:{caddy_https_port}");
        let cmd = resolve_command(app, &upstream_url, hostname)?;
        let (mut rx, child) = cmd
            .spawn()
            .map_err(|e| TunnelError::SpawnFailed(e.to_string()))?;

        let public_url = Arc::new(Mutex::new(None::<String>));
        let public_url_for_task = public_url.clone();

        // Tail the child's output and fill in the public URL once
        // cloudflared announces it. Closes naturally when the child
        // exits and the receiver is drained.
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
            upstream_url,
            public_url,
            started_at_ms,
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
fn isolated_config_path() -> Result<PathBuf> {
    let mut dir = dirs::data_dir()
        .ok_or_else(|| TunnelError::SpawnFailed("no data dir".to_string()))?;
    dir.push("PortBay");
    dir.push("cloudflared");
    std::fs::create_dir_all(&dir)
        .map_err(|e| TunnelError::SpawnFailed(format!("mkdir cloudflared dir: {e}")))?;
    let path = dir.join("tunnel-quick.yml");
    // PortBay quick tunnel: intentionally minimal so the user's ~/.cloudflared/config.yml
    // is not inherited.
    std::fs::write(&path, b"# PortBay quick tunnel: intentionally minimal so the user's ~/.cloudflared/config.yml is not inherited\n")
        .map_err(|e| TunnelError::SpawnFailed(format!("write cloudflared config: {e}")))?;
    Ok(path)
}

/// Build the cloudflared command for a quick (ephemeral) tunnel.
///
/// Traffic is routed through Caddy's local HTTPS listener
/// (`https://127.0.0.1:<caddy_https_port>`) rather than directly to the dev
/// server, so Caddy's Origin/Host normalisation applies for the duration of
/// the share. `--http-host-header` tells cloudflared to send a `Host` header
/// matching the project's Caddy hostname so Caddy routes the request to the
/// correct project.
///
/// `--config` points at an isolated minimal YAML (a comment, not empty) so
/// cloudflared never inherits the user's `~/.cloudflared/config.yml`.
/// `--no-tls-verify` is required because Caddy is using a self-signed mkcert
/// cert on localhost.
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
}
