//! Tauri commands for local text-to-speech (Kokoro), run through the same
//! `portbay-stt` sidecar (its FluidAudio link provides KokoroAne). Mirrors the
//! STT command shapes: a catalog+installed overview, a streamed download with
//! id-keyed cancellation, and a one-shot synthesize that returns WAV bytes.

use serde::Serialize;
use tauri::ipc::Channel;
use tauri::State;

use crate::commands::model_catalog::{TtsCatalogModel, TtsCatalogResult};
use crate::commands::stt::{SttDownloadEvent, SttInstalledModel};
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::stt::SttStatus;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsOverview {
    pub status: SttStatus,
    pub catalog: Vec<TtsCatalogModel>,
    pub installed: Vec<SttInstalledModel>,
    pub models_dir: String,
    /// True when the catalog was served from cache/bundled after a failed live
    /// refresh — mirrors the same flag on `SttOverview` so the UI can show a
    /// "catalog may be out of date" banner. Previously hardcoded `false`.
    pub catalog_stale: bool,
    /// Provenance of the catalog list: "live", "cache", or "bundled". The
    /// frontend uses this alongside `catalog_stale` to label the source.
    pub catalog_source: String,
}

fn models_dir(state: &State<'_, AppState>) -> String {
    crate::ollama::expand_tilde(&state.preferences_snapshot().tts.models_dir)
}

#[cfg(target_os = "macos")]
fn op_err(detail: String) -> AppError {
    AppError::Internal(format!("text-to-speech: {detail}"))
}

/// Catalog + installed voices + storage, one call for the TTS playground.
#[tauri::command]
pub async fn tts_overview(
    state: State<'_, AppState>,
    refresh: Option<bool>,
) -> AppResult<TtsOverview> {
    let dir = models_dir(&state);
    let status = crate::stt::check().await;
    let TtsCatalogResult {
        models: catalog,
        stale: catalog_stale,
        source: catalog_source,
    } = crate::commands::model_catalog::load_tts(&state, refresh.unwrap_or(false)).await;

    #[cfg(target_os = "macos")]
    let installed: Vec<SttInstalledModel> = if status.available {
        let resp =
            crate::stt::one_shot_op(serde_json::json!({ "op": "installed", "modelsDir": dir }))
                .await
                .map_err(op_err)?;
        serde_json::from_value(resp.get("installed").cloned().unwrap_or_default())
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    #[cfg(not(target_os = "macos"))]
    let installed: Vec<SttInstalledModel> = Vec::new();

    Ok(TtsOverview {
        status,
        catalog,
        installed,
        models_dir: dir,
        catalog_stale,
        catalog_source,
    })
}

/// Download a TTS model (the Kokoro mlmodelc chain). Same Channel/terminal-Done
/// contract as `stt_download_model`; the sidecar routes by engine.
#[cfg(target_os = "macos")]
#[tauri::command]
pub async fn tts_download_model(
    state: State<'_, AppState>,
    model: String,
    download_id: String,
    on_event: Channel<SttDownloadEvent>,
) -> AppResult<()> {
    let dir = models_dir(&state);
    let spec = crate::commands::model_catalog::tts_entry(&state, &model)
        .await
        .map(|m| crate::stt::DownloadSpec {
            engine: m.engine,
            repo_model: m.repo_model,
            parakeet_version: None,
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
pub async fn tts_download_model(
    _state: State<'_, AppState>,
    _model: String,
    _download_id: String,
    _on_event: Channel<SttDownloadEvent>,
) -> AppResult<()> {
    Err(AppError::Internal(
        "text-to-speech is macOS-only".to_string(),
    ))
}

/// Synthesize `text` with `voice` and return base64 WAV (24 kHz mono PCM) for
/// the webview to play. The voice pack is fetched on demand by the sidecar.
#[cfg(target_os = "macos")]
#[tauri::command]
pub async fn tts_speak(
    state: State<'_, AppState>,
    model: String,
    text: String,
    voice: Option<String>,
) -> AppResult<String> {
    let dir = models_dir(&state);
    // The first synth cold-loads the 7-stage Kokoro mlmodelc chain (and fetches
    // the voice pack on demand), which runs well past the 20 s metadata ceiling
    // `one_shot_op` uses — so allow a generous synthesis timeout.
    let resp = crate::stt::one_shot_op_with_timeout(
        serde_json::json!({
            "op": "tts-synthesize",
            "modelsDir": dir,
            "model": model,
            "text": text,
            "voice": voice,
        }),
        std::time::Duration::from_secs(180),
    )
    .await
    .map_err(op_err)?;
    // A failed synth comes back as {ok:false, error}; surface that detail rather
    // than a generic "no audio" so the real cause (missing voice pack, engine
    // load failure, …) reaches the user.
    if resp.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
        let detail = resp
            .get("error")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("synthesis failed");
        return Err(op_err(detail.to_string()));
    }
    resp.get("wavBase64")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| op_err("synthesis returned no audio".into()))
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub async fn tts_speak(
    _state: State<'_, AppState>,
    _model: String,
    _text: String,
    _voice: Option<String>,
) -> AppResult<String> {
    Err(AppError::Internal(
        "text-to-speech is macOS-only".to_string(),
    ))
}

/// Delete an installed TTS model (its directory under the TTS models dir).
#[tauri::command]
pub async fn tts_delete_model(state: State<'_, AppState>, model: String) -> AppResult<()> {
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
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (state, model);
        Err(AppError::Internal(
            "text-to-speech is macOS-only".to_string(),
        ))
    }
}
