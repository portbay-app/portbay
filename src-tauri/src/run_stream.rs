//! Live run-log streaming: tail every live lease's transcript file and emit
//! appended lines as Tauri events, so the board watches agents in real time
//! instead of re-fetching a snapshot.
//!
//! One background task tails *all* runs across all projects (leases carry
//! their `log_path`). Two consumers:
//! - `PortBayAgentPanel` appends the lines to its open transcript;
//! - the tasks store keeps a per-card "latest action" line for the board-level
//!   glance view on running cards.
//!
//! The tailer is stateless on the frontend's behalf: a panel that mounts
//! mid-run gets its history from `task_run_log` and only the *new* lines from
//! here. On first sight of a log the tailer emits just the last complete line
//! (catch-up for the card chip), then streams appends.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::registry::store;
use crate::AppState;

/// Tauri event channel appended run-log lines are emitted on.
pub const RUN_LOG_CHANNEL: &str = "portbay://task-run-log";

/// Tail cadence. Each tick is a metadata stat per live lease (and a read only
/// when the file grew), so this stays cheap; ~1s keeps the stream feeling live.
const TAIL_INTERVAL: Duration = Duration::from_millis(900);

/// On first sight of an already-running log, look back at most this far for
/// the catch-up line — never replay a whole transcript through the event bus.
const CATCH_UP_BYTES: u64 = 8 * 1024;

/// Cap on lines per emitted chunk; a torrent (e.g. a build log dump) is
/// truncated to its newest lines rather than flooding the webview.
const MAX_LINES_PER_CHUNK: usize = 120;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunLogChunk {
    pub project_id: String,
    pub card_id: String,
    pub run_id: String,
    /// Complete appended lines (no trailing newline). For a catch-up emit this
    /// is just the transcript's current last line.
    pub lines: Vec<String>,
    /// True for the first emit after the tailer discovers an in-flight run —
    /// the lines are a snapshot of "where it is now", not an append.
    pub catch_up: bool,
}

struct TailState {
    offset: u64,
    /// Bytes after the last newline — held until the line completes.
    partial: String,
    /// Tick stamp for retiring tails whose lease is gone.
    seen_tick: u64,
}

/// Spawn the background run-log tailer. Returns immediately; the task runs for
/// the lifetime of the app handle.
pub fn spawn_run_log_tailer(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut tails: HashMap<String, TailState> = HashMap::new();
        let mut tick_no: u64 = 0;
        let mut tick = tokio::time::interval(TAIL_INTERVAL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            tick_no += 1;
            let state: tauri::State<AppState> = app.state();
            let Ok(reg) = store::load_or_default(&state.registry_path, &state.domain_suffix) else {
                continue;
            };
            for project in reg.list_projects() {
                let Ok(rt) = crate::context::runtime_state::load(&project.path) else {
                    continue;
                };
                for (card_id, lease) in &rt.leases {
                    let Some(log_path) = lease.log_path.as_deref() else {
                        continue;
                    };
                    if let Some(mut chunk) = poll_tail(
                        &mut tails,
                        tick_no,
                        log_path,
                        &project.id.to_string(),
                        card_id,
                        &lease.run_id,
                    ) {
                        normalize_chunk(&mut chunk, &lease.agent);
                        let _ = app.emit(RUN_LOG_CHANNEL, chunk);
                    }
                }
            }
            // Retire tails whose lease vanished (run finished/reclaimed).
            tails.retain(|_, t| t.seen_tick == tick_no);
        }
    });
}

/// Normalize a chunk's lines into the `portbay-event` envelope per the
/// producing agent (`normalize_run_log_line`) — the panel parses only the
/// envelope, so engine-specific NDJSON never reaches the event bus.
fn normalize_chunk(chunk: &mut RunLogChunk, agent: &str) {
    for line in &mut chunk.lines {
        if let Some(normalized) = crate::context::automation::normalize_run_log_line(agent, line) {
            *line = normalized;
        }
    }
}

