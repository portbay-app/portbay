//! System-level commands — `doctor`, `tail_logs`.
//!
//! `doctor` mirrors the CLI's `cmd_doctor` JSON output shape so the GUI
//! and CLI report the same findings to the same support requests.

use tauri::{AppHandle, Emitter, State};

use crate::commands::dto::{DoctorFinding, DoctorReport, DoctorVerdict};
use crate::commands::projects::load_registry;
use crate::error::{AppError, AppResult};
use crate::hosts::HostsManager;
use crate::state::AppState;

#[tauri::command]
pub async fn doctor(state: State<'_, AppState>) -> AppResult<DoctorReport> {
    let mut findings = Vec::new();

    // Registry
    match load_registry(&state) {
        Ok(reg) => findings.push(DoctorFinding {
            check: "registry".into(),
            verdict: DoctorVerdict::Ok,
            detail: format!(
                "{} project(s), v{} schema, suffix .{}",
                reg.list_projects().len(),
                reg.version,
                reg.domain_suffix
            ),
        }),
        Err(e) => findings.push(DoctorFinding {
            check: "registry".into(),
            verdict: DoctorVerdict::Fail,
            detail: e.to_string(),
        }),
    }

    // PC daemon
    let pc_client = state
        .pc_client
        .lock()
        .expect("pc_client mutex poisoned")
        .clone();
    let pc_finding = match pc_client {
        None => DoctorFinding {
            check: "process-compose".into(),
            verdict: DoctorVerdict::Warn,
            detail: "not started yet".into(),
        },
        Some(c) => match c.live().await {
            Ok(true) => DoctorFinding {
                check: "process-compose".into(),
                verdict: DoctorVerdict::Ok,
                detail: "alive".into(),
            },
            Ok(false) => DoctorFinding {
                check: "process-compose".into(),
                verdict: DoctorVerdict::Warn,
                detail: "not reachable".into(),
            },
            Err(e) => DoctorFinding {
                check: "process-compose".into(),
                verdict: DoctorVerdict::Warn,
                detail: e.to_string(),
            },
        },
    };
    findings.push(pc_finding);

    // Caddy daemon
    let caddy_client = state
        .caddy_client
        .lock()
        .expect("caddy_client mutex poisoned")
        .clone();
    let caddy_finding = match caddy_client {
        None => DoctorFinding {
            check: "caddy".into(),
            verdict: DoctorVerdict::Warn,
            detail: "not started yet".into(),
        },
        Some(c) => match c.is_alive().await {
            Ok(true) => DoctorFinding {
                check: "caddy".into(),
                verdict: DoctorVerdict::Ok,
                detail: "alive".into(),
            },
            Ok(false) => DoctorFinding {
                check: "caddy".into(),
                verdict: DoctorVerdict::Warn,
                detail: "not reachable".into(),
            },
            Err(e) => DoctorFinding {
                check: "caddy".into(),
                verdict: DoctorVerdict::Warn,
                detail: e.to_string(),
            },
        },
    };
    findings.push(caddy_finding);

    // Tools on PATH
    for tool in ["mkcert", "caddy", "process-compose"] {
        match which::which(tool) {
            Ok(p) => findings.push(DoctorFinding {
                check: format!("tool: {tool}"),
                verdict: DoctorVerdict::Ok,
                detail: p.display().to_string(),
            }),
            Err(_) => findings.push(DoctorFinding {
                check: format!("tool: {tool}"),
                verdict: DoctorVerdict::Warn,
                detail: "not found on PATH (bundled .app uses its sidecar — this only matters for CLI standalone use)".into(),
            }),
        }
    }

    // /etc/hosts reconcile state
    match (HostsManager::system().list_managed(), load_registry(&state)) {
        (Ok(entries), Ok(reg)) => {
            use std::collections::HashSet;
            let expected: HashSet<String> = reg
                .list_projects()
                .iter()
                .map(|p| p.hostname.clone())
                .collect();
            let present: HashSet<String> = entries.iter().map(|e| e.hostname.clone()).collect();
            let missing = expected.difference(&present).count();
            let orphan = present.difference(&expected).count();
            let verdict = if missing == 0 && orphan == 0 {
                DoctorVerdict::Ok
            } else {
                DoctorVerdict::Warn
            };
            let detail = if missing == 0 && orphan == 0 {
                format!("{} entries, all match registry", entries.len())
            } else {
                format!(
                    "{} entries (missing: {missing}, orphan: {orphan}). Run `sudo portbay hosts reconcile` to fix.",
                    entries.len()
                )
            };
            findings.push(DoctorFinding {
                check: "/etc/hosts".into(),
                verdict,
                detail,
            });
        }
        (Err(e), _) => findings.push(DoctorFinding {
            check: "/etc/hosts".into(),
            verdict: DoctorVerdict::Warn,
            detail: e.to_string(),
        }),
        (_, Err(_)) => {
            // Registry load already errored above; nothing useful to add here.
        }
    }

    Ok(DoctorReport { findings })
}

