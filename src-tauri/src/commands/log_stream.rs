//! Live log streaming via `tauri::ipc::Channel<String>`.
//!
//! Tails the per-project log file the PC sub-reconciler tells Process
//! Compose to write at `<data_dir>/PortBay/logs/<id>.log`. Each new line
//! is forwarded to the frontend's `Channel<string>` so the log viewer's
//! Follow mode renders within ~100 ms of write — replacing the 1.5 s
//! polling stub from card #10.
//!
//! Why file-tail instead of PC's WebSocket endpoint:
//! - PC's REST API exposes a streaming endpoint, but its framing format
//!   varies across PC minor versions and ties us to a transport we'd have
//!   to fight on every upgrade.
//! - The log file on disk is the canonical record PC itself writes;
//!   tailing it captures exactly what landed in the persistent log.
//! - Truncate-on-restart is the natural reset signal — when the file
//!   shrinks below our cursor, we reopen and re-stream from the new
//!   start, matching the user's expectation that a project restart
//!   clears the buffer.
//!
//! Lifecycle:
//! - The command spawns a blocking-pool task and returns immediately.
//! - The task polls the file at 100 ms and emits each new line via the
//!   channel.
//! - The frontend dropping the `Channel<string>` causes `send` to error;
//!   the task exits cleanly and the pool thread is released.

use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::PathBuf;
use std::time::Duration;

use tauri::ipc::Channel;
use tauri::State;

use crate::error::AppResult;
use crate::state::AppState;

/// Poll interval for new bytes once the file exists. 100 ms keeps the
/// perceived latency well under the 200 ms DoD target without burning
/// CPU on idle log files.
const TAIL_POLL: Duration = Duration::from_millis(100);

/// How long to wait for the log file to materialise before giving up.
/// PC creates the file lazily; on a freshly-added project, the file
/// shows up within ~3 s once `boot_pc` finishes its restart. 30 s
/// leaves headroom for slow first-time builds.
const FILE_WAIT_TIMEOUT: Duration = Duration::from_secs(30);

/// Cadence for polling the file's existence before it's created.
const FILE_WAIT_POLL: Duration = Duration::from_millis(500);

/// `subscribe_logs(id, channel)` — open a live tail of `<id>.log` and
/// stream each new line via `channel`. Returns immediately; the tail
/// task runs in the background until the channel is dropped on the
/// frontend.
#[tauri::command]
pub fn subscribe_logs(
    state: State<'_, AppState>,
    id: String,
    on_line: Channel<String>,
) -> AppResult<()> {
    let log_path = state.logs_dir.join(format!("{id}.log"));

    tokio::task::spawn_blocking(move || tail_into(&log_path, &on_line));

    Ok(())
}

/// Run the tail loop. Exits when the channel is closed (frontend
/// dropped the `Channel<string>`), the log file fails to appear within
/// the timeout, or an unrecoverable I/O error occurs.
fn tail_into(path: &PathBuf, on_line: &Channel<String>) {
    if !wait_for_file(path, on_line) {
        return;
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
                // EOF — check for truncation (project restart rewrites
                // the file to zero length), then sleep before retry.
                let cur_len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
                if cur_len < last_len {
                    // File truncated; reopen from the start so the user
                    // sees the new run's output.
                    match open_at_start(path) {
                        Ok(r) => reader = r,
                        Err(_) => return,
                    }
                    last_len = 0;
                    // Surface as a quiet inline marker so the user knows
                    // the stream re-attached after a restart.
                    if on_line
                        .send("--- log truncated; re-attached ---".into())
                        .is_err()
                    {
                        return;
                    }
                    continue;
                }
                last_len = cur_len;
                std::thread::sleep(TAIL_POLL);
            }
            Ok(_) => {
                let trimmed = line.trim_end_matches('\n').to_string();
                if on_line.send(trimmed).is_err() {
                    // Channel closed — frontend toggled Follow off or
                    // closed the viewer. Clean exit.
                    return;
                }
            }
            Err(_) => {
                // Any other read error: pause briefly and retry. Avoids
                // tight-looping on transient FS issues.
                std::thread::sleep(TAIL_POLL);
            }
        }
    }
}

/// Block until the log file appears or `FILE_WAIT_TIMEOUT` elapses.
/// Returns `false` on timeout (caller exits silently).
fn wait_for_file(path: &PathBuf, on_line: &Channel<String>) -> bool {
    let deadline = std::time::Instant::now() + FILE_WAIT_TIMEOUT;
    while !path.exists() {
        if std::time::Instant::now() >= deadline {
            let _ = on_line.send(format!(
                "--- log file did not appear within {}s; subscription ended ---",
                FILE_WAIT_TIMEOUT.as_secs()
            ));
            return false;
        }
        std::thread::sleep(FILE_WAIT_POLL);
    }
    true
}

fn open_at_end(path: &PathBuf) -> std::io::Result<BufReader<std::fs::File>> {
    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::End(0))?;
    Ok(BufReader::new(file))
}

fn open_at_start(path: &PathBuf) -> std::io::Result<BufReader<std::fs::File>> {
    Ok(BufReader::new(std::fs::File::open(path)?))
}