/// Advance one log's tail; returns a chunk when new complete lines appeared.
fn poll_tail(
    tails: &mut HashMap<String, TailState>,
    tick_no: u64,
    log_path: &str,
    project_id: &str,
    card_id: &str,
    run_id: &str,
) -> Option<RunLogChunk> {
    let len = std::fs::metadata(log_path).ok()?.len();
    let first = !tails.contains_key(log_path);
    let entry = tails.entry(log_path.to_string()).or_insert(TailState {
        offset: len.saturating_sub(CATCH_UP_BYTES.min(len)),
        partial: String::new(),
        seen_tick: tick_no,
    });
    entry.seen_tick = tick_no;
    // A shrunk file means it was rewritten — start over from the top.
    if len < entry.offset {
        entry.offset = 0;
        entry.partial.clear();
    }
    if len == entry.offset {
        return None;
    }

    let mut file = std::fs::File::open(log_path).ok()?;
    file.seek(SeekFrom::Start(entry.offset)).ok()?;
    let mut buf = Vec::with_capacity((len - entry.offset) as usize);
    file.read_to_end(&mut buf).ok()?;
    entry.offset = len;
    let text = format!("{}{}", entry.partial, String::from_utf8_lossy(&buf));

    // Keep everything after the last newline as the partial; the rest splits
    // into complete lines.
    let (complete, partial) = match text.rfind('\n') {
        Some(p) => (&text[..p], &text[p + 1..]),
        None => ("", text.as_str()),
    };
    entry.partial = partial.to_string();
    let mut lines: Vec<String> = complete
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    if first {
        // Catch-up: the chunk began mid-transcript (and possibly mid-line) —
        // only the last complete line is trustworthy and interesting.
        lines = lines.pop().into_iter().collect();
    } else if lines.len() > MAX_LINES_PER_CHUNK {
        lines.drain(..lines.len() - MAX_LINES_PER_CHUNK);
    }
    if lines.is_empty() {
        return None;
    }
    Some(RunLogChunk {
        project_id: project_id.to_string(),
        card_id: card_id.to_string(),
        run_id: run_id.to_string(),
        lines,
        catch_up: first,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk_for(
        tails: &mut HashMap<String, TailState>,
        tick: u64,
        path: &std::path::Path,
    ) -> Option<RunLogChunk> {
        poll_tail(tails, tick, path.to_str().unwrap(), "p", "t_1", "r_1")
    }

    #[test]
    fn tail_emits_catch_up_then_appends_and_holds_partials() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("r_1.jsonl");
        std::fs::write(&log, "{\"type\":\"ack\"}\n{\"type\":\"tool_start\"}\n").unwrap();
        let mut tails = HashMap::new();

        // First sight: only the last complete line, flagged as catch-up.
        let c = chunk_for(&mut tails, 1, &log).expect("catch-up chunk");
        assert!(c.catch_up);
        assert_eq!(c.lines, vec!["{\"type\":\"tool_start\"}"]);

        // No growth → no chunk.
        assert!(chunk_for(&mut tails, 2, &log).is_none());

        // Append a complete line + a partial: only the complete one is emitted…
        let mut f = std::fs::OpenOptions::new().append(true).open(&log).unwrap();
        use std::io::Write;
        f.write_all(b"{\"type\":\"tool_result\"}\n{\"type\":\"don")
            .unwrap();
        drop(f);
        let c = chunk_for(&mut tails, 3, &log).expect("append chunk");
        assert!(!c.catch_up);
        assert_eq!(c.lines, vec!["{\"type\":\"tool_result\"}"]);

        // …and the partial completes on the next write.
        let mut f = std::fs::OpenOptions::new().append(true).open(&log).unwrap();
        f.write_all(b"e\"}\n").unwrap();
        drop(f);
        let c = chunk_for(&mut tails, 4, &log).expect("completed partial");
        assert_eq!(c.lines, vec!["{\"type\":\"done\"}"]);
    }

    #[test]
    fn fork_engine_lines_arrive_as_the_envelope() {
        // The P6 panel cutover: a `portbay` (Cline-fork) run's `agent_event`
        // NDJSON is normalized to the portbay-event envelope before it hits
        // the event bus; prose lines stream through untouched.
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("r_2.jsonl");
        std::fs::write(&log, "warming up\n").unwrap();
        let mut tails = HashMap::new();
        let _ = poll_tail(&mut tails, 1, log.to_str().unwrap(), "p", "t_2", "r_2");

        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().append(true).open(&log).unwrap();
        f.write_all(
            b"{\"type\":\"tool_start\",\"toolName\":\"bash\",\"text\":\"ls\"}\nplain prose\n",
        )
        .unwrap();
        drop(f);
        let mut c = poll_tail(&mut tails, 2, log.to_str().unwrap(), "p", "t_2", "r_2")
            .expect("append chunk");
        normalize_chunk(&mut c, "portbay");
        assert_eq!(
            c.lines,
            vec![
                "{\"event\":\"tool\",\"name\":\"bash\",\"phase\":\"start\",\"text\":\"ls\"}",
                "plain prose"
            ]
        );
    }
}
