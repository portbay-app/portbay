//! Local speech-to-text — the `portbay-stt` sidecar client.
//!
//! Smart Dictation's transcription half. macOS dictation (the default
//! engine, `commands::system::start_dictation`) types speech into the field
//! itself; this module is the alternative: a PortBay-managed local engine
//! where the bundled sidecar (src-tauri/stt/) captures the mic and runs
//! Whisper (WhisperKit) or Parakeet (FluidAudio) CoreML models on-device.
//! Audio never leaves the machine; the rewrite layer (`dictation.rs`) sits
//! on top of the transcript exactly as it does for macOS dictation.
//!
//! Wire protocol: line-delimited JSON over stdin/stdout, documented at the
//! top of src-tauri/stt/Sources/portbay-stt/main.swift. This module owns
//! binary resolution and the serve-mode client; the Tauri commands live in
//! `commands::stt`.

#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use serde::{Deserialize, Serialize};

/// Status of the local STT engine, as probed for the settings/AI-page UI.
/// `reason` is machine-readable copy-selection for the frontend:
/// `requires_macos_14` | `sidecar_missing` | `sidecar_failed` | `unsupported`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SttStatus {
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Engine libraries linked into the sidecar (`whisper`, `parakeet`),
    /// reported by `--check` so a build that dropped one is visible here.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub engines: Vec<String>,
}

impl SttStatus {
    fn unavailable(reason: &str) -> Self {
        Self {
            available: false,
            reason: Some(reason.to_string()),
            engines: Vec::new(),
        }
    }
}

/// `--check` output shape (see main.swift).
#[derive(Debug, Deserialize)]
struct CheckOutput {
    available: bool,
    reason: Option<String>,
    #[serde(default)]
    engines: Vec<String>,
}

/// Locate the bundled sidecar. Same search order as `resolve_afm_binary`
/// (dictation.rs): plain name next to the running executable (packaged .app
/// and `tauri dev`, where the CLI strips the triple suffix), then
/// triple-suffixed next to the exe (bare `cargo run`), then the source-tree
/// binaries dir (dev/test runs from a checkout — the baked path simply
/// won't exist on user machines).
pub fn resolve_stt_binary() -> Option<std::path::PathBuf> {
    use std::env::consts::{ARCH, OS};

    // Test/diagnostic override: point the sidecar at an arbitrary executable
    // (a fake sidecar speaking the JSON protocol) without a real build. Only
    // honored when set, so production resolution is unaffected.
    if let Ok(path) = std::env::var("PORTBAY_STT_BIN") {
        let p = std::path::PathBuf::from(path);
        if p.exists() {
            return Some(p);
        }
    }

    let triple = match (OS, ARCH) {
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("macos", "x86_64") => Some("x86_64-apple-darwin"),
        _ => None,
    };

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let plain = dir.join("portbay-stt");
            if plain.exists() {
                return Some(plain);
            }
            if let Some(triple) = triple {
                let suffixed = dir.join(format!("portbay-stt-{triple}"));
                if suffixed.exists() {
                    return Some(suffixed);
                }
            }
        }
    }

    if let Some(triple) = triple {
        let dev = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("binaries")
            .join(format!("portbay-stt-{triple}"));
        if dev.exists() {
            return Some(dev);
        }
    }
    None
}

/// Probe the sidecar (`portbay-stt --check`). The sidecar's deployment
/// target is macOS 14 (the engine libraries' floor) — on older macOS the
/// exec itself fails, which this maps to `requires_macos_14` so the UI can
/// say why instead of a generic failure.
#[cfg(target_os = "macos")]
pub async fn check() -> SttStatus {
    let Some(binary) = resolve_stt_binary() else {
        return SttStatus::unavailable("sidecar_missing");
    };

    // The placeholder files build scripts seed for tauri's externalBin
    // existence check are zero bytes — treat them as missing, not broken.
    if std::fs::metadata(&binary).map(|m| m.len()).unwrap_or(0) == 0 {
        return SttStatus::unavailable("sidecar_missing");
    }

    let output = tokio::process::Command::new(&binary)
        .arg("--check")
        .stdin(std::process::Stdio::null())
        .output()
        .await;

    let output = match output {
        Ok(out) => out,
        Err(_) => return SttStatus::unavailable(exec_failure_reason()),
    };
    if !output.status.success() {
        return SttStatus::unavailable(exec_failure_reason());
    }

    match serde_json::from_slice::<CheckOutput>(&output.stdout) {
        Ok(check) => SttStatus {
            available: check.available,
            reason: check.reason,
            engines: check.engines,
        },
        Err(_) => SttStatus::unavailable("sidecar_failed"),
    }
}

