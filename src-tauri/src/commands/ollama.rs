//! Local Ollama manager IPC.

use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sysinfo::Disks;
use tauri::ipc::Channel;
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::ollama::{
    expand_tilde, is_ollama_pid, kill_pid, managed_record_pid, pid_alive, remove_managed_record,
    write_managed_record,
};
use crate::preferences::AiPrefs;
use crate::state::AppState;

static CANCELLED_PULLS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

fn cancelled_pulls() -> &'static Mutex<HashSet<String>> {
    CANCELLED_PULLS.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Run ids the user asked to stop mid-generation (the test-prompt double-Esc).
/// The streaming loop checks this between chunks and drops the connection,
/// which stops Ollama generating — the honest counterpart to the UI "Stopped".
static CANCELLED_GENERATES: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

fn cancelled_generates() -> &'static Mutex<HashSet<String>> {
    CANCELLED_GENERATES.get_or_init(|| Mutex::new(HashSet::new()))
}

fn is_generate_cancelled(id: &str) -> bool {
    cancelled_generates()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains(id)
}

/// The current pull's latest event, keyed by pull id. The page is the only
/// writer of new pulls (one at a time), but it can unmount mid-download —
/// the overview carries this so a fresh mount re-adopts the live state.
/// Terminal events stick until `ollama_dismiss_pull` or the next pull, so an
/// error that happened while the user was elsewhere is still shown.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivePull {
    pub pull_id: String,
    pub model: String,
    pub event: PullEvent,
}

static ACTIVE_PULL: OnceLock<Mutex<Option<ActivePull>>> = OnceLock::new();

fn active_pull() -> &'static Mutex<Option<ActivePull>> {
    ACTIVE_PULL.get_or_init(|| Mutex::new(None))
}

