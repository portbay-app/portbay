//! Tauri commands for Apple Image Playground (the in-process `ImageCreator`
//! bridge in `crate::imageplayground`). A status probe for the Models page and
//! a one-shot generate that returns the image as base64 PNG.

use crate::error::{AppError, AppResult};
use crate::imageplayground::ImagePlaygroundStatus;

/// Whether Apple Image Playground can generate on this machine (macOS 15.4+,
/// Apple Intelligence enabled, supported device). Drives the Models-page card.
#[tauri::command]
pub async fn imageplayground_check() -> AppResult<ImagePlaygroundStatus> {
    Ok(crate::imageplayground::check().await)
}

/// Generate one image from `prompt` with the system `ImageCreator`. Returns a
/// base64 PNG, or `None` if generation was cancelled. Fails with a message
/// containing `model_not_downloaded` when Apple's image-model resources aren't
/// on this Mac yet — the playground UI offers the system download flow then.
#[tauri::command]
pub async fn imageplayground_generate(prompt: Option<String>) -> AppResult<Option<String>> {
    crate::imageplayground::generate(prompt.as_deref())
        .await
        .map_err(|detail| AppError::Internal(format!("Image Playground: {detail}")))
}

/// Open Apple's Image Playground app. Its first launch walks the user through
/// downloading the system image model — third-party apps can't start that
/// download themselves, so this is the "Download" affordance for the
/// `model_not_downloaded` state.
#[tauri::command]
pub async fn imageplayground_open_app(app: tauri::AppHandle) -> AppResult<()> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_path("/System/Applications/Image Playground.app", None::<&str>)
        .map_err(|e| AppError::Internal(format!("could not open Image Playground: {e}")))
}
