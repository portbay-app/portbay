//! Account login commands — the IPC surface over `crate::auth`.
//!
//! Login is a two-step poll: `begin_login` opens a flow (and, for GitHub,
//! returns a URL the frontend opens in the system browser); `poll_login` is
//! called on an interval until the flow is ready, at which point the session is
//! stored in the keychain and the entitlement is refreshed. `account_resync`
//! runs on app start to re-verify a stored session.

use tauri::State;

use crate::auth::{self, PendingLogin, PollOutcome, RefreshOutcome, CLOUD_BASE_URL};
use crate::entitlements::{self, EffectiveEntitlement};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(serde::Serialize)]
pub struct BeginLoginResponse {
    /// For the GitHub method: the URL the frontend should open in the browser.
    pub authorize_url: Option<String>,
}

/// Open a login flow. `method` is `"github"` or `"email"`; `email` is required
/// for `"email"`. Stores the poll token in app state; returns the browser URL
/// for GitHub (the magic-link email is sent server-side for `"email"`).
#[tauri::command]
pub async fn begin_login(
    state: State<'_, AppState>,
    method: String,
    email: Option<String>,
) -> AppResult<BeginLoginResponse> {
    let resp = auth::init(CLOUD_BASE_URL, &method, email.as_deref())
        .await
        .map_err(AppError::Internal)?;
    *state
        .pending_login
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Some(PendingLogin {
        poll_token: resp.poll_token,
    });
    Ok(BeginLoginResponse {
        authorize_url: resp.authorize_url,
    })
}

#[derive(serde::Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum LoginPoll {
    /// Still waiting on the user to complete the flow.
    Pending,
    /// Signed in — carries the freshly verified effective entitlement.
    Ready { entitlement: EffectiveEntitlement },
    /// Flow expired or no login in progress.
    Expired,
}

/// Poll the in-flight login once. On success, persists the session to the
/// keychain and returns the refreshed entitlement.
#[tauri::command]
pub async fn poll_login(state: State<'_, AppState>) -> AppResult<LoginPoll> {
    let pending = state
        .pending_login
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let Some(pending) = pending else {
        return Ok(LoginPoll::Expired);
    };

    match auth::poll(CLOUD_BASE_URL, &pending.poll_token)
        .await
        .map_err(AppError::Internal)?
    {
        PollOutcome::Pending => Ok(LoginPoll::Pending),
        PollOutcome::Expired => {
            *state
                .pending_login
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = None;
            Ok(LoginPoll::Expired)
        }
        PollOutcome::Ready(session) => {
            *state
                .pending_login
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = None;
            auth::store_session(&session).map_err(AppError::Internal)?;
            let entitlement = entitlements::refresh(CLOUD_BASE_URL, &session.access_token)
                .await
                .map_err(AppError::Internal)?;
            Ok(LoginPoll::Ready { entitlement })
        }
    }
}

/// Cancel any in-flight login (e.g. the user closed the sign-in sheet).
#[tauri::command]
pub async fn cancel_login(state: State<'_, AppState>) -> AppResult<()> {
    *state
        .pending_login
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = None;
    Ok(())
}

/// Sign out: revoke the session server-side (best effort), clear the keychain
/// and the cached entitlement, and fall back to anonymous.
#[tauri::command]
pub async fn logout() -> AppResult<EffectiveEntitlement> {
    if let Some(session) = auth::load_session() {
        auth::logout_remote(CLOUD_BASE_URL, &session.access_token).await;
    }
    let _ = auth::clear_session();
    let _ = entitlements::clear_cache();
    Ok(entitlements::anonymous_fallback())
}

/// Re-verify a stored session on app start. Rotates the (likely-expired) access
/// token via the refresh token, re-fetches the signed entitlement, and returns
/// the effective state. Network failures keep the cached entitlement (offline
/// grace); only a definitive 401 clears the dead session.
#[tauri::command]
pub async fn account_resync() -> AppResult<EffectiveEntitlement> {
    let Some(session) = auth::load_session() else {
        // Not signed in — whatever the cache says (anonymous, or a leftover).
        return Ok(entitlements::current());
    };

    match auth::refresh_session(CLOUD_BASE_URL, &session.refresh_token).await {
        RefreshOutcome::Rotated(new_session) => {
            let _ = auth::store_session(&new_session);
            match entitlements::refresh(CLOUD_BASE_URL, &new_session.access_token).await {
                Ok(eff) => Ok(eff),
                // Refreshed auth but couldn't fetch the license (transient) —
                // fall back to the cached effective entitlement.
                Err(_) => Ok(entitlements::current()),
            }
        }
        RefreshOutcome::Unauthorized => {
            // Session is dead — clear it and the cached license, drop to anon.
            let _ = auth::clear_session();
            let _ = entitlements::clear_cache();
            Ok(entitlements::anonymous_fallback())
        }
        // Offline / server hiccup — trust the cache (honors Pro grace).
        RefreshOutcome::Transient => Ok(entitlements::current()),
    }
}
