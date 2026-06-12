//! Tauri commands for the local speech-to-text engine (`portbay-stt`
//! sidecar — see `crate::stt` for the client and the wire protocol).
//!
//! Mirrors the Ollama manager's command shapes: an overview the page polls,
//! a Channel-streamed download with id-keyed cancellation, and a delete.
//! The models directory comes from `preferences.stt.models_dir` on every
//! call — the sidecar is stateless about storage.

use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::stt::SttStatus;

/// One curated catalog entry, as shipped by the sidecar (CATALOG in
/// main.swift). Deserialized from the sidecar, serialized to the frontend —
/// both sides camelCase.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SttCatalogModel {
    pub id: String,
    /// "whisper" | "parakeet".
    pub engine: String,
    pub display_name: String,
    pub repo_model: String,
    pub approx_size_bytes: u64,
    pub languages: String,
    pub speed_note: String,
    pub recommended: bool,
    pub streaming: bool,
    /// Parakeet generation ("v2" | "v3"); `None` for Whisper. Passed to the
    /// sidecar so it downloads/loads the right FluidAudio version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parakeet_version: Option<String>,
    /// Model-weights license label (e.g. "MIT", "CC-BY-4.0") — disclosed in
    /// the download UI; the linked card is authoritative.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// Where the license/model card lives (Hugging Face card or upstream).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license_url: Option<String>,
    /// Expected install-content digest from the signed catalog (sha256 over
    /// the sorted per-file hashes — the sidecar's `--digest` format). When
    /// present the sidecar verifies the download before sealing it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_digest: Option<String>,
}

/// One installed model (a sealed install under the models dir).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SttInstalledModel {
    pub id: String,
    pub engine: String,
    pub size_bytes: u64,
}

/// Everything the AI page's "Speech to text" section renders, one call.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SttOverview {
    pub status: SttStatus,
    pub catalog: Vec<SttCatalogModel>,
    pub installed: Vec<SttInstalledModel>,
    pub models_dir: String,
    pub disk: super::ollama::DiskUsage,
    /// Catalog served from cache/bundled after a failed live refresh.
    pub catalog_stale: bool,
    /// "live" | "cache" | "bundled" — provenance of `catalog`.
    pub catalog_source: String,
    /// Microphone TCC status for PortBay: "authorized" | "denied" |
    /// "restricted" | "not_determined" | "unknown". The capture runs in the
    /// sidecar but TCC attributes it to the app, so this is the effective
    /// state — surfaced so the UI can pre-flight instead of failing
    /// mid-session at first Fn-hold.
    pub mic_permission: String,
}

/// Download progress for the AI page, streamed over a `Channel` like
/// `ollama_install`'s `InstallEvent`. Fraction-based — the engine libraries
/// report fractions, not bytes (the catalog's approximate size gives the UI
/// a byte estimate if it wants one).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum SttDownloadEvent {
    #[serde(rename = "progress")]
    Progress { fraction: f64, phase: String },
    #[serde(rename = "done")]
    Done {
        success: bool,
        cancelled: bool,
        error: Option<String>,
    },
}

fn models_dir(state: &State<'_, AppState>) -> String {
    crate::ollama::expand_tilde(&state.preferences_snapshot().stt.models_dir)
}

#[cfg(target_os = "macos")]
fn op_err(detail: String) -> AppError {
    AppError::Internal(format!("speech-to-text: {detail}"))
}

fn fmt_gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1_000_000_000.0)
}

