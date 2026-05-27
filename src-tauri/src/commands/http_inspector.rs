//! HTTP request inspector — surfaces the traffic flowing through Caddy.
//!
//! Caddy writes a structured **JSON access log** (wired in `caddy::config::
//! with_access_log`, applied by the Caddy sub-reconciler on every `/load`).
//! This module:
//!
//! - **tails** that file on a background thread, parses each line, maps the
//!   request host → project id, and emits a `portbay://request` event so the
//!   `/inspector` UI streams live (mirrors the log-stream tailer + the status
//!   poller's emit pattern);
//! - serves **`recent_requests`** (a bounded tail read of the same file) so the
//!   UI can backfill on open, and **`clear_requests`** to wipe the log.
//!
//! The frontend keeps its own capped buffer, so there's no shared in-memory
//! state here — and no `AppState` change.

use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::caddy::ACCESS_LOG_FILE;
use crate::error::AppResult;
use crate::registry::{store, Registry};
use crate::state::AppState;

/// Event channel the live request stream is emitted on.
const REQUEST_CHANNEL: &str = "portbay://request";

/// Cap on a `recent_requests` backfill, and the slice of file we read for it.
const DEFAULT_LIMIT: u32 = 200;
const MAX_LIMIT: u32 = 2000;
/// Only the last slice of the (possibly large, rotating) access log is read for
/// a backfill — bounds memory regardless of how big the file has grown.
const TAIL_READ_BYTES: u64 = 512 * 1024;

/// One parsed HTTP request/response, as the inspector UI consumes it.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RequestEntry {
    /// Unix milliseconds when Caddy handled the request.
    pub ts: u64,
    pub method: String,
    pub host: String,
    pub uri: String,
    pub status: u16,
    pub duration_ms: f64,
    /// Response size in bytes.
    pub size: u64,
    /// The PortBay project this host maps to, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// Request headers Caddy logged (for the row-detail view).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub req_headers: Option<BTreeMap<String, Vec<String>>>,
}

/// The Caddy access-log path under a PortBay data dir (the registry's parent).
/// The CLI + MCP server have no `AppState`, so they resolve the log this way —
/// one place that knows the `logs/` subdir convention.
pub fn access_log_path(data_dir: &Path) -> PathBuf {
    data_dir.join("logs").join(ACCESS_LOG_FILE)
}

/// Backfill read shared by the `recent_requests` command and the CLI/MCP: the
/// last `limit` parsed entries from `access_log`, oldest→newest, with each host
/// mapped to its project via `reg`. Empty when no log exists yet. `limit`
/// defaults to [`DEFAULT_LIMIT`] and is capped at [`MAX_LIMIT`].
pub fn read_recent(access_log: &Path, limit: Option<u32>, reg: &Registry) -> Vec<RequestEntry> {
    let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
    read_tail_entries(access_log, limit, &host_to_project(reg))
}

/// Truncate the access log so the inspector starts fresh — shared by the
/// `clear_requests` command and the CLI/MCP. Safe cross-process: the app's
/// tailer detects the shrink and reopens the file at the start.
pub fn clear_access_log(access_log: &Path) -> std::io::Result<()> {
    if access_log.exists() {
        std::fs::write(access_log, b"")?;
    }
    Ok(())
}

/// `recent_requests(limit?)` — backfill the inspector from the tail of the
/// access log. Returns oldest→newest. Empty when no log exists yet.
#[tauri::command]
pub fn recent_requests(
    state: State<'_, AppState>,
    limit: Option<u32>,
) -> AppResult<Vec<RequestEntry>> {
    let reg = store::load_or_default(&state.registry_path, state.domain_suffix.clone())?;
    Ok(read_recent(
        &state.logs_dir.join(ACCESS_LOG_FILE),
        limit,
        &reg,
    ))
}

/// `clear_requests()` — truncate the access log so the inspector starts fresh.
#[tauri::command]
pub fn clear_requests(state: State<'_, AppState>) -> AppResult<()> {
    clear_access_log(&state.logs_dir.join(ACCESS_LOG_FILE))?;
    Ok(())
}