/// Store the latest event for `pull_id` and forward it to the channel.
/// Terminal events are sticky: once `done` is recorded, later non-terminal
/// events for the same pull are dropped (covers the cancel race, where the
/// stream keeps producing progress until it notices the flag).
fn publish_pull_event(on_event: &Channel<PullEvent>, pull_id: &str, model: &str, event: PullEvent) {
    {
        let mut slot = active_pull().lock().unwrap_or_else(|e| e.into_inner());
        let sticky_terminal = slot
            .as_ref()
            .is_some_and(|p| p.pull_id == pull_id && p.event.done && !event.done);
        if !sticky_terminal {
            *slot = Some(ActivePull {
                pull_id: pull_id.to_string(),
                model: model.to_string(),
                event: event.clone(),
            });
        }
    }
    let _ = on_event.send(event);
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OllamaOverview {
    pub config: AiPrefs,
    pub status: OllamaStatus,
    pub binary: OllamaBinaryStatus,
    pub installed_models: Vec<OllamaModel>,
    pub loaded_models: Vec<OllamaLoadedModel>,
    pub models_disk: DiskUsage,
    pub log_path: String,
    pub starter_models: Vec<StarterModel>,
    /// The in-flight (or last terminal, until dismissed) model pull — lives
    /// here so the page re-attaches after navigating away and back instead
    /// of losing the download state.
    pub active_pull: Option<ActivePull>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OllamaRunState {
    Stopped,
    Starting,
    RunningManaged,
    RunningExternal,
    UnreachableManaged,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OllamaStatus {
    pub state: OllamaRunState,
    pub endpoint: String,
    pub version: Option<String>,
    pub pid: Option<u32>,
    pub external: bool,
    pub detail: Option<String>,
    pub port_conflict: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OllamaBinaryStatus {
    pub path: Option<String>,
    pub version: Option<String>,
    pub detected: bool,
    pub install_hint: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskUsage {
    pub path: String,
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub available_bytes: u64,
    pub volume: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OllamaModel {
    pub name: String,
    pub size: u64,
    pub modified_at: Option<String>,
    pub family: Option<String>,
    pub parameter_size: Option<String>,
    pub quantization_level: Option<String>,
    /// Manifest digest from `/api/tags` (sha256 hex). Its 12-char prefix is
    /// what ollama.com prints next to each tag, so comparing the two answers
    /// "is an update available?" without pulling anything.
    #[serde(default)]
    pub digest: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OllamaLoadedModel {
    pub name: String,
    pub size: u64,
    pub size_vram: u64,
    pub expires_at: Option<String>,
    pub processor: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StarterModel {
    pub name: &'static str,
    pub label: &'static str,
    pub fit: &'static str,
    pub size_hint: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullEvent {
    pub status: String,
    pub digest: Option<String>,
    pub total: Option<u64>,
    pub completed: Option<u64>,
    pub error: Option<String>,
    pub done: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SmokeTestResult {
    pub response: String,
    pub model: String,
    pub total_duration_ms: Option<u64>,
}

/// One or more embedding vectors from `/api/embed` (the Embeddings playground
/// embeds 1–2 inputs and computes dimension + cosine similarity client-side).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbedResult {
    pub model: String,
    pub embeddings: Vec<Vec<f32>>,
}

#[tauri::command]
pub async fn ollama_overview(state: State<'_, AppState>) -> AppResult<OllamaOverview> {
    overview(&state).await
}

/// Cheap "is any Ollama serving the configured endpoint?" probe for the
/// global Stop-All signal. Mirrors exactly the three cases `stop_any_server`
/// shuts down — the managed child, a recorded managed orphan, or an external
/// server answering the endpoint — so the Stop-All button's enabled state
/// agrees with what Stop All actually stops. Deliberately skips the disk scan
/// and model listing that `ollama_overview` does, so it's safe to poll
/// app-wide every few seconds.
#[tauri::command]
pub async fn ollama_running(state: State<'_, AppState>) -> AppResult<bool> {
    let prefs = state.preferences_snapshot().normalise_ai_endpoint().ai;
    let managed = {
        let mut manager = state.ollama.lock().unwrap_or_else(|e| e.into_inner());
        manager.pid().is_some()
    } || managed_record_pid(&state.logs_dir, &prefs.endpoint).is_some();
    if managed {
        return Ok(true);
    }
    Ok(endpoint_version(&prefs.endpoint).await.is_some())
}

/// Cheap loaded-model probe for at-a-glance surfaces (the dashboard's Local AI
/// card). Hits `/api/ps` only — none of the disk scan or installed-model
/// listing `ollama_overview` does — so it's safe to poll app-wide alongside
/// `ollama_running`. A non-answering endpoint yields an empty list rather than
/// an error so the caller can poll it without special-casing a stopped server.
#[tauri::command]
pub async fn ollama_loaded_models(state: State<'_, AppState>) -> AppResult<Vec<OllamaLoadedModel>> {
    let prefs = state.preferences_snapshot().normalise_ai_endpoint().ai;
    Ok(loaded_models(&prefs.endpoint).await.unwrap_or_default())
}

#[tauri::command]
pub async fn ollama_start(state: State<'_, AppState>) -> AppResult<OllamaOverview> {
    let prefs = state.preferences_snapshot().normalise_ai_endpoint().ai;
    let managed = state
        .ollama
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_managed_running()
        || managed_record_pid(&state.logs_dir, &prefs.endpoint).is_some();
    // Resolve BEFORE any takeover stop: a binary discoverable only through
    // the running external process must be captured while that process is
    // still alive.
    let binary = resolve_binary(&prefs)
        .ok_or_else(|| AppError::BadInput("Ollama binary was not found.".into()))?;
    if endpoint_version(&prefs.endpoint).await.is_some() && !managed {
        // An external server answers at the endpoint — take it over. PortBay
        // owns the local AI lifecycle the same way it owns its language
        // runtimes: the external server is stopped (verified-`ollama`
        // processes only) and replaced with a managed one.
        stop_external_server(&prefs.endpoint).await?;
    }
    let log_path = state.logs_dir.join("ollama.log");
    let mut child = spawn_ollama(binary, prefs.clone(), log_path).await?;
    if let Err(e) = write_managed_record(&state.logs_dir, child.id(), &prefs.endpoint) {
        // We can't record ownership of the server we just spawned — kill it
        // rather than leak an untracked orphan that no later stop can find.
        let _ = child.kill();
        let _ = child.wait();
        return Err(AppError::Io(e));
    }
    {
        let mut manager = state.ollama.lock().unwrap_or_else(|e| e.into_inner());
        manager.set_child(child);
    }
    tokio::time::sleep(Duration::from_millis(350)).await;
    overview(&state).await
}

#[tauri::command]
pub async fn ollama_stop(state: State<'_, AppState>) -> AppResult<OllamaOverview> {
    stop_any_server(&state).await?;
    overview(&state).await
}

#[tauri::command]
pub async fn ollama_restart(state: State<'_, AppState>) -> AppResult<OllamaOverview> {
    stop_any_server(&state).await?;
    ollama_start(state).await
}

/// Stop whatever Ollama serves the configured endpoint: the managed child,
/// the recorded managed orphan, or an external server (which converts the
/// next Start into a clean managed takeover). Returns whether anything was
/// actually stopped. Shared by Stop, Restart, and Stop All.
pub async fn stop_any_server(state: &AppState) -> AppResult<bool> {
    let prefs = state.preferences_snapshot().normalise_ai_endpoint().ai;
    let child = {
        let mut manager = state.ollama.lock().unwrap_or_else(|e| e.into_inner());
        manager.take_child()
    };
    let stopped = if let Some(mut child) = child {
        tauri::async_runtime::spawn_blocking(move || {
            child.kill()?;
            let _ = child.wait();
            Ok::<(), std::io::Error>(())
        })
        .await
        .map_err(|e| AppError::Internal(format!("failed to join Ollama stop task: {e}")))?
        .map_err(AppError::Io)?;
        true
    } else if let Some(pid) = managed_record_pid(&state.logs_dir, &prefs.endpoint) {
        stop_managed_pid(pid).await?;
        true
    } else if endpoint_version(&prefs.endpoint).await.is_some() {
        stop_external_server(&prefs.endpoint).await?;
        true
    } else {
        false
    };
    let _ = remove_managed_record(&state.logs_dir);
    Ok(stopped)
}

/// Stop an Ollama server PortBay did not start. Local endpoints only, and
/// only processes whose executable really is `ollama` are signalled —
/// anything else squatting the port is reported, never killed. Retries once
/// because the Ollama.app menu-bar agent respawns its serve child; if it
/// keeps coming back, the error says to quit Ollama.app.
async fn stop_external_server(endpoint: &str) -> AppResult<()> {
    let url = url::Url::parse(endpoint)
        .map_err(|e| AppError::BadInput(format!("invalid Ollama endpoint: {e}")))?;
    let host = url.host_str().unwrap_or("127.0.0.1");
    if !matches!(host, "127.0.0.1" | "localhost" | "::1" | "0.0.0.0") {
        return Err(AppError::BadInput(format!(
            "The configured endpoint points at {host}; PortBay can only manage an Ollama server on this machine."
        )));
    }
    let port = url.port_or_known_default().unwrap_or(11434);
    tauri::async_runtime::spawn_blocking(move || {
        for _attempt in 0..2 {
            let pids = listener_pids(port);
            if pids.is_empty() {
                return Ok(());
            }
            let (ollama, foreign): (Vec<u32>, Vec<u32>) =
                pids.into_iter().partition(|pid| is_ollama_pid(*pid));
            if ollama.is_empty() {
                return Err(AppError::BadInput(format!(
                    "Port {port} is held by a non-Ollama process (pid {:?}); PortBay only stops Ollama servers. Free the port or change the endpoint.",
                    foreign
                )));
            }
            for pid in &ollama {
                let _ = kill_pid(*pid);
            }
            // Give it up to 4s to wind down and release the port.
            for _ in 0..40 {
                if ollama.iter().all(|pid| !pid_alive(*pid)) && listener_pids(port).is_empty() {
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            // Still occupied — likely a supervisor respawned it; loop kills
            // the fresh pid once more.
        }
        Err(AppError::Internal(
            "An external Ollama keeps restarting on this endpoint — most likely the Ollama menu-bar app. Quit Ollama.app, then press Start again.".into(),
        ))
    })
    .await
    .map_err(|e| AppError::Internal(format!("failed to join Ollama takeover task: {e}")))?
}

/// Pids LISTENing on a local TCP port (`lsof -t`). Empty on any failure —
/// callers treat that as "port free" and the spawn surfaces a bind error if
/// it wasn't.
fn listener_pids(port: u16) -> Vec<u32> {
    // NB: `-iTCP:<port>` must be one argument — split, lsof reads the port as
    // a file name and errors out.
    Command::new("lsof")
        .args(["-nP", "-t", &format!("-iTCP:{port}"), "-sTCP:LISTEN"])
        .output()
        .ok()
        .map(|out| {
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter_map(|line| line.trim().parse::<u32>().ok())
                .collect()
        })
        .unwrap_or_default()
}

#[tauri::command]
pub async fn ollama_show_model(state: State<'_, AppState>, model: String) -> AppResult<Value> {
    let endpoint = state
        .preferences_snapshot()
        .normalise_ai_endpoint()
        .ai
        .endpoint;
    // `long_client` (connect timeout, no total cap): `/api/show` normally
    // answers in milliseconds but legitimately blocks while a large model is
    // mid-load — a fixed total timeout would hard-fail that instead of waiting.
    let client = long_client()?;
    let res = client
        .post(format!("{}/api/show", endpoint.trim_end_matches('/')))
        .json(&json!({ "model": model }))
        .send()
        .await
        .map_err(http_err)?;
    json_response(res).await
}

#[tauri::command]
pub async fn ollama_delete_model(state: State<'_, AppState>, model: String) -> AppResult<()> {
    let endpoint = state
        .preferences_snapshot()
        .normalise_ai_endpoint()
        .ai
        .endpoint;
    let client = http_client()?;
    let res = client
        .delete(format!("{}/api/delete", endpoint.trim_end_matches('/')))
        .json(&json!({ "model": model }))
        .send()
        .await
        .map_err(http_err)?;
    ensure_ok(res).await
}

#[tauri::command]
pub async fn ollama_unload_model(state: State<'_, AppState>, model: String) -> AppResult<()> {
    let endpoint = state
        .preferences_snapshot()
        .normalise_ai_endpoint()
        .ai
        .endpoint;
    let client = long_client()?;
    let res = client
        .post(format!("{}/api/generate", endpoint.trim_end_matches('/')))
        .json(&json!({ "model": model, "prompt": "", "keep_alive": 0, "stream": false }))
        .send()
        .await
        .map_err(http_err)?;
    ensure_ok(res).await
}

#[tauri::command]
pub async fn ollama_smoke_test(
    state: State<'_, AppState>,
    model: String,
    prompt: String,
) -> AppResult<SmokeTestResult> {
    if model.trim().is_empty() {
        return Err(AppError::BadInput(
            "Choose a model before running a test prompt.".into(),
        ));
    }
    let endpoint = state
        .preferences_snapshot()
        .normalise_ai_endpoint()
        .ai
        .endpoint;
    let client = long_client()?;
    let res = client
        .post(format!("{}/api/generate", endpoint.trim_end_matches('/')))
        .json(&json!({ "model": model, "prompt": prompt, "stream": false }))
        .send()
        .await
        .map_err(http_err)?;
    let value = json_response(res).await?;
    Ok(SmokeTestResult {
        model: value
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        response: value
            .get("response")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        total_duration_ms: value
            .get("total_duration")
            .and_then(Value::as_u64)
            .map(|nanos| nanos / 1_000_000),
    })
}

/// Embed 1–2 inputs with an installed embedding model (`/api/embed`). The
/// Embeddings playground uses the returned vectors to show dimension, a value
/// sparkline, and cosine similarity between two inputs.
#[tauri::command]
pub async fn ollama_embed(
    state: State<'_, AppState>,
    model: String,
    input: Vec<String>,
) -> AppResult<EmbedResult> {
    if model.trim().is_empty() {
        return Err(AppError::BadInput(
            "Choose an embedding model first.".into(),
        ));
    }
    let inputs: Vec<String> = input.into_iter().filter(|s| !s.trim().is_empty()).collect();
    if inputs.is_empty() {
        return Err(AppError::BadInput("Enter text to embed.".into()));
    }
    let endpoint = state
        .preferences_snapshot()
        .normalise_ai_endpoint()
        .ai
        .endpoint;
    let client = long_client()?;
    let res = client
        .post(format!("{}/api/embed", endpoint.trim_end_matches('/')))
        .json(&json!({ "model": model, "input": inputs }))
        .send()
        .await
        .map_err(http_err)?;
    let value = json_response(res).await?;
    let embeddings = value
        .get("embeddings")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .map(|row| {
                    row.as_array()
                        .map(|v| {
                            v.iter()
                                .filter_map(|n| n.as_f64().map(|f| f as f32))
                                .collect()
                        })
                        .unwrap_or_default()
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(EmbedResult {
        model: value
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or(&model)
            .to_string(),
        embeddings,
    })
}

/// Streamed token/metric events for the Test prompt page. Tagged by `kind`
/// (same wire shape as the install/pull channels) so the frontend can run a
/// waiting → streaming → done state machine instead of blocking on one big
/// response. Every terminal path emits exactly one `done` or `error`, so the
/// card can never freeze mid-generation.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum GenerateEvent {
    /// One streamed response fragment (Ollama emits a token or few per line).
    #[serde(rename_all = "camelCase")]
    Token { text: String },
    /// One streamed reasoning fragment — only when `think` is on and the model
    /// supports it. Kept separate from `Token` so the page can render the
    /// chain-of-thought apart from the answer.
    #[serde(rename_all = "camelCase")]
    Thinking { text: String },
    /// Final frame: Ollama's timing/eval counters (nanoseconds → ms here).
    #[serde(rename_all = "camelCase")]
    Done {
        model: String,
        total_duration_ms: Option<u64>,
        load_duration_ms: Option<u64>,
        eval_count: Option<u64>,
        eval_duration_ms: Option<u64>,
        prompt_eval_count: Option<u64>,
        prompt_eval_duration_ms: Option<u64>,
    },
    /// Terminal failure (bad model, server down, HTTP error, dropped stream).
    #[serde(rename_all = "camelCase")]
    Error { message: String },
    /// User stopped the run (double-Esc) — partial output is kept on the page.
    Stopped,
}

/// No-data window after which a streaming generate is declared dead — a model
/// loading into VRAM can be quiet for a while, so this matches the pull stall.
const GENERATE_STALL_TIMEOUT: Duration = Duration::from_secs(120);

/// Ceiling on the unparsed line buffer for the NDJSON streams (generate/pull).
/// Each stream is newline-delimited JSON; a single line is small. A stream that
/// never emits a newline would otherwise grow the buffer without bound and OOM
/// the process — bail instead once a partial line passes this size.
const MAX_PENDING_BYTES: usize = 1024 * 1024;

/// Streaming sibling of `ollama_smoke_test`: posts `stream: true` to
/// `/api/generate` and forwards each token frame as it arrives. Errors are
/// reported through the channel (terminal `error` event), never the Result —
/// the card renders them inline, the same contract as the pull command.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn ollama_test_stream(
    state: State<'_, AppState>,
    model: String,
    prompt: String,
    run_id: String,
    on_event: Channel<GenerateEvent>,
    // Optional system instruction (sent as `system`); empty/absent is omitted.
    system: Option<String>,
    // Request chain-of-thought from a reasoning model (`think: true`).
    think: Option<bool>,
    // Pass-through Ollama sampling `options` (temperature, top_p, num_ctx, …).
    options: Option<Value>,
) -> AppResult<()> {
    if model.trim().is_empty() {
        let _ = on_event.send(GenerateEvent::Error {
            message: "Choose a model before running a test prompt.".into(),
        });
        return Ok(());
    }
    // Clear any stale cancel flag from a previous run reusing this id slot.
    cancelled_generates()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&run_id);
    let result = run_generate(
        &state,
        &model,
        &prompt,
        &run_id,
        &on_event,
        system.as_deref(),
        think.unwrap_or(false),
        options,
    )
    .await;
    // Stop request observed mid-stream → the loop returned Ok after emitting
    // Stopped; an Err is a genuine failure. Either way, drop the flag.
    cancelled_generates()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&run_id);
    if let Err(err) = result {
        let _ = on_event.send(GenerateEvent::Error {
            message: err.to_string(),
        });
    }
    Ok(())
}

/// Stop an in-flight test generation (the page's double-Esc). The streaming
/// loop drops the connection on its next chunk, ending Ollama's generation.
#[tauri::command]
pub async fn ollama_cancel_generate(run_id: String) -> AppResult<()> {
    cancelled_generates()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(run_id);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run_generate(
    state: &State<'_, AppState>,
    model: &str,
    prompt: &str,
    run_id: &str,
    on_event: &Channel<GenerateEvent>,
    system: Option<&str>,
    think: bool,
    options: Option<Value>,
) -> AppResult<()> {
    let endpoint = state
        .preferences_snapshot()
        .normalise_ai_endpoint()
        .ai
        .endpoint;
    let client = long_client()?;
    // Build the request incrementally so unset knobs fall back to Ollama's own
    // defaults rather than being pinned to zeros.
    let mut body = json!({ "model": model, "prompt": prompt, "stream": true });
    if let Some(sys) = system {
        if !sys.trim().is_empty() {
            body["system"] = json!(sys);
        }
    }
    if think {
        body["think"] = json!(true);
    }
    if let Some(opts) = options {
        if opts.as_object().is_some_and(|o| !o.is_empty()) {
            body["options"] = opts;
        }
    }
    let mut res = client
        .post(format!("{}/api/generate", endpoint.trim_end_matches('/')))
        .json(&body)
        .send()
        .await
        .map_err(http_err)?;
    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        let detail = serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_string))
            .unwrap_or(body);
        return Err(AppError::Internal(if detail.trim().is_empty() {
            format!("ollama returned HTTP {status}")
        } else {
            format!("ollama returned HTTP {status}: {}", detail.trim())
        }));
    }

    let ms = |v: &Value, key: &str| v.get(key).and_then(Value::as_u64).map(|n| n / 1_000_000);
    let count = |v: &Value, key: &str| v.get(key).and_then(Value::as_u64);
    let mut pending = String::new();
    loop {
        // Stopped by the user (double-Esc): drop the connection — that ends
        // Ollama's generation — and report the partial result as stopped.
        if is_generate_cancelled(run_id) {
            let _ = on_event.send(GenerateEvent::Stopped);
            return Ok(());
        }
        let chunk = match tokio::time::timeout(GENERATE_STALL_TIMEOUT, res.chunk()).await {
            Err(_) => {
                return Err(AppError::Internal(format!(
                    "no response for {}s — the model stalled. Check the server, then try again.",
                    GENERATE_STALL_TIMEOUT.as_secs()
                )));
            }
            Ok(r) => {
                r.map_err(|e| AppError::Internal(format!("response stream interrupted: {e}")))?
            }
        };
        let Some(chunk) = chunk else { break };
        if is_generate_cancelled(run_id) {
            let _ = on_event.send(GenerateEvent::Stopped);
            return Ok(());
        }
        pending.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(pos) = pending.find('\n') {
            let line = pending[..pos].trim().to_string();
            pending = pending[pos + 1..].to_string();
            if line.is_empty() {
                continue;
            }
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if let Some(error) = value.get("error").and_then(Value::as_str) {
                return Err(AppError::Internal(error.to_string()));
            }
            // Reasoning fragment (only present with `think: true` on a model
            // that supports it) — forwarded separately from the answer.
            if let Some(thinking) = value.get("thinking").and_then(Value::as_str) {
                if !thinking.is_empty() {
                    let _ = on_event.send(GenerateEvent::Thinking {
                        text: thinking.to_string(),
                    });
                }
            }
            if let Some(text) = value.get("response").and_then(Value::as_str) {
                if !text.is_empty() {
                    let _ = on_event.send(GenerateEvent::Token {
                        text: text.to_string(),
                    });
                }
            }
            if value.get("done").and_then(Value::as_bool) == Some(true) {
                let _ = on_event.send(GenerateEvent::Done {
                    model: value
                        .get("model")
                        .and_then(Value::as_str)
                        .unwrap_or(model)
                        .to_string(),
                    total_duration_ms: ms(&value, "total_duration"),
                    load_duration_ms: ms(&value, "load_duration"),
                    eval_count: count(&value, "eval_count"),
                    eval_duration_ms: ms(&value, "eval_duration"),
                    prompt_eval_count: count(&value, "prompt_eval_count"),
                    prompt_eval_duration_ms: ms(&value, "prompt_eval_duration"),
                });
                return Ok(());
            }
        }
        // After draining complete lines, `pending` holds only the trailing
        // partial. If that alone exceeds the cap, the stream isn't emitting
        // newlines — refuse to keep buffering rather than grow until OOM.
        if pending.len() > MAX_PENDING_BYTES {
            return Err(AppError::Internal(format!(
                "response stream exceeded {} MB without a line break — aborting.",
                MAX_PENDING_BYTES / (1024 * 1024)
            )));
        }
    }
    // Stream ended without a `done` frame — report a clean finish anyway so the
    // card leaves its "streaming" state instead of hanging.
    let _ = on_event.send(GenerateEvent::Done {
        model: model.to_string(),
        total_duration_ms: None,
        load_duration_ms: None,
        eval_count: None,
        eval_duration_ms: None,
        prompt_eval_count: None,
        prompt_eval_duration_ms: None,
    });
    Ok(())
}

/// No-data window after which a pull is declared stalled. Generous because
/// the stream goes quiet while Ollama verifies a multi-GB layer's digest.
const PULL_STALL_TIMEOUT: Duration = Duration::from_secs(120);

#[tauri::command]
pub async fn ollama_pull_model(
    state: State<'_, AppState>,
    model: String,
    pull_id: String,
    on_event: Channel<PullEvent>,
) -> AppResult<()> {
    if model.trim().is_empty() {
        return Err(AppError::BadInput("Enter a model name to pull.".into()));
    }
    {
        let mut cancelled = cancelled_pulls().lock().unwrap_or_else(|e| e.into_inner());
        cancelled.remove(&pull_id);
    }
    let endpoint = state
        .preferences_snapshot()
        .normalise_ai_endpoint()
        .ai
        .endpoint;
    // Register the pull up front so the overview reports it even before the
    // first stream event (and after the page unmounts mid-download).
    publish_pull_event(
        &on_event,
        &pull_id,
        &model,
        PullEvent {
            status: "queued".into(),
            digest: None,
            total: None,
            completed: None,
            error: None,
            done: false,
        },
    );
    // Snapshot free space on the models volume so the pull can bail out early
    // if a layer plainly won't fit, instead of running until Ollama errors and
    // leaving a partial install. Best-effort: a failed probe just skips it.
    let available_bytes = disk_usage(
        state
            .preferences_snapshot()
            .normalise_ai_endpoint()
            .ai
            .models_dir,
    )
    .await
    .ok()
    .map(|d| d.available_bytes);
    let result = run_pull(&endpoint, &model, &pull_id, &on_event, available_bytes).await;
    if let Err(e) = &result {
        // EVERY failure path lands the UI in a terminal error state — a pull
        // card frozen mid-progress with no error was the old behaviour when
        // only the command result (a toast) carried the failure.
        publish_pull_event(
            &on_event,
            &pull_id,
            &model,
            PullEvent {
                status: "error".into(),
                digest: None,
                total: None,
                completed: None,
                error: Some(e.to_string()),
                done: true,
            },
        );
    }
    result
}

async fn run_pull(
    endpoint: &str,
    model: &str,
    pull_id: &str,
    on_event: &Channel<PullEvent>,
    available_bytes: Option<u64>,
) -> AppResult<()> {
    let client = long_client()?;
    let mut res = client
        .post(format!("{}/api/pull", endpoint.trim_end_matches('/')))
        .json(&json!({ "name": model, "stream": true }))
        .send()
        .await
        .map_err(http_err)?;
    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        let detail = serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_string))
            .unwrap_or(body);
        return Err(AppError::Internal(if detail.trim().is_empty() {
            format!("ollama returned HTTP {status}")
        } else {
            format!("ollama returned HTTP {status}: {}", detail.trim())
        }));
    }

    let mut pending = String::new();
    loop {
        // Stall watchdog: Ollama streams status lines constantly while a pull
        // is healthy; a long silence means the connection or registry died
        // without an error frame. Pulls resume layer-by-layer, so the fix is
        // always "pull again".
        // Wait for the next chunk while staying responsive to cancellation.
        // Ollama goes silent while it verifies a multi-GB layer's digest, so a
        // single 120 s wait on `chunk()` would swallow a cancel for up to that
        // whole window. Poll the cancel flag on a short tick instead, while
        // still enforcing the overall stall budget against wall-clock elapsed.
        let chunk = {
            let started = tokio::time::Instant::now();
            let mut chunk_fut = std::pin::pin!(res.chunk());
            loop {
                if is_pull_cancelled(pull_id) {
                    publish_pull_event(
                        on_event,
                        pull_id,
                        model,
                        PullEvent {
                            status: "cancelled".into(),
                            digest: None,
                            total: None,
                            completed: None,
                            error: None,
                            done: true,
                        },
                    );
                    return Ok(());
                }
                let remaining = match PULL_STALL_TIMEOUT.checked_sub(started.elapsed()) {
                    Some(r) if !r.is_zero() => r,
                    _ => {
                        return Err(AppError::Internal(format!(
                            "no data received for {}s — the download stalled. Downloaded layers are kept; pull again to resume where it left off.",
                            PULL_STALL_TIMEOUT.as_secs()
                        )));
                    }
                };
                let tick = remaining.min(Duration::from_millis(250));
                match tokio::time::timeout(tick, &mut chunk_fut).await {
                    Ok(r) => {
                        break r.map_err(|e| {
                            AppError::Internal(format!(
                                "download interrupted: {e}. Downloaded layers are kept; pull again to resume where it left off."
                            ))
                        })?;
                    }
                    // Tick elapsed without a chunk — loop to re-check cancel and
                    // the stall budget, then keep waiting on the same future.
                    Err(_) => continue,
                }
            }
        };
        let Some(chunk) = chunk else {
            break;
        };
        pending.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(pos) = pending.find('\n') {
            let line = pending[..pos].trim().to_string();
            pending = pending[pos + 1..].to_string();
            if line.is_empty() {
                continue;
            }
            if let Ok(event) = parse_pull_event(&line) {
                // As soon as Ollama reports a layer's size, bail if a single
                // layer already exceeds the free space we snapshotted — the
                // pull can't possibly complete. Conservative (per-layer, not
                // cumulative) so it won't false-positive on a roomy volume.
                if layer_exceeds_free(available_bytes, event.total) {
                    return Err(AppError::Internal(format!(
                        "Not enough disk space: a layer of this model needs {:.1} GB but only {:.1} GB is free on the models volume. Free up space or change the models folder in Configuration.",
                        event.total.unwrap_or(0) as f64 / 1_000_000_000.0,
                        available_bytes.unwrap_or(0) as f64 / 1_000_000_000.0,
                    )));
                }
                let done = event.done;
                let error = event.error.clone();
                publish_pull_event(on_event, pull_id, model, event);
                if done {
                    // Surface stream-level errors (disk full, unknown model,
                    // registry auth, …) through the command result too.
                    if let Some(error) = error {
                        return Err(AppError::Internal(error));
                    }
                    return Ok(());
                }
            }
        }
        // Trailing partial only at this point — a stream with no newlines must
        // not grow the buffer until OOM. Bail past the cap.
        if pending.len() > MAX_PENDING_BYTES {
            return Err(AppError::Internal(format!(
                "pull stream exceeded {} MB without a line break — aborting.",
                MAX_PENDING_BYTES / (1024 * 1024)
            )));
        }
    }
    publish_pull_event(
        on_event,
        pull_id,
        model,
        PullEvent {
            status: "done".into(),
            digest: None,
            total: None,
            completed: None,
            error: None,
            done: true,
        },
    );
    Ok(())
}

#[tauri::command]
pub async fn ollama_cancel_pull(pull_id: String) -> AppResult<()> {
    cancelled_pulls()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(pull_id.clone());
    // Land the stored state in its terminal form immediately — the stream
    // only notices the flag on its next chunk, and the overview poll must
    // not re-adopt a "downloading" snapshot of a pull the user cancelled.
    let mut slot = active_pull().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(pull) = slot.as_mut() {
        if pull.pull_id == pull_id && !pull.event.done {
            pull.event = PullEvent {
                status: "cancelled".into(),
                digest: None,
                total: None,
                completed: None,
                error: None,
                done: true,
            };
        }
    }
    Ok(())
}

/// Clear the stored (terminal) pull state — the page's Dismiss button.
/// Only terminal states clear; an active pull is never dropped from the
/// overview by a stray dismiss.
#[tauri::command]
pub async fn ollama_dismiss_pull() -> AppResult<()> {
    let mut slot = active_pull().lock().unwrap_or_else(|e| e.into_inner());
    if slot.as_ref().is_some_and(|p| p.event.done) {
        *slot = None;
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OllamaUpdateCheck {
    /// Version of the PortBay-managed install, when one exists.
    pub installed_version: Option<String>,
    /// Newest version in the signed runtimes manifest for this arch.
    pub latest_version: Option<String>,
    pub update_available: bool,
}

/// Compare the managed Ollama install against the signed runtimes manifest.
/// Network call (manifest fetch) — invoked on demand from the UI, never on
/// the overview poll.
#[tauri::command]
pub async fn ollama_update_check() -> AppResult<OllamaUpdateCheck> {
    let installed_version = crate::ollama::managed_install_binary().and_then(|bin| {
        // …/runtimes/ollama/<version>/bin/ollama → <version>
        bin.parent()?
            .parent()?
            .file_name()?
            .to_str()
            .map(str::to_string)
    });
    let manifest = crate::commands::runtimes::fetch_signed_manifest().await?;
    let arch = crate::runtimes::download::manifest::current_arch();
    let latest_version = crate::commands::runtimes::newest_entry(&manifest, "ollama", arch)
        .map(|entry| entry.version);
    let update_available = match (&installed_version, &latest_version) {
        (Some(installed), Some(latest)) => version_newer(latest, installed),
        _ => false,
    };
    Ok(OllamaUpdateCheck {
        installed_version,
        latest_version,
        update_available,
    })
}

/// Numeric-aware "is `a` newer than `b`" for dotted versions — lexicographic
/// comparison calls 0.9.0 newer than 0.30.6.
fn version_newer(a: &str, b: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> {
        v.split('.')
            .map(|part| {
                part.chars()
                    .take_while(char::is_ascii_digit)
                    .collect::<String>()
                    .parse()
                    .unwrap_or(0)
            })
            .collect()
    };
    parse(a) > parse(b)
}

/// Download a PortBay-managed Ollama build from the signed runtimes manifest —
/// the same minisign-verified pipeline the language runtimes use. Installs to
/// `<data-dir>/PortBay/runtimes/ollama/<version>/bin/ollama`, where
/// `resolve_binary` prefers it over any system copy. Re-running it with the
/// same newest version is the repair path: the installer replaces the
/// existing dir atomically, so a half-broken install is re-downloaded whole.
/// Unlike languages it is deliberately NOT registered as a `ManagedRuntime`:
/// it isn't a project runtime and must never show up in language/version
/// pickers.
#[tauri::command]
pub async fn ollama_install(
    on_event: Channel<crate::commands::runtimes::InstallEvent>,
) -> AppResult<()> {
    use crate::commands::runtimes::InstallEvent;

    let result = run_ollama_install(&on_event).await;
    if let Err(e) = &result {
        // Terminal failure event so the inline status never freezes at a
        // stale percentage — same contract as the pull stream.
        let _ = on_event.send(InstallEvent::Log {
            line: format!("Install failed: {e}"),
        });
        let _ = on_event.send(InstallEvent::Done { success: false });
    }
    result
}

async fn run_ollama_install(
    on_event: &Channel<crate::commands::runtimes::InstallEvent>,
) -> AppResult<()> {
    use crate::commands::runtimes::InstallEvent;

    let _ = on_event.send(InstallEvent::Log {
        line: "Fetching signed PortBay runtime manifest…".into(),
    });
    let manifest = crate::commands::runtimes::fetch_signed_manifest().await?;
    let arch = crate::runtimes::download::manifest::current_arch();
    let entry = crate::commands::runtimes::newest_entry(&manifest, "ollama", arch).ok_or_else(
        || {
            AppError::BadInput(format!(
                "no PortBay-managed Ollama build is published for {arch} yet — install from https://ollama.com/download instead"
            ))
        },
    )?;
    let dest_root = crate::commands::runtimes::runtime_dest_root()?;
    let version = entry.version.clone();
    let _ = on_event.send(InstallEvent::Log {
        line: format!("Installing Ollama {version} ({arch})…"),
    });
    let channel_for_progress = on_event.clone();
    let binary = crate::runtimes::download::install::fetch_and_install(
        &entry,
        &dest_root,
        Path::new("bin/ollama"),
        move |downloaded, total| {
            let _ = channel_for_progress.send(InstallEvent::Progress { downloaded, total });
        },
        |bin| {
            Command::new(bin)
                .arg("--version")
                .output()
                .ok()
                .map(|out| {
                    let text = format!(
                        "{}{}",
                        String::from_utf8_lossy(&out.stdout),
                        String::from_utf8_lossy(&out.stderr)
                    );
                    text.to_lowercase().contains("version")
                })
                .unwrap_or(false)
        },
    )
    .await
    .map_err(|e| AppError::Internal(format!("Ollama install failed: {e}")))?;

    let install_dir = binary
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| AppError::Internal("installed Ollama path is malformed".into()))?;
    crate::commands::runtimes::strip_quarantine(install_dir)?;

    let _ = on_event.send(InstallEvent::Done { success: true });
    Ok(())
}

async fn overview(state: &AppState) -> AppResult<OllamaOverview> {
    let prefs = state.preferences_snapshot().normalise_ai_endpoint().ai;
    let log_path = state.logs_dir.join("ollama.log");
    let binary_path = resolve_binary(&prefs);
    let binary = binary_status(binary_path).await;
    let (managed, pid) = {
        let mut manager = state.ollama.lock().unwrap_or_else(|e| e.into_inner());
        let pid = manager.pid();
        let record_pid = managed_record_pid(&state.logs_dir, &prefs.endpoint);
        if pid.is_none() && record_pid.is_none() {
            let _ = remove_managed_record(&state.logs_dir);
        }
        (pid.is_some() || record_pid.is_some(), pid.or(record_pid))
    };
    let version = endpoint_version(&prefs.endpoint).await;
    let port_conflict = if version.is_none() {
        port_holder(&prefs.endpoint).await
    } else {
        None
    };
    let status = match (&version, managed) {
        (Some(v), true) => OllamaStatus {
            state: OllamaRunState::RunningManaged,
            endpoint: prefs.endpoint.clone(),
            version: Some(v.clone()),
            pid,
            external: false,
            detail: Some("PortBay-managed server".into()),
            port_conflict,
        },
        (Some(v), false) => OllamaStatus {
            state: OllamaRunState::RunningExternal,
            endpoint: prefs.endpoint.clone(),
            version: Some(v.clone()),
            pid: None,
            external: true,
            detail: Some(
                "Endpoint is alive, but PortBay did not start this process. Stop shuts it down; Start/Restart replace it with a managed server.".into(),
            ),
            port_conflict,
        },
        (None, true) => OllamaStatus {
            state: OllamaRunState::UnreachableManaged,
            endpoint: prefs.endpoint.clone(),
            version: None,
            pid,
            external: false,
            detail: Some("Managed process is running, but the API is not answering yet.".into()),
            port_conflict,
        },
        (None, false) => OllamaStatus {
            state: if pid.is_some() {
                OllamaRunState::Starting
            } else {
                OllamaRunState::Stopped
            },
            endpoint: prefs.endpoint.clone(),
            version: None,
            pid,
            external: false,
            detail: None,
            port_conflict,
        },
    };
    let (installed_models, loaded_models) = if version.is_some() {
        tokio::join!(
            installed_models(&prefs.endpoint),
            loaded_models(&prefs.endpoint)
        )
    } else {
        (Ok(Vec::new()), Ok(Vec::new()))
    };
    let disk = disk_usage(prefs.models_dir.clone()).await?;
    let active_pull = active_pull()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    Ok(OllamaOverview {
        active_pull,
        config: prefs,
        status,
        binary,
        installed_models: installed_models?,
        loaded_models: loaded_models?,
        models_disk: disk,
        log_path: log_path.to_string_lossy().into_owned(),
        starter_models: starter_models(),
    })
}

async fn spawn_ollama(
    binary: PathBuf,
    prefs: AiPrefs,
    log_path: PathBuf,
) -> AppResult<std::process::Child> {
    tauri::async_runtime::spawn_blocking(move || {
        if let Some(parent) = log_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let stdout = open_log(&log_path)?;
        let stderr = stdout.try_clone()?;
        let mut cmd = Command::new(binary);
        cmd.arg("serve")
            .env("OLLAMA_MODELS", expand_tilde(&prefs.models_dir))
            .env("OLLAMA_HOST", ollama_host(&prefs.endpoint))
            .env("OLLAMA_KEEP_ALIVE", prefs.keep_alive)
            .env("OLLAMA_FLASH_ATTENTION", bool_env(prefs.flash_attention))
            .env("OLLAMA_ORIGINS", prefs.origins)
            .env("OLLAMA_DEBUG", bool_env(prefs.debug))
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        if let Some(n) = prefs.num_parallel {
            cmd.env("OLLAMA_NUM_PARALLEL", n.to_string());
        }
        if prefs.no_history {
            cmd.env("OLLAMA_NOHISTORY", "1");
        }
        if prefs.no_prune {
            cmd.env("OLLAMA_NOPRUNE", "1");
        }
        if prefs.schedule_spread {
            cmd.env("OLLAMA_SCHED_SPREAD", "1");
        }
        if prefs.multi_user_cache {
            cmd.env("OLLAMA_MULTIUSER_CACHE", "1");
        }
        if !prefs.kv_cache_type.trim().is_empty() {
            cmd.env("OLLAMA_KV_CACHE_TYPE", prefs.kv_cache_type);
        }
        if let Some(bytes) = prefs.gpu_overhead {
            cmd.env("OLLAMA_GPU_OVERHEAD", bytes.to_string());
        }
        if !prefs.load_timeout.trim().is_empty() {
            cmd.env("OLLAMA_LOAD_TIMEOUT", prefs.load_timeout);
        }
        if let Some(n) = prefs.max_loaded_models {
            cmd.env("OLLAMA_MAX_LOADED_MODELS", n.to_string());
        }
        if let Some(n) = prefs.max_queue {
            cmd.env("OLLAMA_MAX_QUEUE", n.to_string());
        }
        if !prefs.llm_library.trim().is_empty() {
            cmd.env("OLLAMA_LLM_LIBRARY", prefs.llm_library);
        }
        if !prefs.http_proxy.trim().is_empty() {
            cmd.env("HTTP_PROXY", prefs.http_proxy.trim());
            cmd.env("http_proxy", prefs.http_proxy.trim());
        }
        if !prefs.https_proxy.trim().is_empty() {
            cmd.env("HTTPS_PROXY", prefs.https_proxy.trim());
            cmd.env("https_proxy", prefs.https_proxy.trim());
        }
        if !prefs.no_proxy.trim().is_empty() {
            cmd.env("NO_PROXY", prefs.no_proxy.trim());
            cmd.env("no_proxy", prefs.no_proxy.trim());
        }
        cmd.spawn()
    })
    .await
    .map_err(|e| AppError::Internal(format!("failed to join Ollama start task: {e}")))?
    .map_err(AppError::Io)
}

fn open_log(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)
}

async fn endpoint_version(endpoint: &str) -> Option<String> {
    let client = http_client().ok()?;
    let res = client
        .get(format!("{}/api/version", endpoint.trim_end_matches('/')))
        .send()
        .await
        .ok()?;
    if !res.status().is_success() {
        return None;
    }
    let value = res.json::<Value>().await.ok()?;
    value
        .get("version")
        .and_then(Value::as_str)
        .map(str::to_string)
}

async fn installed_models(endpoint: &str) -> AppResult<Vec<OllamaModel>> {
    let client = http_client()?;
    let res = client
        .get(format!("{}/api/tags", endpoint.trim_end_matches('/')))
        .send()
        .await
        .map_err(http_err)?;
    let value = json_response(res).await?;
    let models = value
        .get("models")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|m| OllamaModel {
            name: m
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            size: m.get("size").and_then(Value::as_u64).unwrap_or(0),
            modified_at: m
                .get("modified_at")
                .and_then(Value::as_str)
                .map(str::to_string),
            family: m
                .get("details")
                .and_then(|d| d.get("family"))
                .and_then(Value::as_str)
                .map(str::to_string),
            parameter_size: m
                .get("details")
                .and_then(|d| d.get("parameter_size"))
                .and_then(Value::as_str)
                .map(str::to_string),
            quantization_level: m
                .get("details")
                .and_then(|d| d.get("quantization_level"))
                .and_then(Value::as_str)
                .map(str::to_string),
            digest: m.get("digest").and_then(Value::as_str).map(str::to_string),
        })
        .filter(|m| !m.name.is_empty())
        .collect();
    Ok(models)
}

async fn loaded_models(endpoint: &str) -> AppResult<Vec<OllamaLoadedModel>> {
    let client = http_client()?;
    let res = client
        .get(format!("{}/api/ps", endpoint.trim_end_matches('/')))
        .send()
        .await
        .map_err(http_err)?;
    let value = json_response(res).await?;
    let models = value
        .get("models")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|m| OllamaLoadedModel {
            name: m
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            size: m.get("size").and_then(Value::as_u64).unwrap_or(0),
            size_vram: m.get("size_vram").and_then(Value::as_u64).unwrap_or(0),
            expires_at: m
                .get("expires_at")
                .and_then(Value::as_str)
                .map(str::to_string),
            processor: m
                .get("processor")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
        .filter(|m| !m.name.is_empty())
        .collect();
    Ok(models)
}

async fn disk_usage(configured_dir: String) -> AppResult<DiskUsage> {
    tauri::async_runtime::spawn_blocking(move || {
        // A running server's own OLLAMA_MODELS is where pulls actually land —
        // the preference only describes the NEXT managed start. Reporting the
        // configured dir while an external server stored everything on another
        // volume showed the wrong disk's free space entirely.
        let expanded = running_server_models_dir().unwrap_or_else(|| expand_tilde(&configured_dir));
        let used = dir_size(Path::new(&expanded));
        // Match mount points against the symlink-resolved path: a models dir
        // reached through a home-dir symlink onto another volume otherwise
        // string-matches "/" and reports the boot disk.
        let canonical = nearest_canonical(Path::new(&expanded));
        let disks = Disks::new_with_refreshed_list();
        let best = disks
            .iter()
            .filter(|d| canonical.starts_with(d.mount_point()))
            .max_by_key(|d| d.mount_point().as_os_str().len());
        let (total, available, volume) = best
            .map(|d| {
                (
                    d.total_space(),
                    d.available_space(),
                    Some(d.mount_point().to_string_lossy().into_owned()),
                )
            })
            .unwrap_or((0, 0, None));
        Ok(DiskUsage {
            path: expanded,
            total_bytes: total,
            used_bytes: used,
            available_bytes: available,
            volume,
        })
    })
    .await
    .map_err(|e| AppError::Internal(format!("failed to join disk scan: {e}")))?
}

/// The models directory the *running* Ollama server actually uses: its own
/// `OLLAMA_MODELS` env when set, else Ollama's stock default. `None` when no
/// server process is up (callers fall back to the configured dir) or its
/// environment can't be read (other-user process). Prefers the `serve`
/// process over `run`/runner siblings, which carry a client's env.
///
/// sysinfo only locates the pids here — on macOS it returns empty `cmd()`
/// and `environ()` for processes it didn't spawn, so the command+env line
/// comes from `ps -wwE` instead (same-user processes only, which is exactly
/// the scope that matters).
fn running_server_models_dir() -> Option<String> {
    let mut system = sysinfo::System::new();
    system.refresh_processes();
    let pids: Vec<u32> = system
        .processes()
        .values()
        .filter(|p| {
            p.exe()
                .and_then(|exe| exe.file_name())
                .and_then(|name| name.to_str())
                .map(|name| name == "ollama")
                .unwrap_or_else(|| p.name() == "ollama")
        })
        .map(|p| p.pid().as_u32())
        .collect();
    let mut serve_line: Option<String> = None;
    let mut any_line: Option<String> = None;
    for pid in pids {
        let Some(line) = process_command_with_env(pid) else {
            continue;
        };
        let command = &line[..env_block_start(&line)];
        if command.split_whitespace().any(|t| t == "serve") {
            serve_line = Some(line);
            break;
        }
        if any_line.is_none() {
            any_line = Some(line);
        }
    }
    let line = serve_line.or(any_line)?;
    let env_block = &line[env_block_start(&line)..];
    if env_block.trim().is_empty() {
        // Command visible but environment not — can't know better than the
        // configured dir.
        return None;
    }
    match ps_env_value(env_block, "OLLAMA_MODELS").filter(|v| !v.is_empty()) {
        Some(dir) => Some(expand_tilde(dir)),
        // Server up without the env: it stores under Ollama's own default.
        None => {
            dirs::home_dir().map(|home| home.join(".ollama/models").to_string_lossy().into_owned())
        }
    }
}

/// One process's command line with its environment appended, `ps -wwE`
/// style. macOS: literally that invocation. Elsewhere sysinfo can read both
/// directly, so they are joined into the same shape.
#[cfg(target_os = "macos")]
fn process_command_with_env(pid: u32) -> Option<String> {
    let out = Command::new("/bin/ps")
        .args(["-wwE", "-p", &pid.to_string(), "-o", "command="])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let line = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!line.is_empty()).then_some(line)
}

#[cfg(not(target_os = "macos"))]
fn process_command_with_env(pid: u32) -> Option<String> {
    let mut system = sysinfo::System::new();
    system.refresh_processes();
    let p = system.process(sysinfo::Pid::from_u32(pid))?;
    let line = format!("{} {}", p.cmd().join(" "), p.environ().join(" "));
    let line = line.trim().to_string();
    (!line.is_empty()).then_some(line)
}

/// Whether a whitespace-delimited token starts a `NAME=value` environment
/// assignment (the boundary between command and env in a `ps -wwE` line).
fn is_env_assignment(token: &str) -> bool {
    let Some((name, _)) = token.split_once('=') else {
        return false;
    };
    !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Byte offset where the env block starts in a `ps -wwE` command line: the
/// first token shaped like an assignment. `line.len()` when there is none.
fn env_block_start(line: &str) -> usize {
    let mut offset = 0;
    for token in line.split_whitespace() {
        let start = match line[offset..].find(token) {
            Some(i) => i + offset,
            None => break,
        };
        if is_env_assignment(token) {
            return start;
        }
        offset = start + token.len();
    }
    line.len()
}

/// `name`'s value inside a `ps -wwE` env block. Values are not quoted, so a
/// value containing spaces runs until the next `NAME=`-shaped token (or end
/// of line) — good enough for paths, which is all this reads.
fn ps_env_value<'a>(env_block: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!("{name}=");
    let mut offset = 0;
    let mut value_start: Option<usize> = None;
    for token in env_block.split_whitespace() {
        let pos = match env_block[offset..].find(token) {
            Some(i) => i + offset,
            None => break,
        };
        offset = pos + token.len();
        if let Some(start) = value_start {
            if is_env_assignment(token) {
                return Some(env_block[start..pos].trim());
            }
        } else if token.starts_with(&needle) && is_env_assignment(token) {
            value_start = Some(pos + needle.len());
        }
    }
    value_start.map(|start| env_block[start..].trim())
}

/// Canonicalize `path`, falling back to its nearest existing ancestor so a
/// not-yet-created models dir still resolves to the right volume.
/// `pub(crate)`: the STT models page reports disk usage the same way.
pub(crate) fn nearest_canonical(path: &Path) -> PathBuf {
    let mut probe = path;
    loop {
        if let Ok(canonical) = std::fs::canonicalize(probe) {
            return canonical;
        }
        match probe.parent() {
            Some(parent) => probe = parent,
            None => return path.to_path_buf(),
        }
    }
}

pub(crate) fn dir_size(path: &Path) -> u64 {
    let Ok(meta) = std::fs::metadata(path) else {
        return 0;
    };
    if meta.is_file() {
        return meta.len();
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| dir_size(&entry.path()))
        .sum()
}

fn resolve_binary(prefs: &AiPrefs) -> Option<PathBuf> {
    // The canonical resolver (pref path → PATH → prefixes → Ollama.app bundle
    // → running `ollama serve` process) — shared with board dispatch so the
    // manager and dispatch can never disagree about which Ollama is installed.
    crate::ollama::resolve_binary(prefs).map(|r| r.path)
}

async fn stop_managed_pid(pid: u32) -> AppResult<()> {
    tauri::async_runtime::spawn_blocking(move || {
        kill_pid(pid)?;
        for _ in 0..30 {
            if !pid_alive(pid) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        // SIGTERM hasn't taken in 3 s (e.g. the server is stuck releasing GPU
        // memory). Escalate to SIGKILL rather than returning success and
        // leaving a live, now-untracked orphan; give it a final beat to die.
        let _ = crate::ollama::kill_pid_force(pid);
        for _ in 0..10 {
            if !pid_alive(pid) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if pid_alive(pid) {
            return Err(AppError::Internal(format!(
                "Ollama process {pid} did not exit after SIGTERM and SIGKILL"
            )));
        }
        Ok(())
    })
    .await
    .map_err(|e| AppError::Internal(format!("failed to join Ollama stop task: {e}")))?
}

async fn binary_status(path: Option<PathBuf>) -> OllamaBinaryStatus {
    let version = match path.clone() {
        Some(path) => tauri::async_runtime::spawn_blocking(move || {
            Command::new(path)
                .arg("--version")
                .output()
                .ok()
                .map(|out| {
                    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if text.is_empty() {
                        String::from_utf8_lossy(&out.stderr).trim().to_string()
                    } else {
                        text
                    }
                })
        })
        .await
        .ok()
        .flatten()
        .filter(|s| !s.is_empty()),
        None => None,
    };
    OllamaBinaryStatus {
        detected: path.is_some(),
        path: path.map(|p| p.to_string_lossy().into_owned()),
        version,
        install_hint: "Download a PortBay-managed build below, or install from https://ollama.com/download / `brew install ollama` and set a custom binary path if it lives outside PATH.".into(),
    }
}

async fn port_holder(endpoint: &str) -> Option<String> {
    let port = endpoint_port(endpoint)?;
    if TcpListener::bind(("127.0.0.1", port)).is_ok() {
        return None;
    }
    tauri::async_runtime::spawn_blocking(move || {
        Command::new("lsof")
            .args(["-nP", &format!("-iTCP:{port}"), "-sTCP:LISTEN"])
            .output()
            .ok()
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| Some(format!("port {port} is already bound")))
    })
    .await
    .ok()
    .flatten()
}

fn endpoint_port(endpoint: &str) -> Option<u16> {
    let url = url::Url::parse(endpoint).ok()?;
    url.port_or_known_default()
}

fn ollama_host(endpoint: &str) -> String {
    let Ok(url) = url::Url::parse(endpoint) else {
        return endpoint.to_string();
    };
    let host = url.host_str().unwrap_or("127.0.0.1");
    let port = url.port_or_known_default().unwrap_or(11434);
    format!("{host}:{port}")
}

fn bool_env(value: bool) -> &'static str {
    if value {
        "1"
    } else {
        "0"
    }
}

/// Client for quick JSON endpoints (version/tags/ps/show/delete). The total
/// timeout is safe here because these calls answer in milliseconds.
fn http_client() -> AppResult<reqwest::Client> {
    reqwest::Client::builder()
        // A process that holds the port but never replies (zombie server, VPN,
        // a different app on :11434) would otherwise stall every overview/poll
        // for the full total timeout — bounding the connect phase keeps the UI
        // and the `ollama_running` poll responsive.
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| AppError::Internal(format!("failed to build HTTP client: {e}")))
}

/// Client for long-running calls (`/api/pull`, `/api/generate`): connect
/// timeout only. A multi-GB pull or a cold first model load legitimately runs
/// past any fixed total timeout — and reqwest surfaces a mid-body timeout as
/// the baffling "error decoding response body", which is exactly what model
/// pulls used to die with after 120 s.
fn long_client() -> AppResult<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| AppError::Internal(format!("failed to build HTTP client: {e}")))
}

