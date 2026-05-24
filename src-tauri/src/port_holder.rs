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
//! safe for leaked PortBay orphans — see `is_reclaimable_orphan`).

use std::path::PathBuf;

/// How many process-tree ancestors we'll walk up when searching for
/// a PortBay-managed orphan. Three levels covers the typical chain:
/// worker (`next-server`) → dev-server shell (`node /path/to/next dev`)
/// → wrapper (`pnpm dev`). Bounded so a runaway parent chain can't
/// lock us up.
const MAX_ANCESTORS: usize = 4;

#[derive(Debug, Clone)]
pub struct PortHolder {
    pub pid: u32,
    /// e.g. "node", "php-fpm". `lsof`'s COMMAND field (first column).
    pub command: String,
    /// Full executable path when resolvable; otherwise just the command.
    pub binary: Option<PathBuf>,
    /// Full `/proc`-style command line if we can find it; falls back
    /// to just the command name. Used to decide whether a holder is
    /// a PortBay-managed orphan or an external local-dev tool.
    pub command_line: Option<String>,
    /// The holder's current working directory, when resolvable. This is the
    /// most reliable attribution signal: a dev server inherits its project
    /// directory as cwd, and — unlike argv and the parent chain — cwd survives
    /// the process being orphaned to launchd. Worker processes like
    /// `next-server` hide the project path in argv and lose their ancestors
    /// when orphaned; their cwd still points inside the project.
    pub cwd: Option<PathBuf>,
    /// Walk from the immediate parent up to `MAX_ANCESTORS` levels.
    /// Worker processes (e.g. Next.js's `next-server`) hide the
    /// project path; the shell that spawned them
    /// (`node /Volumes/…/project/.bin/next dev`) carries it. We need
    /// the chain to attribute orphans correctly and to know which
    /// PID to SIGTERM (always the topmost matching ancestor so the
    /// wrapper propagates the signal to its worker).
    pub ancestors: Vec<Ancestor>,
    /// True when the holder has no live parent (reparented to launchd /
    /// PID 1, or the parent is otherwise unresolvable). An orphaned holder
    /// that ties to the project is a leaked PortBay dev server we can safely
    /// reclaim; a holder with a live parent is either our own running
    /// supervisor tree or a process the user is managing themselves, and we
    /// never kill those.
    pub orphaned: bool,
}

#[derive(Debug, Clone)]
pub struct Ancestor {
    pub pid: u32,
    pub command_line: String,
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

    /// Does this holder belong to the given project? Matches three signals,
    /// any of which is sufficient:
    ///   - its **cwd** is the project dir or a descendant (the strongest
    ///     signal — survives orphaning, where argv/ancestors don't),
    ///   - its own command line references the project path / leaf folder,
    ///   - any ancestor's command line does (workers hide the path; the shell
    ///     that spawned them carries it).
    pub fn ties_to_project(&self, working_dir: &str) -> bool {
        let dir_token = std::path::Path::new(working_dir)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        if dir_token.is_empty() {
            return false;
        }
        if let Some(cwd) = &self.cwd {
            if cwd.starts_with(working_dir) {
                return true;
            }
        }
        let matches_cmd = |cmd: &str| cmd.contains(working_dir) || cmd.contains(dir_token);
        if let Some(cmd) = &self.command_line {
            if matches_cmd(cmd) {
                return true;
            }
        }
        self.ancestors.iter().any(|a| matches_cmd(&a.command_line))
    }

    /// A leaked PortBay dev server we can safely reclaim on the user's behalf:
    /// it has no live parent (orphaned to launchd) AND it ties to the project
    /// whose Play button was just pressed. The orphan gate is what keeps us
    /// from ever killing a live process the user is managing themselves.
    pub fn is_reclaimable_orphan(&self, working_dir: &str) -> bool {
        self.orphaned && self.ties_to_project(working_dir)
    }