/// Spawn the background tailer. Call once at app boot (after the data dir is
/// known). Returns immediately; the tail thread lives for the app's lifetime.
pub fn spawn_request_tailer(app: AppHandle) {
    let (logs_dir, registry_path, domain_suffix) = {
        let state: tauri::State<'_, AppState> = app.state();
        (
            state.logs_dir.clone(),
            state.registry_path.clone(),
            state.domain_suffix.clone(),
        )
    };
    let path = logs_dir.join(ACCESS_LOG_FILE);

    tauri::async_runtime::spawn_blocking(move || {
        // Defence-in-depth: a panic in the tail loop must never take down the
        // app — it just ends the inspector stream (same posture as log_stream).
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            tail_and_emit(&app, &path, &registry_path, &domain_suffix);
        }));
    });
}

/// Follow the access log forever, emitting a `portbay://request` per new line.
fn tail_and_emit(app: &AppHandle, path: &Path, registry_path: &Path, suffix: &str) {
    let mut host_map = load_host_map(registry_path, suffix);
    let mut last_refresh = Instant::now();

    // Caddy creates the file lazily (first served request). Wait, don't give up.
    while !path.exists() {
        std::thread::sleep(Duration::from_secs(1));
    }

    let mut reader = match open_at_end(path) {
        Ok(r) => r,
        Err(_) => return,
    };
    let mut last_len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => {
                // EOF — handle rotation/truncation (Caddy rolls the log), then
                // poll. The registry-derived host map is refreshed lazily here.
                let cur = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
                if cur < last_len {
                    if let Ok(r) = open_at_start(path) {
                        reader = r;
                    }
                    last_len = 0;
                    continue;
                }
                last_len = cur;
                if last_refresh.elapsed() > Duration::from_secs(5) {
                    host_map = load_host_map(registry_path, suffix);
                    last_refresh = Instant::now();
                }
                std::thread::sleep(Duration::from_millis(250));
            }
            Ok(_) => {
                if let Some(entry) = parse_line(line.trim_end(), &host_map) {
                    // A closed window just means no one's listening — keep going.
                    let _ = app.emit(REQUEST_CHANNEL, entry);
                }
            }
            Err(_) => std::thread::sleep(Duration::from_millis(250)),
        }
    }
}

/// Parse one Caddy JSON access-log line into a [`RequestEntry`]. Returns `None`
/// for any line that isn't a request log (no `request` object / no `status`),
/// so non-access lines and partial reads are skipped harmlessly.
fn parse_line(line: &str, host_map: &HashMap<String, String>) -> Option<RequestEntry> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let req = v.get("request")?;
    let status = v.get("status")?.as_u64()? as u16;

    let host = req
        .get("host")
        .and_then(|h| h.as_str())
        .unwrap_or_default()
        .to_string();
    let method = req
        .get("method")
        .and_then(|m| m.as_str())
        .unwrap_or_default()
        .to_string();
    let uri = req
        .get("uri")
        .and_then(|u| u.as_str())
        .unwrap_or_default()
        .to_string();
    let ts = v
        .get("ts")
        .and_then(|t| t.as_f64())
        .map(|s| (s * 1000.0) as u64)
        .unwrap_or(0);
    let duration_ms = v
        .get("duration")
        .and_then(|d| d.as_f64())
        .map(|s| s * 1000.0)
        .unwrap_or(0.0);
    let size = v.get("size").and_then(|s| s.as_u64()).unwrap_or(0);
    let project_id = host_map.get(&host).cloned();
    let req_headers = req.get("headers").and_then(|h| h.as_object()).map(|obj| {
        obj.iter()
            .filter_map(|(k, val)| {
                let vals: Vec<String> = val
                    .as_array()?
                    .iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect();
                Some((k.clone(), vals))
            })
            .collect::<BTreeMap<_, _>>()
    });

    Some(RequestEntry {
        ts,
        method,
        host,
        uri,
        status,
        duration_ms,
        size,
        project_id,
        req_headers,
    })
}

