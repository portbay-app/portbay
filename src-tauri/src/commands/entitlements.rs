//! Pro entitlement commands — the IPC surface over `crate::entitlements`.
//!
//! `get_entitlement` is the read every gate ultimately calls (via the Svelte
//! store). `refresh_entitlement` is invoked after GitHub login with the user's
//! bearer token (see the client-login card); `clear_entitlement` runs on logout.

use crate::auth::{self, CLOUD_BASE_URL};
use crate::entitlements::{self, EffectiveEntitlement};
use crate::error::{AppError, AppResult};

/// Current effective entitlement, computed from the on-disk cache with offline
/// grace. No network — safe to call on every gate check / app start.
#[tauri::command]
pub async fn get_entitlement() -> AppResult<EffectiveEntitlement> {
    Ok(entitlements::current())
}

/// Fetch a fresh signed entitlement from the issuer with the caller's session
/// token, verify its signature, cache it, and return the new effective state.
#[tauri::command]
pub async fn refresh_entitlement(token: String) -> AppResult<EffectiveEntitlement> {
    entitlements::refresh(CLOUD_BASE_URL, &token)
        .await
        .map_err(AppError::Internal)
}

/// Drop the cached entitlement (logout). Falls back to the anonymous tier immediately.
#[tauri::command]
pub async fn clear_entitlement() -> AppResult<EffectiveEntitlement> {
    entitlements::clear_cache()?;
    Ok(entitlements::anonymous_fallback())
}

/// Create a per-user Paddle checkout and return the hosted-checkout URL for the
/// frontend to open in the system browser. Requires a signed-in session — the
/// user's id is stamped into the checkout server-side (`/account/checkout`) so
/// the resulting subscription webhook issues *their* Pro license.
#[tauri::command]
pub async fn pro_checkout_url() -> AppResult<String> {
    let token = auth::access_token_refreshing(CLOUD_BASE_URL)
        .await
        .ok_or_else(|| AppError::Internal("Sign in to PortBay to upgrade to Pro.".into()))?;

    let url = format!("{}/account/checkout", CLOUD_BASE_URL.trim_end_matches('/'));
    let resp = reqwest::Client::new()
        .post(&url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("checkout request failed: {e}")))?;

    if resp.status().as_u16() == 503 {
        return Err(AppError::Internal(
            "Pro checkout isn't available yet — please try again soon.".into(),
        ));
    }
    if !resp.status().is_success() {
        return Err(AppError::Internal(format!(
            "checkout request returned {}",
            resp.status()
        )));
    }

    let body = resp
        .json::<serde_json::Value>()
        .await
        .map_err(|e| AppError::Internal(format!("reading checkout response failed: {e}")))?;
    body.get("checkout_url")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| AppError::Internal("checkout response had no URL".into()))
}