    /// True when `pid` appears anywhere in the holder's ancestor chain — i.e.
    /// the holder descends from that process. Used with PortBay's own
    /// process-compose pid to recognise our own running dev server (so the
    /// pre-flight doesn't mistake it for an external conflict).
    pub fn descends_from(&self, pid: u32) -> bool {
        self.ancestors.iter().any(|a| a.pid == pid)
    }

    /// PID to SIGTERM when we've decided to kill an orphan. Returns
    /// the topmost ancestor that matches the project's working_dir —
    /// kill the wrapper, the worker dies with it. Falls back to the
    /// holder's own PID when no ancestor matches (defensive; we only
    /// reach this code when `is_reclaimable_orphan` returned true).
    pub fn kill_target(&self, working_dir: &str) -> u32 {
        let dir_token = std::path::Path::new(working_dir)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        let matches_cmd = |cmd: &str| {
            !dir_token.is_empty() && (cmd.contains(working_dir) || cmd.contains(dir_token))
        };
        // Ancestors are ordered closest-to-holder first; the last one
        // is the topmost. Walk in reverse so a matching parent wins
        // over a matching worker.
        for a in self.ancestors.iter().rev() {
            if matches_cmd(&a.command_line) {
                return a.pid;
            }
        }
        if let Some(cmd) = &self.command_line {
            if matches_cmd(cmd) {
                return self.pid;
            }
        }
        self.pid
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
        // A parent of 0/1 (or one we can't read) means the process was
        // reparented to launchd — i.e. orphaned.
        let orphaned = !matches!(resolve_parent_pid(pid), Some(p) if p > 1);
        return Some(PortHolder {
            pid,
            command: command.clone(),
            binary: resolve_binary(pid),
            command_line: resolve_command_line(pid),
            cwd: resolve_cwd(pid),
            ancestors: walk_ancestors(pid),
            orphaned,
        });
    }
    None
}