fn http_err(error: reqwest::Error) -> AppError {
    AppError::Internal(format!("ollama request failed: {error}"))
}

async fn ensure_ok(res: reqwest::Response) -> AppResult<()> {
    if res.status().is_success() {
        Ok(())
    } else {
        Err(AppError::Internal(format!(
            "ollama returned HTTP {}",
            res.status()
        )))
    }
}

async fn json_response(res: reqwest::Response) -> AppResult<Value> {
    if !res.status().is_success() {
        return Err(AppError::Internal(format!(
            "ollama returned HTTP {}",
            res.status()
        )));
    }
    res.json::<Value>()
        .await
        .map_err(|e| AppError::Internal(format!("ollama returned unreadable JSON: {e}")))
}

fn parse_pull_event(line: &str) -> AppResult<PullEvent> {
    let value: Value = serde_json::from_str(line)
        .map_err(|e| AppError::Internal(format!("ollama pull stream was unreadable: {e}")))?;
    let status = value
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let error = value
        .get("error")
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok(PullEvent {
        done: error.is_some()
            || status.eq_ignore_ascii_case("success")
            || status.eq_ignore_ascii_case("done"),
        status,
        digest: value
            .get("digest")
            .and_then(Value::as_str)
            .map(str::to_string),
        total: value.get("total").and_then(Value::as_u64),
        completed: value.get("completed").and_then(Value::as_u64),
        error,
    })
}

