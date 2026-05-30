//! Account session + login handshake against PortBay Cloud.
//!
//! Login uses the backend's unified **flow + poll** handshake (see
//! `portbay-cloud` `/auth/session/*`): the app opens a flow, drives it via the
//! system browser (GitHub) or a magic-link email, then polls for the issued
//! tokens. Tokens never pass through the webview — they go from `reqwest`
//! straight into the OS keychain.
//!
//! The session is `{ access_token (15-min JWT), refresh_token (90-day opaque) }`.
//! On app start we rotate the refresh token to mint a fresh access token, then
//! re-verify the entitlement.

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

/// Production PortBay Cloud base URL (branded custom domain — must be added to
/// the Worker in Cloudflare). Never localhost or workers.dev in shipped builds.
pub const CLOUD_BASE_URL: &str = "https://cloud.portbay.app";

// Shown verbatim in the macOS keychain access prompt ("…stored in
// 'PortBay Account'…"), so it's worded for a human, not as a reverse-DNS id.
// Renaming this points us at a fresh keychain item: any session stored under
// the old name is abandoned (harmless — the user just signs in once more).
const KEYCHAIN_SERVICE: &str = "PortBay Account";
const KEYCHAIN_USER: &str = "default";

// ---------------------------------------------------------------------------
// Session + keychain
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub access_token: String,
    pub refresh_token: String,
}

fn entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_USER).map_err(|e| e.to_string())
}

/// Persist the session in the OS keychain (single JSON blob).
pub fn store_session(session: &Session) -> Result<(), String> {
    let json = serde_json::to_string(session).map_err(|e| e.to_string())?;
    entry()?.set_password(&json).map_err(|e| e.to_string())
}

/// Load the cached session, or `None` if not signed in / unreadable.
pub fn load_session() -> Option<Session> {
    let raw = entry().ok()?.get_password().ok()?;
    serde_json::from_str(&raw).ok()
}

/// Remove the cached session. Idempotent (a missing entry is success).
pub fn clear_session() -> Result<(), String> {
    match entry()?.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Pending login (held in AppState while a flow is in flight)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct PendingLogin {
    pub poll_token: String,
}

// ---------------------------------------------------------------------------
// Flow handshake
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct InitResponse {
    #[allow(dead_code)]
    pub flow_id: String,
    pub poll_token: String,
    /// Present for the GitHub method — the URL to open in the system browser.
    pub authorize_url: Option<String>,
}

/// Open a login flow. `method` is `"github"` or `"email"`; `email` is required
/// for the email method.
pub async fn init(
    base_url: &str,
    method: &str,
    email: Option<&str>,
) -> Result<InitResponse, String> {
    let url = format!("{}/auth/session/init", base_url.trim_end_matches('/'));
    let mut body = serde_json::json!({ "method": method });
    if let Some(e) = email {
        body["email"] = serde_json::Value::String(e.to_string());
    }
    let resp = reqwest::Client::new()
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("login init failed: {e}"))?;
    if !resp.status().is_success() {
        let code = resp.status();
        let detail = resp.text().await.unwrap_or_default();
        return Err(format!("login init returned {code}: {detail}"));
    }
    resp.json::<InitResponse>()
        .await
        .map_err(|e| format!("reading login init failed: {e}"))
}

pub enum PollOutcome {
    Pending,
    Ready(Session),
    Expired,
}

#[derive(Debug, Deserialize)]
struct PollReady {
    access_token: String,
    refresh_token: String,
}

/// Poll a pending flow once.
pub async fn poll(base_url: &str, poll_token: &str) -> Result<PollOutcome, String> {
    let url = format!("{}/auth/session/poll", base_url.trim_end_matches('/'));
    let resp = reqwest::Client::new()
        .post(&url)
        .json(&serde_json::json!({ "poll_token": poll_token }))
        .send()
        .await
        .map_err(|e| format!("login poll failed: {e}"))?;

    match resp.status().as_u16() {
        202 => Ok(PollOutcome::Pending),
        200 => {
            let ready = resp
                .json::<PollReady>()
                .await
                .map_err(|e| format!("reading login result failed: {e}"))?;
            Ok(PollOutcome::Ready(Session {
                access_token: ready.access_token,
                refresh_token: ready.refresh_token,
            }))
        }
        // 404 unknown / 410 expired or already consumed.
        404 | 410 => Ok(PollOutcome::Expired),
        other => Err(format!("login poll returned {other}")),
    }
}