/// Resolve a process's current working directory via
/// `lsof -a -p <pid> -d cwd -Fn`, which prints the cwd on a line prefixed
/// with `n`. `None` when lsof can't read it (permissions, process gone).
fn resolve_cwd(pid: u32) -> Option<PathBuf> {
    let out = std::process::Command::new("lsof")
        .args(["-a", "-d", "cwd", "-Fn", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        if let Some(path) = line.strip_prefix('n') {
            let path = path.trim();
            if !path.is_empty() {
                return Some(PathBuf::from(path));
            }
        }
    }
    None
}

/// Walk from the holder's parent up to `MAX_ANCESTORS` levels.
/// Returns the chain ordered closest-first (parent at [0], grandparent
/// at [1], ...). Empty when the holder has no parent we can resolve
/// (orphaned to PID 1, etc.).
fn walk_ancestors(pid: u32) -> Vec<Ancestor> {
    let mut out = Vec::new();
    let mut current = pid;
    for _ in 0..MAX_ANCESTORS {
        let Some(parent) = resolve_parent_pid(current) else {
            break;
        };
        if parent == 0 || parent == 1 {
            // PID 1 (init/launchd) means the process is orphaned —
            // there's nothing useful above it for attribution.
            break;
        }
        let Some(cmd) = resolve_command_line(parent) else {
            break;
        };
        out.push(Ancestor {
            pid: parent,
            command_line: cmd,
        });
        current = parent;
    }
    out
}

fn resolve_parent_pid(pid: u32) -> Option<u32> {
    let out = std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "ppid="])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    s.parse::<u32>().ok()
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
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
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

    fn holder(command_line: &str, ancestors: Vec<Ancestor>) -> PortHolder {
        PortHolder {
            pid: 999,
            command: "node".into(),
            binary: None,
            command_line: Some(command_line.into()),
            cwd: None,
            ancestors,
            orphaned: false,
        }
    }

    #[test]
    fn ties_to_project_matches_on_project_dir_token() {
        let h = holder(
            "node /Volumes/DevSSD/projects/Clients/test-project/node_modules/.bin/next dev",
            vec![],
        );
        assert!(h.ties_to_project("/Volumes/DevSSD/projects/Clients/test-project"));
    }

    #[test]
    fn ties_to_project_rejects_unrelated_processes() {
        let mut h = holder("/usr/local/sbin/nginx -c /opt/nginx.conf", vec![]);
        h.command = "nginx".into();
        assert!(!h.ties_to_project("/Volumes/DevSSD/projects/Clients/test-project"));
    }

    #[test]
    fn ties_to_project_matches_via_parent_when_worker_hides_path() {
        // next-server (the worker that binds the port) reports its name
        // without any path; we attribute it via the parent's command line.
        let h = holder(
            "next-server (v16.2.6)",
            vec![
                Ancestor {
                    pid: 998,
                    command_line:
                        "node /Volumes/DevSSD/projects/Clients/test-project/node_modules/.bin/next dev --port 3010"
                            .into(),
                },
                Ancestor { pid: 997, command_line: "pnpm dev".into() },
            ],
        );
        assert!(h.ties_to_project("/Volumes/DevSSD/projects/Clients/test-project"));
    }

    #[test]
    fn ties_to_project_matches_via_cwd_when_orphaned() {
        // The real BookSlash case: an orphaned next-server with no path in
        // argv and no ancestors (reparented to launchd). Its cwd — a
        // descendant of the project root — is the only signal, and it's enough.
        let mut h = holder("next-server (v15.5.15)", vec![]);
        h.cwd = Some(PathBuf::from(
            "/Volumes/DevSSD/projects/Clients/BookSlash/apps/web",
        ));
        h.orphaned = true;
        assert!(h.ties_to_project("/Volumes/DevSSD/projects/Clients/BookSlash"));
        assert!(h.is_reclaimable_orphan("/Volumes/DevSSD/projects/Clients/BookSlash"));
    }

    #[test]
    fn live_holder_in_project_tree_is_not_reclaimable() {
        // Same project tie via cwd, but a LIVE parent (the user running it in
        // a terminal). We must NOT reclaim/kill the user's own process — only
        // warn. Orphan gate enforces that.
        let mut h = holder(
            "node /Volumes/DevSSD/projects/Clients/test-project/node_modules/.bin/vite",
            vec![Ancestor {
                pid: 998,
                command_line: "npm run dev:client".into(),
            }],
        );
        h.cwd = Some(PathBuf::from(
            "/Volumes/DevSSD/projects/Clients/test-project",
        ));
        h.orphaned = false;
        assert!(h.ties_to_project("/Volumes/DevSSD/projects/Clients/test-project"));
        assert!(!h.is_reclaimable_orphan("/Volumes/DevSSD/projects/Clients/test-project"));
    }

    #[test]
    fn descends_from_detects_supervisor_ancestor() {
        let h = holder(
            "next-server (v16.2.6)",
            vec![
                Ancestor {
                    pid: 998,
                    command_line: "node .../next dev".into(),
                },
                Ancestor {
                    pid: 555,
                    command_line: "process-compose -f .../process-compose.yaml up".into(),
                },
            ],
        );
        assert!(h.descends_from(555));
        assert!(!h.descends_from(444));
    }

    #[test]
    fn kill_target_picks_topmost_matching_ancestor() {
        // SIGTERM the wrapper (`pnpm …` here is the most distant matching
        // ancestor), not the worker, so the whole tree dies via propagation.
        let h = holder(
            "next-server (v16.2.6)",
            vec![
                Ancestor {
                    pid: 998,
                    command_line:
                        "node /Volumes/DevSSD/projects/Clients/test-project/.bin/next dev".into(),
                },
                Ancestor {
                    pid: 997,
                    command_line: "pnpm /Volumes/DevSSD/projects/Clients/test-project run dev"
                        .into(),
                },
            ],
        );
        assert_eq!(
            h.kill_target("/Volumes/DevSSD/projects/Clients/test-project"),
            997
        );
    }
}