#[cfg(not(target_os = "macos"))]
pub async fn check() -> SttStatus {
    SttStatus::unavailable("unsupported")
}

/// Distinguish "this Mac is too old to run the sidecar" from "the sidecar is
/// broken": the binary's deployment target is macOS 14, so on 13 and older
/// the exec fails by design. `sw_vers` is authoritative and cheap.
#[cfg(target_os = "macos")]
fn exec_failure_reason() -> &'static str {
    let major = std::process::Command::new("/usr/bin/sw_vers")
        .arg("-productVersion")
        .output()
        .ok()
        .and_then(|out| {
            String::from_utf8_lossy(&out.stdout)
                .trim()
                .split('.')
                .next()
                .and_then(|m| m.parse::<u32>().ok())
        });
    match major {
        Some(m) if m < 14 => "requires_macos_14",
        _ => "sidecar_failed",
    }
}

// --- Serve-mode client -------------------------------------------------------
//
// Two transports, both per-call spawns of `portbay-stt --serve`:
//   • `one_shot_op` — metadata ops (catalog / installed / delete): write one
//     request line, read until its terminal response, done. Process start is
//     ~30 ms; keeping a server warm for these would be machinery without a
//     win (unlike AFM, where the warm server skips framework load per
//     rewrite — these ops load nothing).
//   • `run_download` — one dedicated process per download: progress events
//     stream until the terminal `download` response. Cancel is a
//     `cancel-download` line written into that process's stdin (kept in
//     ACTIVE_DOWNLOADS), mirroring `ollama_cancel_pull`'s id-keyed registry.

#[cfg(target_os = "macos")]
pub use client::*;

#[cfg(target_os = "macos")]
mod client {
    use std::collections::{HashMap, HashSet};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use once_cell::sync::Lazy;
    use serde_json::Value;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    use super::resolve_stt_binary;

    /// Ceiling for metadata ops — they only touch the local disk. Generous
    /// because `installed` walks model directories (gigabytes of mlmodelc
    /// files) for sizes.
    const OP_TIMEOUT: Duration = Duration::from_secs(20);

    /// Stdin handles of in-flight downloads, keyed by the app-chosen
    /// download id, so `stt_cancel_download` can reach into the right
    /// process. Entries are removed when the download task finishes.
    static ACTIVE_DOWNLOADS: Lazy<Mutex<HashMap<String, Arc<tokio::sync::Mutex<tokio::process::ChildStdin>>>>> =
        Lazy::new(|| Mutex::new(HashMap::new()));

    /// Download ids cancelled before their stdin was registered in
    /// `ACTIVE_DOWNLOADS`. Closes the race where a very fast cancel lands in
    /// the spawn→register window and would otherwise be a silent no-op
    /// (mirrors Ollama's `CANCELLED_PULLS`). `run_download` consumes the flag
    /// once registered; both registries are cleared when the task ends.
    static CANCELLED_DOWNLOADS: Lazy<Mutex<HashSet<String>>> =
        Lazy::new(|| Mutex::new(HashSet::new()));

    /// Guards against two concurrent prewarms racing the same multi-GB CoreML
    /// load to no benefit (the real session has its own `ACTIVE_CAPTURE`
    /// guard). A prewarm already in flight makes a second one a no-op.
    static PREWARMING: AtomicBool = AtomicBool::new(false);

