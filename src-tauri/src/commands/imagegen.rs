//! Tauri commands for local image generation (FLUX / SD3), run through the
//! `portbay-imagegen` DiffusionKit sidecar. Mirrors the STT/TTS command
//! shapes: a catalog+installed overview, a streamed download with id-keyed
//! cancellation, and a streamed generate that ends with a base64 PNG.

use serde::Serialize;
use tauri::ipc::Channel;
use tauri::State;

use crate::commands::model_catalog::ImageCatalogModel;
use crate::commands::stt::SttDownloadEvent;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::stt::SttStatus;

/// One installed image model on disk (mirrors `SttInstalledModel`).
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImagegenInstalledModel {
    pub id: String,
    pub engine: String,
    pub size_bytes: u64,
}

/// Everything the Image-generation category + playground render, one call.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImagegenOverview {
    pub status: SttStatus,
    pub catalog: Vec<ImageCatalogModel>,
    pub installed: Vec<ImagegenInstalledModel>,
    pub models_dir: String,
    pub catalog_stale: bool,
    /// "live" (verified manifest), "cache", or "bundled" — same provenance
    /// the STT/TTS sections surface.
    pub catalog_source: String,
}

/// Per-step diffusion progress, then a terminal frame with the PNG. Tagged by
/// `kind` so the playground runs a generating → done/error state machine.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ImagegenGenerateEvent {
    #[serde(rename_all = "camelCase")]
    Progress {
        fraction: f64,
        step: u32,
        total_steps: u32,
    },
    #[serde(rename_all = "camelCase")]
    Done { image_base64: String },
    #[serde(rename_all = "camelCase")]
    Error { message: String },
}

fn models_dir(state: &State<'_, AppState>) -> String {
    crate::ollama::expand_tilde(&state.preferences_snapshot().imagegen.models_dir)
}

#[cfg(target_os = "macos")]
fn op_err(detail: String) -> AppError {
    AppError::Internal(format!("image generation: {detail}"))
}

/// Catalog + installed models + storage, one call for the AI page.
#[tauri::command]
pub async fn imagegen_overview(
    state: State<'_, AppState>,
    refresh: Option<bool>,
) -> AppResult<ImagegenOverview> {
    let dir = models_dir(&state);
    let status = crate::imagegen::check().await;
    let catalog =
        crate::commands::model_catalog::load_image(&state, refresh.unwrap_or(false)).await;

    #[cfg(target_os = "macos")]
    let installed: Vec<ImagegenInstalledModel> = if status.available {
        let resp = crate::imagegen::one_shot_op(
            serde_json::json!({ "op": "installed", "modelsDir": dir }),
        )
        .await
        .map_err(op_err)?;
        serde_json::from_value(resp.get("installed").cloned().unwrap_or_default())
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    #[cfg(not(target_os = "macos"))]
    let installed: Vec<ImagegenInstalledModel> = Vec::new();

    Ok(ImagegenOverview {
        status,
        catalog: catalog.models,
        installed,
        models_dir: dir,
        catalog_stale: catalog.stale,
        catalog_source: catalog.source,
    })
}

/// Download an image model. Same Channel/terminal-Done contract as
/// `stt_download_model`; the sidecar routes by engine.
#[cfg(target_os = "macos")]
#[tauri::command]
pub async fn imagegen_download_model(
    state: State<'_, AppState>,
    model: String,
    download_id: String,
    on_event: Channel<SttDownloadEvent>,
) -> AppResult<()> {
    let dir = models_dir(&state);
    let spec = crate::commands::model_catalog::image_entry(&state, &model)
        .await
        .map(|m| crate::imagegen::DownloadSpec {
            engine: m.engine,
            repo_model: m.repo_model,
            compiled_glob: m.compiled_glob,
            content_digest: m.content_digest,
        });
    let progress_channel = on_event.clone();
    let outcome =
        crate::imagegen::run_download(&dir, &model, &download_id, spec, move |fraction, phase| {
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
pub async fn imagegen_download_model(
    _state: State<'_, AppState>,
    _model: String,
    _download_id: String,
    _on_event: Channel<SttDownloadEvent>,
) -> AppResult<()> {
    Err(AppError::Internal(
        "image generation is macOS-only".to_string(),
    ))
}

/// Cancel an in-flight image-model download by id.
#[tauri::command]
pub async fn imagegen_cancel_download(download_id: String) -> AppResult<()> {
    #[cfg(target_os = "macos")]
    {
        crate::imagegen::cancel_download(&download_id).await;
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = download_id;
        Err(AppError::Internal(
            "image generation is macOS-only".to_string(),
        ))
    }
}

/// Generate one image. Streams per-step progress over `on_event`, then a
/// terminal `Done { imageBase64 }` (PNG) or `Error`.
#[cfg(target_os = "macos")]
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn imagegen_generate(
    state: State<'_, AppState>,
    model: String,
    generate_id: String,
    prompt: String,
    negative_prompt: Option<String>,
    steps: Option<u32>,
    guidance: Option<f64>,
    size: Option<u32>,
    seed: Option<i64>,
    on_event: Channel<ImagegenGenerateEvent>,
) -> AppResult<()> {
    let dir = models_dir(&state);
    let params = crate::imagegen::GenerateParams {
        prompt,
        negative_prompt,
        steps,
        guidance,
        size,
        seed,
    };
    let progress_channel = on_event.clone();
    let result = crate::imagegen::generate(
        &dir,
        &model,
        &generate_id,
        params,
        move |fraction, step, total_steps| {
            let _ = progress_channel.send(ImagegenGenerateEvent::Progress {
                fraction,
                step,
                total_steps,
            });
        },
    )
    .await;
    match result {
        Ok(image_base64) => {
            let _ = on_event.send(ImagegenGenerateEvent::Done { image_base64 });
            Ok(())
        }
        Err(detail) => {
            let _ = on_event.send(ImagegenGenerateEvent::Error {
                message: detail.clone(),
            });
            Err(op_err(detail))
        }
    }
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn imagegen_generate(
    _state: State<'_, AppState>,
    _model: String,
    _generate_id: String,
    _prompt: String,
    _negative_prompt: Option<String>,
    _steps: Option<u32>,
    _guidance: Option<f64>,
    _size: Option<u32>,
    _seed: Option<i64>,
    _on_event: Channel<ImagegenGenerateEvent>,
) -> AppResult<()> {
    Err(AppError::Internal(
        "image generation is macOS-only".to_string(),
    ))
}

/// Cancel an in-flight generation by id — kills its dedicated sidecar
/// process so a hung diffusion can't wedge the Generate button.
#[tauri::command]
pub async fn imagegen_cancel_generate(generate_id: String) -> AppResult<()> {
    #[cfg(target_os = "macos")]
    {
        crate::imagegen::cancel_generate(&generate_id);
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = generate_id;
        Err(AppError::Internal(
            "image generation is macOS-only".to_string(),
        ))
    }
}

/// Delete an installed image model (its directory under the imagegen dir).
#[tauri::command]
pub async fn imagegen_delete_model(state: State<'_, AppState>, model: String) -> AppResult<()> {
    #[cfg(target_os = "macos")]
    {
        let dir = models_dir(&state);
        let response = crate::imagegen::one_shot_op(
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
            "image generation is macOS-only".to_string(),
        ))
    }
}