/// Read the last `limit` valid entries from the tail of the log file.
fn read_tail_entries(
    path: &Path,
    limit: usize,
    host_map: &HashMap<String, String>,
) -> Vec<RequestEntry> {
    let Ok(meta) = std::fs::metadata(path) else {
        return vec![];
    };
    let start = meta.len().saturating_sub(TAIL_READ_BYTES);
    let Ok(mut file) = std::fs::File::open(path) else {
        return vec![];
    };
    if start > 0 {
        // Seeking mid-line just yields one unparseable fragment, dropped below.
        let _ = file.seek(SeekFrom::Start(start));
    }
    let mut entries: Vec<RequestEntry> = BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter_map(|l| parse_line(&l, host_map))
        .collect();
    if entries.len() > limit {
        entries = entries.split_off(entries.len() - limit);
    }
    entries
}

fn host_to_project(reg: &Registry) -> HashMap<String, String> {
    reg.list_projects()
        .iter()
        .map(|p| (p.hostname.clone(), p.id.as_str().to_string()))
        .collect()
}

fn load_host_map(registry_path: &Path, suffix: &str) -> HashMap<String, String> {
    store::load_or_default(registry_path, suffix)
        .map(|reg| host_to_project(&reg))
        .unwrap_or_default()
}

fn open_at_end(path: &Path) -> std::io::Result<BufReader<std::fs::File>> {
    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::End(0))?;
    Ok(BufReader::new(file))
}

fn open_at_start(path: &Path) -> std::io::Result<BufReader<std::fs::File>> {
    Ok(BufReader::new(std::fs::File::open(path)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_line() -> &'static str {
        // A representative Caddy JSON access entry.
        r#"{"level":"info","ts":1700000000.5,"logger":"http.log.access.portbay_access","msg":"handled request","request":{"method":"GET","host":"blog.test","uri":"/api/users?page=2","headers":{"User-Agent":["curl/8.0"]}},"duration":0.0123,"size":1024,"status":200}"#
    }

    #[test]
    fn parses_a_caddy_access_line_and_maps_the_project() {
        let mut map = HashMap::new();
        map.insert("blog.test".to_string(), "blog".to_string());
        let e = parse_line(sample_line(), &map).expect("should parse");
        assert_eq!(e.method, "GET");
        assert_eq!(e.host, "blog.test");
        assert_eq!(e.uri, "/api/users?page=2");
        assert_eq!(e.status, 200);
        assert_eq!(e.size, 1024);
        assert_eq!(e.ts, 1_700_000_000_500);
        assert!((e.duration_ms - 12.3).abs() < 0.001);
        assert_eq!(e.project_id.as_deref(), Some("blog"));
        assert_eq!(
            e.req_headers.unwrap().get("User-Agent"),
            Some(&vec!["curl/8.0".to_string()])
        );
    }

    #[test]
    fn unknown_host_yields_no_project_id() {
        let e = parse_line(sample_line(), &HashMap::new()).unwrap();
        assert!(e.project_id.is_none());
    }

    #[test]
    fn non_access_lines_are_skipped() {
        assert!(parse_line("not json", &HashMap::new()).is_none());
        assert!(parse_line(r#"{"level":"info","msg":"serving"}"#, &HashMap::new()).is_none());
        // A request line missing `status` is not an access entry.
        assert!(parse_line(
            r#"{"request":{"host":"x.test","method":"GET","uri":"/"}}"#,
            &HashMap::new()
        )
        .is_none());
    }

    #[test]
    fn tail_read_returns_last_n_entries_newest_last() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("caddy-access.log");
        let mut body = String::new();
        for i in 0..10 {
            body.push_str(&format!(
                r#"{{"ts":{}.0,"request":{{"method":"GET","host":"a.test","uri":"/{}"}},"duration":0.001,"size":1,"status":200}}"#,
                1_700_000_000 + i,
                i
            ));
            body.push('\n');
        }
        std::fs::write(&path, body).unwrap();
        let entries = read_tail_entries(&path, 3, &HashMap::new());
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].uri, "/7");
        assert_eq!(entries[2].uri, "/9");
    }
}
