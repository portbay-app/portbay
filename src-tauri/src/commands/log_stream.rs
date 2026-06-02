//! Live log streaming via `tauri::ipc::Channel<String>`.
//!
//! Tails the per-project log file the PC sub-reconciler tells Process
//! Compose to write at `<data_dir>/PortBay/logs/<id>.log`. Each new line
//! is forwarded to the frontend's `Channel<string>` so the log viewer's
//! Follow mode surfaces lines essentially as fast as a native `tail -f`.
//!
//! Why event-driven instead of a fixed poll: a native terminal `tail -f`
//! blocks on a kernel filesystem notification (kqueue/FSEvents) and wakes
//! the instant the file is appended. The old loop slept a flat 100 ms
//! between reads, so every line carried 0–100 ms of dead latency on top of
//! the actual write — perceptible, and the reason PortBay's logs lagged
//! behind a terminal side-by-side. We now register a `notify` watcher on
//! the file and block on its events, so steady-state latency is the FS
//! event delivery time (a few ms), not a poll period. A short timeout is
//! retained purely as a safety net if an event is ever coalesced or the
//! watcher fails to register.
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
//! - The task registers an FS watcher on the file and blocks on its
//!   events, draining and emitting each new line as it's written.
//! - The frontend dropping the `Channel<string>` causes `send` to error;
//!   the task exits cleanly, the watcher is unregistered, and the pool
//!   thread is released.

use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use notify::{RecursiveMode, Watcher};
use tauri::ipc::Channel;
use tauri::State;

use crate::error::AppResult;
use crate::state::AppState;

/// Safety-net wake interval while following an idle file. The primary wake
/// signal is a `notify` filesystem event, which fires within a few ms of
/// the writer appending — so this is *not* the steady-state latency. It
/// only bounds the worst case if an FS event is coalesced or dropped, and
/// it becomes the effective poll period only in the rare case the watcher
/// fails to register at all (see `FALLBACK_POLL`).
const IDLE_FALLBACK: Duration = Duration::from_millis(500);

/// Effective poll period if the FS watcher could not be created. Mirrors
/// the previous always-polling behaviour so following still works — just
/// without the native-feeling latency — rather than degrading to 500 ms.
const FALLBACK_POLL: Duration = Duration::from_millis(100);

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

    // Tauri runs sync commands on its own worker pool, *not* on the
    // tokio runtime. Calling `tokio::task::spawn_blocking` from
    // there panics with "no reactor running" — which is what crashed
    // the app the first time the user clicked Follow. Use Tauri's
    // own helper, which dispatches against the runtime Tauri
    // manages internally for us.
    //
    // The catch_unwind below is defence-in-depth: it caught nothing
    // before because the panic was in *this line*, not inside the
    // closure. Now that the closure can actually start, the unwind
    // armor protects everything tail_into does.
    tauri::async_runtime::spawn_blocking(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            tail_into(&log_path, &on_line);
        }));
        if let Err(payload) = result {
            // The panic payload is usually a &str or String. Pull the
            // message out for the log entry; never re-panic.
            let msg = payload
                .downcast_ref::<&'static str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "(non-string panic payload)".to_string());
            tracing::error!(
                project_id = %id,
                error = %msg,
                "subscribe_logs tail thread panicked — subscription ended",
            );
            // Try to notify the frontend so the UI can clear its
            // follow state, but ignore failure (the channel may
            // already be torn down).
            let _ = on_line.send(format!("--- log stream ended ({msg}) ---"));
        }
    });

    Ok(())
}

/// Run the tail loop. Exits when the channel is closed (frontend
/// dropped the `Channel<string>`), the log file fails to appear within
/// the timeout, or an unrecoverable I/O error occurs.
fn tail_into(path: &PathBuf, on_line: &Channel<String>) {
    if !wait_for_file(path, on_line) {
        return;
    }

    // Register the FS watcher *before* the first read so we can never miss
    // an append that lands between reading to EOF and starting to wait.
    // `_watcher` is bound for the lifetime of the loop; dropping it on
    // return unregisters the watch. When `None`, the file couldn't be
    // watched and we fall back to a fixed poll (`FALLBACK_POLL`).
    let (tx, rx) = channel::<()>();
    let _watcher = make_watcher(path, tx);
    let idle_wait = if _watcher.is_some() {
        IDLE_FALLBACK
    } else {
        FALLBACK_POLL
    };

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
                // the file to zero length), then block until the watcher
                // signals a change or the safety timeout elapses.
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
                wait_for_change(&rx, idle_wait);
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
                std::thread::sleep(idle_wait);
            }
        }
    }
}

/// Block until the watcher reports a change to the log file or `timeout`
/// elapses, whichever comes first — the event-driven equivalent of a poll
/// sleep. A burst of writes is collapsed into a single wakeup by draining
/// any queued ticks, so the caller does one read pass per quiet period
/// rather than once per FS event.
fn wait_for_change(rx: &Receiver<()>, timeout: Duration) {
    match rx.recv_timeout(timeout) {
        Ok(()) => {
            // Drain coalesced events so we don't spin one read per tick.
            while rx.try_recv().is_ok() {}
        }
        // No event in the window: re-read anyway (safety net) or, when the
        // watcher never registered, this is the steady-state poll tick.
        Err(RecvTimeoutError::Timeout) => {}
        // Watcher thread gone (sender dropped): degrade to a timed wait so
        // following keeps working at poll latency instead of busy-looping.
        Err(RecvTimeoutError::Disconnected) => std::thread::sleep(timeout),
    }
}

/// Build a `notify` watcher that forwards a unit tick through `tx` on each
/// filesystem event for `path`. Returns `None` if the watcher can't be
/// created or the watch can't be registered — the caller then falls back
/// to fixed-interval polling. The file is watched directly (non-recursive);
/// FSEvents tracks it by path across the truncate-and-rewrite a project
/// restart performs.
fn make_watcher(path: &Path, tx: Sender<()>) -> Option<notify::RecommendedWatcher> {
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        // Any successful event means "something changed, read again". A
        // send error just means the tail loop has already exited; ignore.
        if res.is_ok() {
            let _ = tx.send(());
        }
    })
    .ok()?;
    watcher.watch(path, RecursiveMode::NonRecursive).ok()?;
    Some(watcher)
}

/// Block until the log file appears or `FILE_WAIT_TIMEOUT` elapses.
/// Returns `false` on timeout (caller exits silently).
fn wait_for_file(path: &Path, on_line: &Channel<String>) -> bool {
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
