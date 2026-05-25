//! Pro entitlement commands — the IPC surface over `crate::entitlements`.
//!
//! `get_entitlement` is the read every gate ultimately calls (via the Svelte
//! store). `refresh_entitlement` is invoked after GitHub login with the user's
//! bearer token (see the client-login card); `clear_entitlement` runs on logout.

use crate::auth::CLOUD_BASE_URL;
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