/// Best-effort disk-space pre-check before a download. Looks up the model's
/// approximate size in the catalog and refuses the download if the target
/// volume can't hold it plus a little extraction headroom — otherwise the
/// engine library fails mid-stream with a vague error and leaves a partial
/// install. A missing catalog entry or a failed disk probe skips the check
/// (never block a download on uncertainty); only a positive shortfall stops it.
#[cfg(target_os = "macos")]
async fn ensure_disk_space_for(
    state: &State<'_, AppState>,
    dir: &str,
    model: &str,
) -> AppResult<()> {
    let Some(entry) = crate::commands::model_catalog::stt_entry(state, model).await else {
        return Ok(());
    };
    if entry.approx_size_bytes == 0 {
        return Ok(());
    }
    // 15% headroom: sizes are approximate and the engine needs scratch space
    // while sealing the install.
    let needed = entry
        .approx_size_bytes
        .saturating_add(entry.approx_size_bytes / 100 * 15);
    let Ok(disk) = disk_usage_of(dir.to_string()).await else {
        return Ok(());
    };
    if disk.available_bytes < needed {
        return Err(AppError::BadInput(format!(
            "Not enough disk space for {}: needs about {} but only {} is free on the models volume. Free up space or choose a different models folder.",
            entry.display_name,
            fmt_gb(needed),
            fmt_gb(disk.available_bytes),
        )));
    }
    Ok(())
}

/// Probe the local STT engine for the settings/AI-page UI: is the sidecar
/// present and runnable on this machine? Mirrors `dictation_provider_status`
/// in shape — a plain status struct, never an error (an unreachable engine
/// is a state to display, not a failure to toast).
#[tauri::command]
pub async fn stt_status() -> SttStatus {
    crate::stt::check().await
}

/// Catalog + installed models + storage, one call for the page. `refresh`
/// forces a live re-fetch of the PortBay Model Catalog (the AI page's refresh
/// button); otherwise the catalog comes from the daily cache / bundled fallback.
#[tauri::command]
pub async fn stt_overview(
    state: State<'_, AppState>,
    refresh: Option<bool>,
) -> AppResult<SttOverview> {
    let dir = models_dir(&state);
    let status = crate::stt::check().await;

    // The catalog is Rust-owned now (live signed manifest + cache + bundled
    // fallback), not the sidecar's static list — so new models ship without an
    // app release, and the list renders even when the sidecar can't run.
    let catalog_result =
        crate::commands::model_catalog::load_stt(&state, refresh.unwrap_or(false)).await;

    #[cfg(target_os = "macos")]
    let installed: Vec<SttInstalledModel> = if status.available {
        let installed_response =
            crate::stt::one_shot_op(serde_json::json!({ "op": "installed", "modelsDir": dir }))
                .await
                .map_err(op_err)?;
        serde_json::from_value(
            installed_response
                .get("installed")
                .cloned()
                .unwrap_or_default(),
        )
        .unwrap_or_default()
    } else {
        Vec::new()
    };
    #[cfg(not(target_os = "macos"))]
    let installed: Vec<SttInstalledModel> = Vec::new();

    let disk = disk_usage_of(dir.clone()).await?;
    Ok(SttOverview {
        status,
        catalog: catalog_result.models,
        installed,
        models_dir: dir,
        disk,
        catalog_stale: catalog_result.stale,
        catalog_source: catalog_result.source,
        mic_permission: crate::dictation_session::mic_permission(),
    })
}

/// Trigger the system microphone prompt (or report the settled state).
/// Returns true when access is granted. The UI calls this from its
/// "Enable microphone" CTA while the status is still `not_determined`, so
/// the grant happens before the first dictation instead of mid-session.
#[tauri::command]
pub async fn stt_request_mic_access() -> bool {
    crate::dictation_session::request_mic_access().await
}