/// Result of attempting to rotate the refresh token.
pub enum RefreshOutcome {
    Rotated(Session),
    /// The refresh token is invalid/expired/revoked — the session is dead.
    Unauthorized,
    /// Transient (network/server) failure — keep the cached session.
    Transient,
}

/// Rotate the refresh token, minting a fresh access token.
pub async fn refresh_session(base_url: &str, refresh_token: &str) -> RefreshOutcome {
    let url = format!("{}/auth/refresh", base_url.trim_end_matches('/'));
    let resp = reqwest::Client::new()
        .post(&url)
        .json(&serde_json::json!({ "refresh_token": refresh_token }))
        .send()
        .await;
    let resp = match resp {
        Ok(r) => r,
        Err(_) => return RefreshOutcome::Transient,
    };
    match resp.status().as_u16() {
        200 => match resp.json::<PollReady>().await {
            Ok(r) => RefreshOutcome::Rotated(Session {
                access_token: r.access_token,
                refresh_token: r.refresh_token,
            }),
            Err(_) => RefreshOutcome::Transient,
        },
        401 => RefreshOutcome::Unauthorized,
        _ => RefreshOutcome::Transient,
    }
}

/// Serializes every refresh process-wide. Refresh tokens are **single-use** —
/// each refresh rotates to a new one (see [`refresh_session`]) — so two refreshes
/// racing on the same stored token make the second POST an already-consumed token
/// → `401` → the session is cleared and the user is signed out. This was the
/// "click Sync and get signed out" bug: the sync refresh raced the startup
/// `account_resync`. The lock makes refreshes run one-at-a-time, and each reloads
/// the latest stored session *inside* the lock so the rotated token is never
/// reused. (Process-local — it fixes the GUI's own concurrent paths.)
static REFRESH_LOCK: Mutex<()> = Mutex::const_new(());

/// Refresh the stored session under [`REFRESH_LOCK`], persisting the rotation
/// (`Rotated`) or clearing the dead session (`Unauthorized`). Every refresh path
/// goes through here so token rotation can't race itself.
pub async fn refresh_session_locked(base_url: &str) -> RefreshOutcome {
    let _guard = REFRESH_LOCK.lock().await;
    // Reload inside the lock: a queued caller must see the token a prior refresh
    // just rotated, not the one it read before blocking.
    let Some(session) = load_session() else {
        return RefreshOutcome::Unauthorized;
    };
    let outcome = refresh_session(base_url, &session.refresh_token).await;
    match &outcome {
        RefreshOutcome::Rotated(ns) => {
            let _ = store_session(ns);
        }
        RefreshOutcome::Unauthorized => {
            let _ = clear_session();
        }
        RefreshOutcome::Transient => {}
    }
    outcome
}

/// Return a usable access token for an authenticated API call, refreshing the
/// session first (access tokens are short-lived). `None` when not signed in or
/// the session is definitively dead (a transient failure falls back to the
/// stored access token so an offline op can still be attempted).
pub async fn access_token_refreshing(base_url: &str) -> Option<String> {
    load_session()?;
    match refresh_session_locked(base_url).await {
        RefreshOutcome::Rotated(ns) => Some(ns.access_token),
        RefreshOutcome::Unauthorized => None,
        RefreshOutcome::Transient => load_session().map(|s| s.access_token),
    }
}

/// Best-effort server-side logout (revoke the session). Failures are ignored —
/// the local keychain clear is what matters for the user.
pub async fn logout_remote(base_url: &str, access_token: &str) {
    let url = format!("{}/auth/logout", base_url.trim_end_matches('/'));
    let _ = reqwest::Client::new()
        .post(&url)
        .bearer_auth(access_token)
        .send()
        .await;
}
