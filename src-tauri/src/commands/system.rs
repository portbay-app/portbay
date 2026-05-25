//! System-level commands — `doctor`, `tail_logs`.
//!
//! `doctor` mirrors the CLI's `cmd_doctor` JSON output shape so the GUI
//! and CLI report the same findings to the same support requests.

use tauri::{AppHandle, State};

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
/// surfaces such as the tray panel.
#[tauri::command]
pub async fn open_main_window(app: AppHandle) -> AppResult<()> {
    crate::tray::show_main_window(&app);
    Ok(())
}

/// `tail_logs(id, limit, offset)` — static log tail from PC's buffer.
///
/// For live streaming, see card #10's Channel<T>-based follow mode — this
/// command intentionally returns a snapshot.
#[tauri::command]
pub async fn tail_logs(
    state: State<'_, AppState>,
    id: String,
    #[allow(non_snake_case)] limit: Option<u32>,
    #[allow(non_snake_case)] offset: Option<u64>,
) -> AppResult<Vec<String>> {
    let client = state.pc_client()?;
    let lines = client
        .logs(&id, offset.unwrap_or(0), limit.unwrap_or(200))
        .await
        .map_err(AppError::Pc)?;
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::parse_dotenv;

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