/// Download a catalog model, streaming progress into `on_event`. The
/// terminal `done` event always arrives — success, failure, or cancel.
#[cfg(target_os = "macos")]
#[tauri::command]
pub async fn stt_download_model(
    state: State<'_, AppState>,
    model: String,
    download_id: String,
    on_event: Channel<SttDownloadEvent>,
) -> AppResult<()> {
    let dir = models_dir(&state);
    // Refuse early (with the same terminal Done event the UI expects) if the
    // volume plainly can't hold the model, rather than failing mid-download.
    if let Err(e) = ensure_disk_space_for(&state, &dir, &model).await {
        let _ = on_event.send(SttDownloadEvent::Done {
            success: false,
            cancelled: false,
            error: Some(e.to_string()),
        });
        return Err(e);
    }
    // Resolve the engine/variant from the PortBay Model Catalog and hand it to
    // the sidecar, so models the sidecar's bundled list doesn't know still
    // download correctly (the live-catalog path).
    let spec = crate::commands::model_catalog::stt_entry(&state, &model)
        .await
        .map(|m| crate::stt::DownloadSpec {
            engine: m.engine,
            repo_model: m.repo_model,
            parakeet_version: m.parakeet_version,
            content_digest: m.content_digest,
        });
    let progress_channel = on_event.clone();
    let outcome =
        crate::stt::run_download(&dir, &model, &download_id, spec, move |fraction, phase| {
            let _ = progress_channel.send(SttDownloadEvent::Progress { fraction, phase });
        })
        .await;

    match outcome {
        Ok(done) => {
            let _ = on_event.send(SttDownloadEvent::Done {
                success: done.success,
                cancelled: done.cancelled,
                error: done.error,
            });
            Ok(())
        }
        Err(detail) => {
            let _ = on_event.send(SttDownloadEvent::Done {
                success: false,
                cancelled: false,
                error: Some(detail.clone()),
            });
            Err(op_err(detail))
        }
    }
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub async fn stt_download_model(
    _state: State<'_, AppState>,
    _model: String,
    _download_id: String,
    on_event: Channel<SttDownloadEvent>,
) -> AppResult<()> {
    let _ = on_event.send(SttDownloadEvent::Done {
        success: false,
        cancelled: false,
        error: Some("speech-to-text is macOS-only".to_string()),
    });
    Ok(())
}

/// Cancel an in-flight download. No-op for unknown ids (already finished).
#[tauri::command]
pub async fn stt_cancel_download(download_id: String) -> AppResult<()> {
    #[cfg(target_os = "macos")]
    crate::stt::cancel_download(&download_id).await;
    #[cfg(not(target_os = "macos"))]
    let _ = download_id;
    Ok(())
}

/// Delete an installed model (its whole directory under the models dir).
#[tauri::command]
pub async fn stt_delete_model(state: State<'_, AppState>, model: String) -> AppResult<()> {
    #[cfg(target_os = "macos")]
    {
        let dir = models_dir(&state);
        let response = crate::stt::one_shot_op(
            serde_json::json!({ "op": "delete", "modelsDir": dir, "model": model }),
        )
        .await
        .map_err(op_err)?;
        if response.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
            let detail = response
                .get("error")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("delete failed");
            return Err(op_err(detail.to_string()));
        }
        // Drop the resident engine process in case it was holding the model we
        // just deleted — a stale engine pointing at removed files must not
        // serve the next capture; it respawns + reloads on the next prewarm.
        crate::stt::evict_engine();
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (state, model);
        Ok(())
    }
}

