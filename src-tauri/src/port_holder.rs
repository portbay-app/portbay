//! Port-holder discovery — identifies which process (if any) is
//! binding a TCP port on localhost, so PortBay can offer a useful
//! error message instead of letting Process Compose flail.
//!
//! Strategy:
//!   - Use `lsof -nP -iTCP:<port> -sTCP:LISTEN` because it gives us
//!     the PID + binary path + command line in a single call,
//!     works on macOS and Linux, and doesn't need elevated
//!     privileges for the user's own processes.
//!   - For ports held by processes we don't own (root-owned, sandboxed),
//!     lsof returns nothing. We treat that as "not held" — best-effort.
//!
//! No process is mutated here. Callers decide whether to kill (only
//! safe for PortBay-managed orphans — see `is_likely_portbay_managed`).

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct PortHolder {
    pub pid: u32,
    /// e.g. "node", "php-fpm". `lsof`'s COMMAND field (first column).
    pub command: String,
    /// Full executable path when resolvable; otherwise just the command.
    pub binary: Option<PathBuf>,
    /// Full `/proc`-style command line if we can find it; falls back
    /// to just the command name. Used to decide whether a holder is
    /// a PortBay-managed orphan or an external (ServBay nginx, MAMP).
    pub command_line: Option<String>,
}

impl PortHolder {
    /// Best-effort label for the user — falls back through the most
    /// specific fields first.
    pub fn display(&self) -> String {
        if let Some(cmd) = &self.command_line {
            // Truncate so the toast doesn't blow up — full path lives
            // in the structured error envelope.
            let truncated: String = cmd.chars().take(80).collect();
            return format!("{} (pid {})", truncated.trim(), self.pid);
        }
        format!("{} (pid {})", self.command, self.pid)
    }

    /// Heuristic: does this look like a process PortBay itself
    /// spawned and lost track of? Two signals:
    ///   1. The command line includes the project's hostname or
    ///      working_dir prefix (a PortBay-aware dev server).
    ///   2. The parent process is process-compose (the PID we
    ///      already track in AppState).
    /// Caller passes the hints it has; we never kill a process we
    /// can't strongly identify.
    pub fn looks_like_portbay_orphan(&self, working_dir: &str) -> bool {
        let dir_token = std::path::Path::new(working_dir)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        if dir_token.is_empty() {
            return false;
        }
        let cmd = self.command_line.as_deref().unwrap_or("");
        cmd.contains(working_dir) || cmd.contains(dir_token)
    }
}

/// Identify the process listening on `port` on localhost. Returns
/// `None` when nothing is listening or `lsof` is unavailable.
pub fn find(port: u16) -> Option<PortHolder> {
    let out = std::process::Command::new("lsof")
        .args(["-nP", "-sTCP:LISTEN"])
        .arg(format!("-iTCP:{port}"))
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    parse_lsof_first(&text)
}

/// Parse lsof's tabular output. Format:
/// ```text
/// COMMAND   PID USER FD TYPE DEVICE SIZE/OFF NODE NAME
/// node    12345 user 22u IPv4 0xabc      0t0  TCP *:3010 (LISTEN)
/// ```
/// We return the first LISTEN row; multi-bind ports give the same
/// useful PID either way.
fn parse_lsof_first(text: &str) -> Option<PortHolder> {
    for line in text.lines().skip(1) {
        let trimmed = line.trim();
        if trimmed.is_empty() || !trimmed.contains("LISTEN") {
            continue;
        }
        let mut cols = trimmed.split_whitespace();
        let command = cols.next()?.to_string();
        let pid_str = cols.next()?;
        let pid: u32 = pid_str.parse().ok()?;
        return Some(PortHolder {
            pid,
            command: command.clone(),
            binary: resolve_binary(pid),
            command_line: resolve_command_line(pid),
        });
    }
    None
}

/// macOS-friendly: ask `ps -o command=` for the full argv string.
/// Linux's `/proc/<pid>/cmdline` would be a more direct read but the
/// app targets macOS first; ps works on both.
fn resolve_command_line(pid: u32) -> Option<String> {
    let out = std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "command="])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

/// Best-effort binary path via `ps -o comm=`.
fn resolve_binary(pid: u32) -> Option<PathBuf> {
    let out = std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(PathBuf::from(s))
    }
}

/// Send SIGTERM to a process, then SIGKILL after `grace`. Returns Ok
/// when the process is gone (or wasn't there to begin with).
pub fn kill_gracefully(pid: u32, grace: std::time::Duration) -> std::io::Result<()> {
    use std::time::Instant;
    let _ = std::process::Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .output();
    let deadline = Instant::now() + grace;
    while Instant::now() < deadline {
        if !pid_alive(pid) {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let _ = std::process::Command::new("kill")
        .args(["-KILL", &pid.to_string()])
        .output();
    // One more brief wait so the caller's port-re-check sees the slot
    // free even on slow systems.
    std::thread::sleep(std::time::Duration::from_millis(150));
    Ok(())
}

fn pid_alive(pid: u32) -> bool {
    // `kill -0` only checks signal-delivery feasibility, doesn't send.
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_lsof_returns_none_for_empty_output() {
        assert!(parse_lsof_first("").is_none());
        assert!(parse_lsof_first("COMMAND   PID USER\n").is_none());
    }

    #[test]
    fn parse_lsof_pulls_first_listen_row() {
        let sample = "\
COMMAND   PID USER   FD   TYPE  DEVICE SIZE/OFF NODE NAME
node    99887 nour   22u  IPv4 0xabc       0t0  TCP *:3010 (LISTEN)
";
        let h = parse_lsof_first(sample).expect("should parse");
        assert_eq!(h.pid, 99887);
        assert_eq!(h.command, "node");
    }

    #[test]
    fn parse_lsof_skips_established_connections() {
        let sample = "\
COMMAND   PID USER   FD   TYPE  DEVICE SIZE/OFF NODE NAME
chrome  10001 nour   22u  IPv4 0xabc       0t0  TCP 127.0.0.1:55555->127.0.0.1:3010 (ESTABLISHED)
node    99887 nour   22u  IPv4 0xdef       0t0  TCP *:3010 (LISTEN)
";
        let h = parse_lsof_first(sample).expect("should parse");
        assert_eq!(h.pid, 99887);
    }

    #[test]
    fn looks_like_orphan_matches_on_project_dir_token() {
        let h = PortHolder {
            pid: 123,
            command: "node".into(),
            binary: None,
            command_line: Some(
                "node /Volumes/DevSSD/projects/Clients/nour-beiruti/node_modules/.bin/next dev".into(),
            ),
        };
        assert!(h.looks_like_portbay_orphan(
            "/Volumes/DevSSD/projects/Clients/nour-beiruti",
        ));
    }

    #[test]
    fn looks_like_orphan_rejects_unrelated_processes() {
        let h = PortHolder {
            pid: 123,
            command: "nginx".into(),
            binary: None,
            command_line: Some("/Applications/ServBay/bin/nginx -c …".into()),
        };
        assert!(!h.looks_like_portbay_orphan(
            "/Volumes/DevSSD/projects/Clients/nour-beiruti",
        ));
    }
}