    fn spawn_serve() -> Result<tokio::process::Child, String> {
        let binary = resolve_stt_binary().ok_or("speech-to-text sidecar is missing")?;
        if std::fs::metadata(&binary).map(|m| m.len()).unwrap_or(0) == 0 {
            return Err("speech-to-text sidecar is missing".to_string());
        }
        tokio::process::Command::new(&binary)
            .arg("--serve")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("failed to start speech-to-text sidecar: {e}"))
    }

    /// Run one metadata op and return its terminal response line. Event
    /// lines (none expected for these ops) are skipped, not errors.
    pub async fn one_shot_op(request: Value) -> Result<Value, String> {
        one_shot_op_with_timeout(request, OP_TIMEOUT).await
    }

    async fn one_shot_op_with_timeout(request: Value, timeout: Duration) -> Result<Value, String> {
        let op = request
            .get("op")
            .and_then(Value::as_str)
            .ok_or("request is missing op")?
            .to_string();
        let mut child = spawn_serve()?;
        let mut stdin = child.stdin.take().ok_or("sidecar stdin unavailable")?;
        let stdout = child.stdout.take().ok_or("sidecar stdout unavailable")?;

        let mut line = serde_json::to_string(&request).map_err(|e| e.to_string())?;
        line.push('\n');
        stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|e| format!("sidecar write failed: {e}"))?;
        // EOF after the one request: the sidecar answers, then exits clean.
        drop(stdin);

        let mut lines = BufReader::new(stdout).lines();
        tokio::time::timeout(timeout, async {
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(value) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if value.get("event").is_some() {
                    continue;
                }
                if value.get("op").and_then(Value::as_str) == Some(op.as_str()) {
                    return Ok(value);
                }
            }
            Err(format!("sidecar closed without answering {op}"))
        })
        .await
        .map_err(|_| format!("sidecar timed out answering {op}"))?
    }

    /// What a download reported when it finished.
    pub struct DownloadOutcome {
        pub success: bool,
        pub cancelled: bool,
        pub error: Option<String>,
    }

    /// Run one model download in a dedicated sidecar process, relaying each
    /// progress event into `on_progress(fraction, phase)`. Returns when the
    /// sidecar sends the download's terminal response (or dies).
    pub async fn run_download(
        models_dir: &str,
        model: &str,
        download_id: &str,
        mut on_progress: impl FnMut(f64, String),
    ) -> Result<DownloadOutcome, String> {
        let mut child = spawn_serve()?;
        let mut stdin = child.stdin.take().ok_or("sidecar stdin unavailable")?;
        let stdout = child.stdout.take().ok_or("sidecar stdout unavailable")?;

        let request = serde_json::json!({
            "op": "download",
            "modelsDir": models_dir,
            "model": model,
            "downloadId": download_id,
        });
        let mut line = serde_json::to_string(&request).map_err(|e| e.to_string())?;
        line.push('\n');
        stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|e| format!("sidecar write failed: {e}"))?;

        // Park the stdin where stt_cancel_download can find it. NOT dropped
        // here — EOF would end the sidecar's read loop, and the cancel line
        // still needs a way in.
        let stdin = Arc::new(tokio::sync::Mutex::new(stdin));
        ACTIVE_DOWNLOADS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(download_id.to_string(), Arc::clone(&stdin));

        // Honor a cancel that raced ahead of registration: now that stdin is
        // parked, push the cancel line before entering the read loop.
        let pre_cancelled = CANCELLED_DOWNLOADS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(download_id);
        if pre_cancelled {
            let line = format!(
                "{}\n",
                serde_json::json!({ "op": "cancel-download", "downloadId": download_id })
            );
            let _ = stdin.lock().await.write_all(line.as_bytes()).await;
        }

        let mut lines = BufReader::new(stdout).lines();
        let outcome = loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    let Ok(value) = serde_json::from_str::<Value>(&line) else {
                        continue;
                    };
                    if value.get("event").and_then(Value::as_str) == Some("progress") {
                        let fraction = value.get("fraction").and_then(Value::as_f64).unwrap_or(0.0);
                        let phase = value
                            .get("phase")
                            .and_then(Value::as_str)
                            .unwrap_or("downloading")
                            .to_string();
                        on_progress(fraction, phase);
                        continue;
                    }
                    if value.get("op").and_then(Value::as_str) == Some("download") {
                        let ok = value.get("ok").and_then(Value::as_bool).unwrap_or(false);
                        let code = value.get("code").and_then(Value::as_i64);
                        break Ok(DownloadOutcome {
                            success: ok,
                            cancelled: code == Some(6),
                            error: value
                                .get("error")
                                .and_then(Value::as_str)
                                .map(str::to_string),
                        });
                    }
                }
                Ok(None) => break Err("sidecar exited mid-download".to_string()),
                Err(e) => break Err(format!("sidecar read failed: {e}")),
            }
        };

        ACTIVE_DOWNLOADS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(download_id);
        CANCELLED_DOWNLOADS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(download_id);
        outcome
    }

    /// Ask an in-flight download's sidecar to cancel. No-op for unknown ids
    /// (already finished — same contract as `ollama_cancel_pull`).
    pub async fn cancel_download(download_id: &str) {
        // Record the intent first so a cancel that beats `run_download`'s
        // registration still takes effect when it checks the flag on start.
        CANCELLED_DOWNLOADS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(download_id.to_string());
        let stdin = ACTIVE_DOWNLOADS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(download_id)
            .cloned();
        if let Some(stdin) = stdin {
            let line = format!(
                "{}\n",
                serde_json::json!({ "op": "cancel-download", "downloadId": download_id })
            );
            let _ = stdin.lock().await.write_all(line.as_bytes()).await;
        }
    }

    // --- Capture session (resident-engine transport) -----------------------
    //
    // ONE persistent `portbay-stt --serve` process owns the capture + prewarm
    // path and is kept ALIVE across captures, so the model it loaded stays
    // resident in the sidecar's EngineCache and Fn-hold goes mic-hot without
    // reloading multi-GB CoreML weights. (Metadata ops and downloads keep
    // their own throwaway processes — they load nothing the resident process
    // needs.) This is the difference between a ~1 s warm start per capture and
    // the instant start the always-on dictation promise needs.
    //
    // A single stdout reader per process multiplexes the line protocol:
    //   listening → resolves the start; emits dictation://listening
    //   partial   → stt://partial {text}
    //   level     → stt://level {rms}
    //   final     → resolves stop_capture's text
    //   ended     → dictation://ended
    //   {op:start-capture, ok:false} → fails the start (no listening came)
    //   {op:prewarm, ...}            → resolves a pending ensure_engine load
    // Capture events route to ROUTING (the one active capture — machine-wide
    // single, like macOS dictation); the prewarm response routes to CONTROL.
    // A generation stamp keeps a replaced process's dying reader from clearing
    // its successor's slot.

    /// The resident serve process for captures + prewarm.
    struct EngineProc {
        stdin: Arc<tokio::sync::Mutex<tokio::process::ChildStdin>>,
        /// The model loaded resident in this process — the fast-path key.
        model: String,
        models_dir: String,
        /// Identity for the reader's EOF cleanup (see `ENGINE_GEN`).
        generation: u64,
        /// Dropping reaps the process (`kill_on_drop`): releases the mic if a
        /// capture was live and frees the resident model from RAM.
        _child: tokio::process::Child,
    }

    static ENGINE: Lazy<Mutex<Option<EngineProc>>> = Lazy::new(|| Mutex::new(None));
    /// Serializes engine (re)spawns so a startup prewarm and a first Fn-hold
    /// racing `ensure_engine` don't launch two processes.
    static ENGINE_LOCK: Lazy<tokio::sync::Mutex<()>> = Lazy::new(|| tokio::sync::Mutex::new(()));
    /// Monotonic process identity; the reader only clears `ENGINE` on EOF when
    /// the still-registered process is its own (a model switch replaces the
    /// process, and the old reader must not evict the new one).
    static ENGINE_GEN: AtomicU64 = AtomicU64::new(0);

    /// Channels the reader resolves for the one active capture.
    struct Routing {
        app: tauri::AppHandle,
        started_tx: Option<tokio::sync::oneshot::Sender<Result<(), String>>>,
        final_tx: Option<tokio::sync::oneshot::Sender<String>>,
    }
    static ROUTING: Lazy<Mutex<Option<Routing>>> = Lazy::new(|| Mutex::new(None));

    /// Pending prewarm (control-op) response. One in flight at a time —
    /// micSession serializes start/stop and a capture never overlaps a prewarm.
    static CONTROL: Lazy<Mutex<Option<tokio::sync::oneshot::Sender<Value>>>> =
        Lazy::new(|| Mutex::new(None));

    /// The active capture's final-text receiver + a handle to write stop/cancel
    /// into the resident process. Taken by stop/cancel; the process itself
    /// lives in `ENGINE` and is NOT dropped here (that would kill residency).
    struct ActiveCapture {
        stdin: Arc<tokio::sync::Mutex<tokio::process::ChildStdin>>,
        final_rx: tokio::sync::oneshot::Receiver<String>,
    }
    static ACTIVE_CAPTURE: Lazy<Mutex<Option<ActiveCapture>>> = Lazy::new(|| Mutex::new(None));

    /// Model-load ceiling for a cold engine spawn (first use after boot / OS
    /// update re-specialization pages in multi-GB CoreML weights). Warm starts
    /// — the resident path — confirm in well under a second.
    const START_TIMEOUT: Duration = Duration::from_secs(60);
    /// Final-transcription ceiling: the full-buffer pass on a long dictation
    /// with the slowest model (large-v3) — generous, like Ollama's 90 s.
    const FINAL_TIMEOUT: Duration = Duration::from_secs(180);

    /// Serialize a JSON request and write it (newline-framed) into a serve
    /// process's stdin.
    async fn write_line(
        stdin: &Arc<tokio::sync::Mutex<tokio::process::ChildStdin>>,
        value: Value,
    ) -> Result<(), String> {
        let mut line = serde_json::to_string(&value).map_err(|e| e.to_string())?;
        line.push('\n');
        stdin
            .lock()
            .await
            .write_all(line.as_bytes())
            .await
            .map_err(|e| format!("sidecar write failed: {e}"))
    }

    /// Route one capture event to the active capture's channels / Tauri events.
    fn route_event(event: &str, value: &Value) {
        use tauri::Emitter;
        let mut routing = ROUTING.lock().unwrap_or_else(|e| e.into_inner());
        let Some(rt) = routing.as_mut() else {
            return;
        };
        match event {
            "listening" => {
                if let Some(tx) = rt.started_tx.take() {
                    let _ = tx.send(Ok(()));
                }
                let _ = rt.app.emit("dictation://listening", ());
            }
            "partial" => {
                let text = value.get("text").and_then(Value::as_str).unwrap_or("");
                let _ = rt.app.emit("stt://partial", serde_json::json!({ "text": text }));
            }
            "level" => {
                let rms = value.get("rms").and_then(Value::as_f64).unwrap_or(0.0);
                let _ = rt.app.emit("stt://level", serde_json::json!({ "rms": rms }));
            }
            "final" => {
                if let Some(tx) = rt.final_tx.take() {
                    let _ = tx.send(
                        value
                            .get("text")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                    );
                }
            }
            "ended" => {
                let _ = rt.app.emit("dictation://ended", ());
            }
            _ => {}
        }
    }

    /// One reader per resident process: demux events + control responses until
    /// the process's stdout closes, then fail in-flight waiters and (if this is
    /// still the registered process) clear the slot so the next caller respawns.
    fn spawn_reader(stdout: tokio::process::ChildStdout, generation: u64) {
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(value) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if let Some(event) = value.get("event").and_then(Value::as_str) {
                    route_event(event, &value);
                } else if let Some(op) = value.get("op").and_then(Value::as_str) {
                    match op {
                        // A start-capture response only arrives on FAILURE —
                        // success is signalled by the `listening` event.
                        "start-capture" => {
                            let detail = value
                                .get("error")
                                .and_then(Value::as_str)
                                .unwrap_or("capture failed to start")
                                .to_string();
                            if let Some(rt) =
                                ROUTING.lock().unwrap_or_else(|e| e.into_inner()).as_mut()
                            {
                                if let Some(tx) = rt.started_tx.take() {
                                    let _ = tx.send(Err(detail));
                                }
                            }
                        }
                        "prewarm" => {
                            if let Some(tx) =
                                CONTROL.lock().unwrap_or_else(|e| e.into_inner()).take()
                            {
                                let _ = tx.send(value);
                            }
                        }
                        // stop/cancel responses are unused (the `final` event
                        // carries the text); metadata never comes through here.
                        _ => {}
                    }
                }
            }
            // EOF — the process died. Only the STILL-REGISTERED process cleans
            // up: a superseded reader (its process replaced by a model switch)
            // must not steal the new process's pending CONTROL/ROUTING waiters,
            // which would spuriously fail the new engine.
            let still_current = {
                let mut engine = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
                if engine.as_ref().map(|e| e.generation) == Some(generation) {
                    *engine = None;
                    true
                } else {
                    false
                }
            };
            if still_current {
                if let Some(rt) = ROUTING.lock().unwrap_or_else(|e| e.into_inner()).take() {
                    if let Some(tx) = rt.started_tx {
                        let _ = tx.send(Err("speech-to-text engine exited".into()));
                    }
                    // final_tx drops here → stop_capture's recv errors out.
                }
                if let Some(tx) = CONTROL.lock().unwrap_or_else(|e| e.into_inner()).take() {
                    let _ = tx.send(serde_json::json!({ "op": "prewarm", "ok": false, "error": "engine exited" }));
                }
            }
        });
    }

    /// Ensure a resident serve process loaded with `model` exists, spawning +
    /// prewarming one (or replacing a different-model process) on a miss.
    /// Returns its stdin for the caller to drive start/stop against. Serialized
    /// by `ENGINE_LOCK` so concurrent callers don't double-spawn.
    async fn ensure_engine(
        models_dir: &str,
        model: &str,
    ) -> Result<Arc<tokio::sync::Mutex<tokio::process::ChildStdin>>, String> {
        let _guard = ENGINE_LOCK.lock().await;

        // Fast path: a live process already holding this exact model.
        {
            let engine = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(e) = engine.as_ref() {
                if e.model == model && e.models_dir == models_dir {
                    return Ok(e.stdin.clone());
                }
            }
        }
        // Replace any existing (different-model / dead) process: drop reaps it.
        drop(ENGINE.lock().unwrap_or_else(|e| e.into_inner()).take());

        let mut child = spawn_serve()?;
        let stdin = child.stdin.take().ok_or("sidecar stdin unavailable")?;
        let stdout = child.stdout.take().ok_or("sidecar stdout unavailable")?;
        let stdin = Arc::new(tokio::sync::Mutex::new(stdin));
        let generation = ENGINE_GEN.fetch_add(1, Ordering::SeqCst) + 1;

        // Register before the reader runs so an instant EOF finds the slot.
        *ENGINE.lock().unwrap_or_else(|e| e.into_inner()) = Some(EngineProc {
            stdin: stdin.clone(),
            model: model.to_string(),
            models_dir: models_dir.to_string(),
            generation,
            _child: child,
        });
        spawn_reader(stdout, generation);

        // Prewarm: load + cache the model resident in the sidecar so the first
        // capture is instant. The reader resolves CONTROL with the response.
        let (tx, rx) = tokio::sync::oneshot::channel::<Value>();
        *CONTROL.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
        write_line(
            &stdin,
            serde_json::json!({ "op": "prewarm", "modelsDir": models_dir, "model": model }),
        )
        .await?;
        let resp = tokio::time::timeout(START_TIMEOUT, rx)
            .await
            .map_err(|_| "engine prewarm timed out".to_string())?
            .map_err(|_| "engine exited during prewarm".to_string())?;
        if resp.get("ok").and_then(Value::as_bool) != Some(true) {
            // Bad model / load failure: drop the process, surface the reason.
            drop(ENGINE.lock().unwrap_or_else(|e| e.into_inner()).take());
            return Err(resp
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("engine failed to load")
                .to_string());
        }
        Ok(stdin)
    }

    /// Drop the resident engine process — call after deleting the model it
    /// holds so a stale engine (pointing at removed files) can't serve the
    /// next capture. The next prewarm/start respawns and reloads.
    pub fn evict_engine() {
        drop(ENGINE.lock().unwrap_or_else(|e| e.into_inner()).take());
    }

    /// Start a capture session against the resident engine process. Resolves
    /// once the sidecar's mic is hot (its `listening` event) or the start
    /// failed. `ensure_engine` reuses the prewarmed, model-resident process so
    /// the common path is an instant mic-hot — no per-capture model reload. Any
    /// routing left in the slot is cleared first (micSession serializes
    /// stop→start, so anything here is an orphan of a crashed flow).
    pub async fn start_capture(
        app: tauri::AppHandle,
        models_dir: &str,
        model: &str,
        // Recognizer bias: the resolved term snapshot (see
        // `dictation_context`). WhisperKit tokenizes these into
        // `DecodingOptions.promptTokens`; engines without a text-prompt seam
        // (Parakeet) ignore them. Empty = no bias. The caller has already
        // gated by engine capability and capped the list.
        bias_terms: &[String],
    ) -> Result<(), String> {
        // Clear any orphan routing/active capture from a crashed flow.
        *ROUTING.lock().unwrap_or_else(|e| e.into_inner()) = None;
        drop(ACTIVE_CAPTURE.lock().unwrap_or_else(|e| e.into_inner()).take());

        // Reuse (or cold-spawn + prewarm) the resident process for this model.
        let stdin = ensure_engine(models_dir, model).await?;

        let (started_tx, started_rx) = tokio::sync::oneshot::channel::<Result<(), String>>();
        let (final_tx, final_rx) = tokio::sync::oneshot::channel::<String>();
        *ROUTING.lock().unwrap_or_else(|e| e.into_inner()) = Some(Routing {
            app,
            started_tx: Some(started_tx),
            final_tx: Some(final_tx),
        });
        *ACTIVE_CAPTURE.lock().unwrap_or_else(|e| e.into_inner()) = Some(ActiveCapture {
            stdin: stdin.clone(),
            final_rx,
        });

        write_line(
            &stdin,
            serde_json::json!({
                "op": "start-capture",
                "modelsDir": models_dir,
                "model": model,
                "biasTerms": bias_terms,
            }),
        )
        .await?;

        let started = tokio::time::timeout(START_TIMEOUT, started_rx)
            .await
            .map_err(|_| "capture start timed out".to_string())?
            .map_err(|_| "engine exited during capture start".to_string())?;
        if started.is_err() {
            // Failed start: clear the routing/active slots (the engine process
            // stays resident for the next attempt).
            *ROUTING.lock().unwrap_or_else(|e| e.into_inner()) = None;
            drop(ACTIVE_CAPTURE.lock().unwrap_or_else(|e| e.into_inner()).take());
        }
        started
    }

    /// Stop the capture and return the final transcript. The resident engine
    /// process is LEFT ALIVE (model stays loaded for the next capture); only
    /// the per-capture routing is torn down.
    pub async fn stop_capture() -> Result<String, String> {
        let active = ACTIVE_CAPTURE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .ok_or("no capture session is active")?;

        write_line(&active.stdin, serde_json::json!({ "op": "stop-capture" })).await?;

        let text = tokio::time::timeout(FINAL_TIMEOUT, active.final_rx)
            .await
            .map_err(|_| "transcription timed out".to_string())?
            .map_err(|_| "engine exited before finishing".to_string());
        *ROUTING.lock().unwrap_or_else(|e| e.into_inner()) = None;
        text
    }

    /// Tear down the capture without a transcript (the words are discarded).
    /// The resident engine process stays alive.
    pub async fn cancel_capture() {
        let active = ACTIVE_CAPTURE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(active) = active {
            let _ = write_line(&active.stdin, serde_json::json!({ "op": "cancel-capture" })).await;
            // Give the sidecar a beat to emit `ended` (which the frontend hears
            // as dictation://ended) before we drop the routing.
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        *ROUTING.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// Synchronous teardown for app shutdown. Drops the resident engine process
    /// so `kill_on_drop` reaps the `portbay-stt` sidecar and releases the
    /// microphone immediately. `AppState::shutdown_all` runs on the sync quit
    /// path (⌘Q / tray Quit / `RunEvent::Exit`) where the async
    /// `cancel_capture` can't be awaited — without this the sidecar keeps the
    /// mic (and its TCC grant), and the resident model in RAM, until the OS
    /// reaps the orphan.
    pub fn shutdown_capture() {
        drop(ACTIVE_CAPTURE.lock().unwrap_or_else(|e| e.into_inner()).take());
        *ROUTING.lock().unwrap_or_else(|e| e.into_inner()) = None;
        drop(ENGINE.lock().unwrap_or_else(|e| e.into_inner()).take());
    }

    /// Fire-and-forget model prewarm: spin up (or reuse) the resident engine
    /// process with the model loaded, so the next capture starts instant. Now
    /// that the process is kept alive, prewarm's load persists in RAM rather
    /// than only warming OS caches. Failures are logged-by-absence.
    pub async fn prewarm(models_dir: &str, model: &str) {
        // One prewarm at a time: a second concurrent call just races another
        // multi-GB CoreML load to no benefit (ensure_engine also serializes).
        if PREWARMING.swap(true, Ordering::SeqCst) {
            return;
        }
        let _ = ensure_engine(models_dir, model).await;
        PREWARMING.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_serializes_camel_case_and_skips_empty() {
        let status = SttStatus::unavailable("sidecar_missing");
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["available"], false);
        assert_eq!(json["reason"], "sidecar_missing");
        // Empty engines list is skipped, not serialized as [].
        assert!(json.get("engines").is_none());
    }

    #[test]
    fn check_output_parses_with_and_without_optionals() {
        let full: CheckOutput =
            serde_json::from_str(r#"{"available":true,"engines":["whisper","parakeet"]}"#).unwrap();
        assert!(full.available);
        assert_eq!(full.engines, vec!["whisper", "parakeet"]);

        let minimal: CheckOutput =
            serde_json::from_str(r#"{"available":false,"reason":"requires_macos_14"}"#).unwrap();
        assert!(!minimal.available);
        assert_eq!(minimal.reason.as_deref(), Some("requires_macos_14"));
        assert!(minimal.engines.is_empty());
    }

    // Drives the real `run_download` loop against a stand-in sidecar (injected
    // via PORTBAY_STT_BIN) that speaks the JSON protocol — exercising the
    // progress callback, terminal-frame parsing, and outcome mapping without a
    // built WhisperKit binary. macOS-only because the download client is.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn run_download_streams_progress_and_terminal_via_fake_sidecar() {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;

        let script = "#!/bin/bash\nread -r _req\n\
printf '{\"event\":\"progress\",\"fraction\":0.5,\"phase\":\"downloading\"}\\n'\n\
printf '{\"op\":\"download\",\"ok\":true,\"code\":0}\\n'\n";
        let path = std::env::temp_dir().join(format!("portbay-fake-stt-{}.sh", std::process::id()));
        {
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(script.as_bytes()).unwrap();
            let mut perms = f.metadata().unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&path, perms).unwrap();
        }
        // Only this test touches PORTBAY_STT_BIN, so the process-global set is
        // safe against the other (sidecar-free) STT tests.
        std::env::set_var("PORTBAY_STT_BIN", &path);

        let seen: std::sync::Arc<std::sync::Mutex<Vec<(f64, String)>>> = Default::default();
        let sink = std::sync::Arc::clone(&seen);
        let outcome = crate::stt::run_download("/tmp/models", "tiny", "dl-1", move |frac, phase| {
            sink.lock().unwrap().push((frac, phase));
        })
        .await;

        std::env::remove_var("PORTBAY_STT_BIN");
        let _ = std::fs::remove_file(&path);

        let outcome = outcome.expect("fake download should complete");
        assert!(outcome.success);
        assert!(!outcome.cancelled);
        assert!(outcome.error.is_none());
        let seen = seen.lock().unwrap();
        assert!(
            seen.iter().any(|(f, p)| (*f - 0.5).abs() < 1e-9 && p == "downloading"),
            "expected the progress event, saw {seen:?}"
        );
    }
}