/// Start a local-engine capture session: the sidecar loads the model, opens
/// the mic, and streams `dictation://listening` / `stt://partial` /
/// `stt://level` events. Resolves when the mic is hot — micSession's arming
/// phase covers the (possibly slow, cold-load) wait, exactly like macOS
/// dictation's confirmation window. `mode` labels the session for the
/// overlay ("dictation" | "edit" | "rewrite"; absent/unknown = dictation).
#[cfg(target_os = "macos")]
#[tauri::command]
pub async fn stt_start_capture(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    model: String,
    mode: Option<String>,
) -> AppResult<()> {
    let dir = models_dir(&state);
    // Resolve the recognizer bias from the unified Context Store (global term
    // set for in-app — §10.3), gated by engine capability so we never send a
    // prompt Parakeet can't use. Same resolver the rewrite consumes, so the two
    // biasing seams stay consistent.
    let bias = if crate::dictation_context::recognizer_bias_enabled()
        && crate::dictation_context::engine_supports_text_bias(&model)
    {
        let terms = crate::commands::dictation::recognizer_terms(&state, None, None).await;
        crate::dictation_context::instrument::record_bias(terms.len());
        tracing::debug!(model = %model, terms = terms.len(), "stt: recognizer bias resolved");
        terms
    } else {
        // No recognizer bias: engine can't take a prompt (Parakeet, and the
        // turbo/distil Whisper decoders — see `engine_supports_text_bias`) or
        // the kill switch is set. The always-on correction net + rewrite
        // still fix known terms downstream.
        Vec::new()
    };
    // The notch HUD covers in-app sessions too (same UI as dictate-anywhere):
    // arming while the model loads, live once the mic is hot.
    crate::dictation_anywhere::inapp_arming(&app, mode.as_deref().unwrap_or("dictation")).await;
    match crate::stt::start_capture(app.clone(), &dir, &model, &bias).await {
        Ok(()) => {
            crate::dictation_anywhere::inapp_live(&app);
            Ok(())
        }
        Err(detail) => {
            // micSession toasts the failure; the overlay just goes away.
            crate::dictation_anywhere::inapp_hidden(&app);
            Err(op_err(detail))
        }
    }
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub async fn stt_start_capture(
    _app: tauri::AppHandle,
    _state: State<'_, AppState>,
    _model: String,
    _mode: Option<String>,
) -> AppResult<()> {
    Err(AppError::Internal(
        "speech-to-text is macOS-only".to_string(),
    ))
}

/// Stop the capture and return the final transcript (possibly empty for a
/// silent session). The frontend splices this into the focused field before
/// running the rewrite layer.
#[tauri::command]
pub async fn stt_stop_capture(app: tauri::AppHandle) -> AppResult<String> {
    #[cfg(target_os = "macos")]
    {
        crate::dictation_anywhere::inapp_processing(&app);
        match crate::stt::stop_capture().await {
            Ok(transcript) => {
                // Done beat + hide run spawned — the transcript returns to
                // the frontend without waiting out the exit animation.
                crate::dictation_anywhere::inapp_done(&app);
                Ok(transcript)
            }
            Err(detail) => {
                crate::dictation_anywhere::inapp_hidden(&app);
                Err(op_err(detail))
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Err(AppError::Internal(
            "speech-to-text is macOS-only".to_string(),
        ))
    }
}

/// Tear down the capture without a transcript — the cancel path (arming
/// aborted, handoff), where the words are discarded by design.
#[tauri::command]
pub async fn stt_cancel_capture(app: tauri::AppHandle) -> AppResult<()> {
    #[cfg(target_os = "macos")]
    {
        crate::stt::cancel_capture().await;
        crate::dictation_anywhere::inapp_hidden(&app);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = app;
    Ok(())
}

/// Fire-and-forget: page the selected model in at dictation START so the
/// capture (and final pass) at dictation end starts hot. Same contract as
/// `dictation_prewarm` for the rewrite layer.
#[tauri::command]
pub async fn stt_prewarm(state: State<'_, AppState>, model: String) -> AppResult<()> {
    #[cfg(target_os = "macos")]
    {
        let dir = models_dir(&state);
        tauri::async_runtime::spawn(async move {
            crate::stt::prewarm(&dir, &model).await;
        });
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (state, model);
    Ok(())
}

/// Disk usage of the (configured) STT models dir — same resolution rules as
/// the Ollama models disk card: symlink-resolved nearest mount point, so a
/// dir on an external volume reports that volume's space.
async fn disk_usage_of(dir: String) -> AppResult<super::ollama::DiskUsage> {
    tauri::async_runtime::spawn_blocking(move || {
        use sysinfo::Disks;
        let path = std::path::Path::new(&dir);
        let used = super::ollama::dir_size(path);
        let canonical = super::ollama::nearest_canonical(path);
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
        Ok(super::ollama::DiskUsage {
            path: dir,
            total_bytes: total,
            used_bytes: used,
            available_bytes: available,
            volume,
        })
    })
    .await
    .map_err(|e| AppError::Internal(format!("failed to join disk scan: {e}")))?
}