/// `read_dotenv(path)` — read a user-picked `.env`-style file and
/// return its `KEY=value` pairs as a vector preserving file order.
/// Comments (`#`) and blank lines are skipped; surrounding quotes
/// on the value are stripped when matched on both ends.
///
/// We do the parse on the Rust side so the wire shape is already
/// clean — the frontend just merges the result into its row state.
/// Files larger than 256 KB are rejected to avoid hostile inputs.
#[tauri::command]
pub async fn read_dotenv(path: String) -> AppResult<Vec<(String, String)>> {
    use std::fs;

    const MAX_BYTES: u64 = 256 * 1024;
    let meta =
        fs::metadata(&path).map_err(|e| AppError::BadInput(format!("can't open {path}: {e}")))?;
    if !meta.is_file() {
        return Err(AppError::BadInput(format!("not a regular file: {path}")));
    }
    if meta.len() > MAX_BYTES {
        return Err(AppError::BadInput(format!(
            ".env file is too large ({} bytes); paste it instead",
            meta.len()
        )));
    }
    let text = fs::read_to_string(&path)
        .map_err(|e| AppError::BadInput(format!("can't read {path}: {e}")))?;
    Ok(parse_dotenv(&text))
}

/// Parser for [`read_dotenv`]. Exposed for unit tests.
pub(crate) fn parse_dotenv(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Strip an optional `export ` prefix to be friendly to shell-
        // sourced env files.
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some(eq) = line.find('=') else {
            continue;
        };
        let key = line[..eq].trim();
        if key.is_empty() {
            continue;
        }
        let mut value = line[eq + 1..].trim().to_string();
        if (value.starts_with('"') && value.ends_with('"') && value.len() >= 2)
            || (value.starts_with('\'') && value.ends_with('\'') && value.len() >= 2)
        {
            value = value[1..value.len() - 1].to_string();
        }
        out.push((key.to_string(), value));
    }
    out
}

/// `quit_app` — explicit "Quit PortBay" from the user menu.
///
/// Mirrors the tray's quit path (`app.exit(0)`) so window-close-to-tray
/// stays separate from a true exit. The Rust window-close handler is
/// responsible for the menu-bar-hint toast, not this command — calling
/// `exit(0)` bypasses that hint, which is the right behaviour for an
/// explicit quit from the user menu.
#[tauri::command]
pub async fn quit_app(app: AppHandle) -> AppResult<()> {
    app.exit(0);
    Ok(())
}

/// `open_main_window` — reveal PortBay's primary window from secondary UI
/// surfaces such as the tray panel. When `path` is supplied (e.g. the tray
/// popover's nav grid), route the main window there via the same
/// `portbay://nav` channel the tray menu uses (handled in +layout.svelte).
#[tauri::command]
pub async fn open_main_window(app: AppHandle, path: Option<String>) -> AppResult<()> {
    crate::tray::show_main_window(&app);
    if let Some(route) = path {
        let _ = app.emit("portbay://nav", route);
    }
    Ok(())
}

/// `tail_logs(id, limit, offset)` — snapshot of a project's recent log output.
///
/// Reads the tail of the per-project log file PC writes at
/// `<logs_dir>/<id>.log` — the same canonical file `subscribe_logs` streams.
/// We deliberately do *not* hit PC's REST `/process/logs` endpoint: it returns
/// HTTP 400 (`process <id> doesn't exist`) for any process not currently loaded
/// in the daemon — i.e. every stopped project — and even for running ones only
/// returns whatever is still in PC's in-memory ring, which is frequently empty.
/// The on-disk file is the durable record, so reading it shows history for
/// stopped projects and never errors.
///
/// For live streaming, see `subscribe_logs` (Channel<T> follow mode).
///
/// Deliberately a **synchronous** command. Tauri runs sync commands on the
/// blocking thread pool, whereas `async` commands share the async-worker pool —
/// and that worker pool gets congested by the reconciler's synchronous work
/// (mkcert, `ps` sweeps, the PC stop grace-sleep) running on it. As an `async`
/// command this snapshot would queue behind that congestion and the log viewer
/// would sit blank for many seconds before "old logs" appeared, while the HTTP
/// inspector — whose `recent_requests` backfill is sync — stayed instant. Sync
/// puts the file read on its own pool so the snapshot returns immediately. The
/// read itself is bounded and cheap, so it never starves that pool.
#[tauri::command]
pub fn tail_logs(
    state: State<'_, AppState>,
    id: String,
    #[allow(non_snake_case)] limit: Option<u32>,
    // Accepted for API compatibility. File-tail always returns the most recent
    // lines, so an offset into PC's in-memory buffer no longer applies.
    #[allow(non_snake_case)] offset: Option<u64>,
) -> AppResult<Vec<String>> {
    let _ = offset;
    let limit = limit.unwrap_or(1000).max(1) as usize;
    let path = state.logs_dir.join(format!("{id}.log"));
    Ok(tail_file_lines(&path, limit))
}

