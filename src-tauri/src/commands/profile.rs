//! Account profile commands — edit the display name and custom avatar.
//!
//! Each command mutates the account server-side (PortBay Cloud), then re-fetches
//! the freshly signed entitlement so the new `display_name` / `avatar_url` flow
//! back through the normal entitlement path. The access token is rotated first
//! (it's short-lived) via [`auth::access_token_refreshing`].

use crate::auth::{self, CLOUD_BASE_URL};
use crate::entitlements::{self, EffectiveEntitlement};
use crate::error::{AppError, AppResult};

/// Server-side cap, mirrored client-side so an oversized file fails fast with a
/// clear message instead of a 413 round-trip.
const AVATAR_MAX_BYTES: usize = 256 * 1024;

/// A usable access token, or a "not signed in" error. Rotates the session first.
async fn authed_token() -> AppResult<String> {
    auth::access_token_refreshing(CLOUD_BASE_URL)
        .await
        .ok_or_else(|| AppError::Internal("not signed in".into()))
}

/// Re-fetch + cache the signed entitlement after a successful profile mutation.
async fn resync(token: &str) -> AppResult<EffectiveEntitlement> {
    entitlements::refresh(CLOUD_BASE_URL, token)
        .await
        .map_err(AppError::Internal)
}

/// Set or clear the account display name. `None` (or empty) clears it; the app
/// then falls back to login-derived initials.
#[tauri::command]
pub async fn update_display_name(name: Option<String>) -> AppResult<EffectiveEntitlement> {
    let token = authed_token().await?;
    let url = format!("{}/account/profile", CLOUD_BASE_URL.trim_end_matches('/'));
    let resp = reqwest::Client::new()
        .patch(&url)
        .bearer_auth(&token)
        .json(&serde_json::json!({ "display_name": name }))
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("display-name update failed: {e}")))?;
    if !resp.status().is_success() {
        return Err(AppError::Internal(format!(
            "display-name update returned {}",
            resp.status()
        )));
    }
    resync(&token).await
}

/// Upload a custom avatar from a local file path. Validates the image type and
/// size locally before sending; on success the issuer stores it and the next
/// signed entitlement points `avatar_url` at it.
#[tauri::command]
pub async fn upload_avatar(path: String) -> AppResult<EffectiveEntitlement> {
    let token = authed_token().await?;
    let bytes =
        std::fs::read(&path).map_err(|e| AppError::Internal(format!("reading {path}: {e}")))?;
    if bytes.len() > AVATAR_MAX_BYTES {
        return Err(AppError::Internal(
            "image too large — pick one under 256 KB".into(),
        ));
    }
    let content_type = crate::avatar::detect_upload_mime(&bytes)
        .ok_or_else(|| AppError::Internal("unsupported image — use PNG, JPEG, or WebP".into()))?;

    let url = format!("{}/account/avatar", CLOUD_BASE_URL.trim_end_matches('/'));
    let resp = reqwest::Client::new()
        .put(&url)
        .bearer_auth(&token)
        .header("Content-Type", content_type)
        .body(bytes)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("avatar upload failed: {e}")))?;
    if !resp.status().is_success() {
        return Err(AppError::Internal(format!(
            "avatar upload returned {}",
            resp.status()
        )));
    }
    // Drop the local avatar cache so the new image is fetched on next render
    // (belt-and-braces — the new signed `avatar_url` also carries a fresh `?v=`).
    crate::avatar::clear_cache();
    resync(&token).await
}

/// Remove the custom avatar, reverting to the GitHub photo (or initials).
#[tauri::command]
pub async fn remove_avatar() -> AppResult<EffectiveEntitlement> {
    let token = authed_token().await?;
    let url = format!("{}/account/avatar", CLOUD_BASE_URL.trim_end_matches('/'));
    let resp = reqwest::Client::new()
        .delete(&url)
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("avatar removal failed: {e}")))?;
    if !resp.status().is_success() {
        return Err(AppError::Internal(format!(
            "avatar removal returned {}",
            resp.status()
        )));
    }
    crate::avatar::clear_cache();
    resync(&token).await
}