/// True when a single reported layer (`total`) already exceeds the free space
/// snapshotted at pull start — the pull cannot complete, so bail early. `None`
/// on either side (size not yet streamed, or the disk probe failed) means
/// "don't know, don't block".
fn layer_exceeds_free(available_bytes: Option<u64>, total: Option<u64>) -> bool {
    matches!((available_bytes, total), (Some(a), Some(t)) if t > a)
}

fn is_pull_cancelled(id: &str) -> bool {
    cancelled_pulls()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains(id)
}

fn starter_models() -> Vec<StarterModel> {
    vec![
        StarterModel {
            name: "qwen2.5:7b",
            label: "Recommended dictation",
            fit: "Best PortBay default for smart dictation and coding prompts.",
            size_hint: "~4.7 GB",
        },
        StarterModel {
            name: "qwen2.5-coder:7b",
            label: "Developer work",
            fit: "Stronger code completion and repo Q&A on Apple Silicon.",
            size_hint: "~4.7 GB",
        },
        StarterModel {
            name: "llama3.1:8b",
            label: "General assistant",
            fit: "Balanced local chat model for product and research notes.",
            size_hint: "~4.9 GB",
        },
        StarterModel {
            name: "nomic-embed-text",
            label: "Embeddings",
            fit: "Small model for local semantic search and RAG experiments.",
            size_hint: "~274 MB",
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const PS_LINE: &str = "/Volumes/Dev SSD/ai/ollama serve COLORTERM=truecolor HOME=/Users/nour OLLAMA_MODELS=/Volumes/Dev SSD/ai/models PATH=/usr/bin:/bin LANG=en_US.UTF-8";

    #[test]
    fn env_block_starts_at_first_assignment_token() {
        let cmd = &PS_LINE[..env_block_start(PS_LINE)];
        assert_eq!(cmd.trim(), "/Volumes/Dev SSD/ai/ollama serve");
        assert!(PS_LINE[env_block_start(PS_LINE)..].starts_with("COLORTERM="));
        // No env at all → the whole line is command.
        assert_eq!(env_block_start("ollama serve"), "ollama serve".len());
    }

    #[test]
    fn ps_env_value_reads_values_with_spaces() {
        let env = &PS_LINE[env_block_start(PS_LINE)..];
        // The value runs to the next NAME= token, surviving the space in the
        // volume name.
        assert_eq!(
            ps_env_value(env, "OLLAMA_MODELS"),
            Some("/Volumes/Dev SSD/ai/models")
        );
        assert_eq!(ps_env_value(env, "HOME"), Some("/Users/nour"));
        // Last variable reads to end of line.
        assert_eq!(ps_env_value(env, "LANG"), Some("en_US.UTF-8"));
        assert_eq!(ps_env_value(env, "OLLAMA_HOST"), None);
    }

    #[test]
    fn version_newer_compares_numerically() {
        assert!(version_newer("0.30.6", "0.9.0")); // lexicographic would invert this
        assert!(version_newer("1.0.0", "0.99.9"));
        assert!(version_newer("0.30.10", "0.30.6"));
        assert!(!version_newer("0.30.6", "0.30.6"));
        assert!(!version_newer("0.30.6", "0.31.0"));
        // Trailing non-digits are ignored rather than fatal.
        assert!(version_newer("0.31.0-rc1", "0.30.6"));
    }

    #[test]
    fn env_assignment_shape_is_strict() {
        assert!(is_env_assignment("OLLAMA_MODELS=/x"));
        assert!(is_env_assignment("A=1"));
        assert!(!is_env_assignment("serve"));
        assert!(!is_env_assignment("=oops"));
        assert!(!is_env_assignment("9NUM=1"));
        assert!(!is_env_assignment("/path/with=equals"));
    }

    #[test]
    fn parse_pull_event_marks_terminal_states() {
        // A progress frame is not done and carries byte counts + digest.
        let e = parse_pull_event(
            r#"{"status":"pulling abc","digest":"sha256:abc","total":1000,"completed":250}"#,
        )
        .unwrap();
        assert!(!e.done);
        assert_eq!(e.total, Some(1000));
        assert_eq!(e.completed, Some(250));
        assert_eq!(e.digest.as_deref(), Some("sha256:abc"));
        assert!(e.error.is_none());

        // "success" / "done" are terminal, case-insensitively.
        assert!(parse_pull_event(r#"{"status":"success"}"#).unwrap().done);
        assert!(parse_pull_event(r#"{"status":"SUCCESS"}"#).unwrap().done);
        assert!(parse_pull_event(r#"{"status":"Done"}"#).unwrap().done);

        // An error frame is terminal and surfaces the message.
        let e = parse_pull_event(r#"{"error":"file does not exist"}"#).unwrap();
        assert!(e.done);
        assert_eq!(e.error.as_deref(), Some("file does not exist"));

        // A plain status with no totals: not done, no panic.
        let e = parse_pull_event(r#"{"status":"verifying sha256 digest"}"#).unwrap();
        assert!(!e.done);
        assert_eq!(e.total, None);

        // Malformed JSON is a typed error, not a panic.
        assert!(parse_pull_event("not json at all").is_err());
        assert!(parse_pull_event("").is_err());
    }

    #[test]
    fn layer_exceeds_free_only_trips_on_known_shortfall() {
        // Known size larger than known free space → bail.
        assert!(layer_exceeds_free(Some(1000), Some(1001)));
        // Exactly fits, or smaller → fine.
        assert!(!layer_exceeds_free(Some(1000), Some(1000)));
        assert!(!layer_exceeds_free(Some(1000), Some(999)));
        // Unknown free space (probe failed) or unknown size → never block.
        assert!(!layer_exceeds_free(None, Some(10_000_000_000)));
        assert!(!layer_exceeds_free(Some(1), None));
        assert!(!layer_exceeds_free(None, None));
    }

    // --- Streaming integration over a one-shot mock HTTP server ----------
    // Exercises the real `run_pull` loop end to end (HTTP body → chunk reads →
    // line buffering → parse → terminal/error/cancel/disk handling) without a
    // live Ollama, by pointing the endpoint at a throwaway TCP listener and
    // collecting emitted events through a constructed `Channel`.

    use std::sync::{Arc, Mutex as StdMutex};
    use tauri::ipc::{Channel, InvokeResponseBody};

    /// Serve a single canned HTTP/1.1 response and return its base URL. One
    /// connection per call (each test gets its own listener).
    fn serve_once(body: String) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            if let Ok((mut stream, _)) = listener.accept() {
                // Drain the request (headers + small JSON body); contents unused.
                let mut buf = [0u8; 2048];
                let _ = stream.read(&mut buf);
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        });
        format!("http://{addr}")
    }

    /// A `Channel<PullEvent>` that records every emitted event as JSON.
    fn collecting_channel() -> (Channel<PullEvent>, Arc<StdMutex<Vec<Value>>>) {
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        let ch = Channel::new(move |body: InvokeResponseBody| {
            if let InvokeResponseBody::Json(s) = body {
                sink.lock().unwrap().push(serde_json::from_str(&s).unwrap());
            }
            Ok(())
        });
        (ch, seen)
    }

    #[tokio::test]
    async fn run_pull_streams_progress_then_done() {
        let body =
            "{\"status\":\"pulling\",\"total\":100,\"completed\":40}\n{\"status\":\"success\"}\n";
        let url = serve_once(body.to_string());
        let (ch, seen) = collecting_channel();
        let r = run_pull(&url, "tiny", "pull-happy", &ch, None).await;
        assert!(r.is_ok(), "{r:?}");
        let seen = seen.lock().unwrap();
        assert!(seen
            .iter()
            .any(|e| e["status"] == "pulling" && e["total"] == 100));
        assert!(seen
            .iter()
            .any(|e| e["status"] == "success" && e["done"] == true));
    }

    #[tokio::test]
    async fn run_pull_surfaces_stream_error_frame() {
        let body = "{\"status\":\"pulling\"}\n{\"error\":\"model not found\"}\n";
        let url = serve_once(body.to_string());
        let (ch, seen) = collecting_channel();
        let r = run_pull(&url, "ghost", "pull-err", &ch, None).await;
        assert!(r.is_err());
        assert!(r.unwrap_err().to_string().contains("model not found"));
        assert!(seen
            .lock()
            .unwrap()
            .iter()
            .any(|e| e["error"] == "model not found"));
    }

    #[tokio::test]
    async fn run_pull_aborts_when_a_layer_exceeds_free_space() {
        // First frame already reports a layer larger than the 10 bytes free.
        let body = "{\"status\":\"pulling\",\"total\":5000000000}\n{\"status\":\"success\"}\n";
        let url = serve_once(body.to_string());
        let (ch, _seen) = collecting_channel();
        let r = run_pull(&url, "big", "pull-disk", &ch, Some(10)).await;
        let err = r.unwrap_err().to_string();
        assert!(err.contains("Not enough disk space"), "{err}");
    }

    #[tokio::test]
    async fn run_pull_honors_a_cancel_flag_set_before_streaming() {
        // Pre-set the cancel flag: the first loop iteration must observe it and
        // finish as cancelled rather than draining the stream as a real pull.
        cancelled_pulls()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert("pull-cancel".to_string());
        let body =
            "{\"status\":\"pulling\",\"total\":100,\"completed\":10}\n{\"status\":\"success\"}\n";
        let url = serve_once(body.to_string());
        let (ch, seen) = collecting_channel();
        let r = run_pull(&url, "tiny", "pull-cancel", &ch, None).await;
        assert!(r.is_ok(), "{r:?}");
        assert!(seen
            .lock()
            .unwrap()
            .iter()
            .any(|e| e["status"] == "cancelled" && e["done"] == true));
        cancelled_pulls()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove("pull-cancel");
    }
}