/// Read at most this many bytes from the end of a log file for the snapshot.
/// Per-project logs don't rotate, so without this a long-lived project's log
/// would grow unbounded and a full-file scan would re-introduce the open lag.
/// Mirrors the request inspector's `TAIL_READ_BYTES` (which is why it's instant).
const TAIL_READ_BYTES: u64 = 512 * 1024;

/// Return the last `limit` lines of a log file, oldest-first.
///
/// Robust to the realities of live log files:
/// - missing file (project never started) → empty vec, not an error;
/// - invalid UTF-8 (stray bytes mid-stream) → decoded lossily so a single
///   bad byte never blanks the viewer;
/// - bounded read: for a large file we `seek` to the last [`TAIL_READ_BYTES`]
///   and drop the first (partial) line, so the snapshot stays O(tail) — instant
///   even at hundreds of MB — instead of scanning the whole file;
/// - bounded memory: a ring buffer keeps at most `limit` lines.
fn tail_file_lines(path: &std::path::Path, limit: usize) -> Vec<String> {
    use std::collections::VecDeque;
    use std::io::{BufRead, BufReader, Seek, SeekFrom};

    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    // For a large file, jump to the tail window; the first line we then read is
    // almost certainly a fragment, so skip it once we're past the start.
    let mut skip_partial = false;
    if let Ok(meta) = file.metadata() {
        if meta.len() > TAIL_READ_BYTES {
            let start = meta.len() - TAIL_READ_BYTES;
            if file.seek(SeekFrom::Start(start)).is_ok() {
                skip_partial = true;
            }
        }
    }
    let mut reader = BufReader::new(file);
    if skip_partial {
        let mut discard: Vec<u8> = Vec::new();
        let _ = reader.read_until(b'\n', &mut discard);
    }
    let mut ring: VecDeque<String> = VecDeque::with_capacity(limit.min(4096) + 1);
    let mut buf: Vec<u8> = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) => break,
            Ok(_) => {
                while matches!(buf.last(), Some(b'\n' | b'\r')) {
                    buf.pop();
                }
                if ring.len() == limit {
                    ring.pop_front();
                }
                ring.push_back(String::from_utf8_lossy(&buf).into_owned());
            }
            Err(_) => break,
        }
    }
    ring.into()
}

#[cfg(test)]
mod tests {
    use super::{parse_dotenv, tail_file_lines, TAIL_READ_BYTES};

    #[test]
    fn tail_returns_last_lines_small_file() {
        let dir = std::env::temp_dir().join(format!("portbay_tail_small_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("s.log");
        std::fs::write(&path, "a\nb\nc\nd\ne\n").unwrap();
        assert_eq!(tail_file_lines(&path, 3), vec!["c", "d", "e"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tail_seeks_past_huge_file_and_keeps_correct_tail() {
        // Write well over TAIL_READ_BYTES of numbered lines, then confirm the
        // bounded (seek-to-end) read still returns the true last lines intact —
        // i.e. the partial-first-line skip didn't corrupt or drop the real tail.
        let dir = std::env::temp_dir().join(format!("portbay_tail_big_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("big.log");

        let mut content = String::new();
        let mut n = 0u32;
        while (content.len() as u64) < TAIL_READ_BYTES + 256 * 1024 {
            content.push_str(&format!("line-{n}\n"));
            n += 1;
        }
        std::fs::write(&path, &content).unwrap();

        let last = tail_file_lines(&path, 4);
        assert_eq!(last.len(), 4);
        assert_eq!(last[3], format!("line-{}", n - 1));
        assert_eq!(last[0], format!("line-{}", n - 4));
        // Every returned line is a clean, whole record (no fragment leaked in).
        assert!(last.iter().all(|l| l.starts_with("line-")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_keys_strips_comments_and_blanks() {
        let body = "\
# top comment
DATABASE_URL=postgres://localhost/foo

API_KEY=abc123
";
        let kv = parse_dotenv(body);
        assert_eq!(kv.len(), 2);
        assert_eq!(
            kv[0],
            ("DATABASE_URL".into(), "postgres://localhost/foo".into())
        );
        assert_eq!(kv[1], ("API_KEY".into(), "abc123".into()));
    }

    #[test]
    fn unwraps_matched_quotes_only() {
        let kv = parse_dotenv("A=\"with spaces\"\nB='single'\nC=\"mismatch'");
        assert_eq!(kv[0].1, "with spaces");
        assert_eq!(kv[1].1, "single");
        assert_eq!(kv[2].1, "\"mismatch'");
    }

    #[test]
    fn strips_export_prefix() {
        let kv = parse_dotenv("export FOO=bar\n");
        assert_eq!(kv[0], ("FOO".into(), "bar".into()));
    }

    #[test]
    fn ignores_lines_without_equals_or_empty_keys() {
        let kv = parse_dotenv("notakv\n=missingkey\nGOOD=ok\n");
        assert_eq!(kv.len(), 1);
        assert_eq!(kv[0].0, "GOOD");
    }
}
