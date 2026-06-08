//! Pro entitlement commands — the IPC surface over `crate::entitlements`.
//!
//! `get_entitlement` is the read every gate ultimately calls (via the Svelte
//! store). `refresh_entitlement` is invoked after GitHub login with the user's
//! bearer token (see the client-login card); `clear_entitlement` runs on logout.

use serde::Serialize;

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

    map_checkout_status(resp.status().as_u16())?;

    let body = resp
        .json::<serde_json::Value>()
        .await
        .map_err(|e| AppError::Internal(format!("reading checkout response failed: {e}")))?;
    extract_checkout_url(&body)
}

/// Map an HTTP status code from the checkout endpoint to an error, or `Ok(())`
/// when the response should be processed further. Extracted so the mapping
/// rules are unit-testable without a real HTTP stack.
pub(crate) fn map_checkout_status(status: u16) -> AppResult<()> {
    if status == 503 {
        return Err(AppError::Internal(
            "Pro checkout isn't available yet — please try again soon.".into(),
        ));
    }
    // reqwest's `is_success` covers 200-299.
    if !(200..300).contains(&status) {
        return Err(AppError::Internal(format!(
            "checkout request returned {status}"
        )));
    }
    Ok(())
}

/// Extract the `checkout_url` string from a successful checkout JSON body.
pub(crate) fn extract_checkout_url(body: &serde_json::Value) -> AppResult<String> {
    body.get("checkout_url")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| AppError::Internal("checkout response had no URL".into()))
}

// ---------------------------------------------------------------------------
// Billing management (subscription status + Paddle customer portal)
// ---------------------------------------------------------------------------

/// The user's paid-subscription state as the issuer knows it (webhook-fed).
/// `None` from [`subscription_status`] means the account never subscribed —
/// Pro acquired via donate/contribute/manual has no subscription to manage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionStatus {
    /// `trialing` | `active` | `past_due` | `canceled` | `refunded`
    /// (server vocabulary; `trialing` = free trial, first charge at
    /// `current_period_end`).
    pub status: String,
    /// ISO timestamp the current paid period ends — the renewal date, or the
    /// access-until date when `cancel_at_period_end` is set.
    pub current_period_end: Option<String>,
    /// A cancellation is scheduled; Pro stays active until `current_period_end`.
    pub cancel_at_period_end: bool,
}

/// Fresh, short-lived Paddle customer-portal links. Auto-authenticating —
/// never cache, persist, or log them; request a new set per click.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BillingPortalUrls {
    /// Portal home: payment method, invoices/receipts, cancel.
    pub overview_url: String,
    /// Deep link to the pre-opened cancel form (end-of-period).
    pub cancel_url: Option<String>,
    /// Deep link to the pre-opened update-payment-method form.
    pub update_payment_url: Option<String>,
}

/// Current subscription state from the issuer (`GET /account/subscription`).
/// Requires a signed-in session; returns `None` when the account has no
/// subscription row (e.g. Pro from a contribution).
#[tauri::command]
pub async fn subscription_status() -> AppResult<Option<SubscriptionStatus>> {
    let token = auth::access_token_refreshing(CLOUD_BASE_URL)
        .await
        .ok_or_else(|| AppError::Internal("Sign in to PortBay to view billing.".into()))?;

    let url = format!(
        "{}/account/subscription",
        CLOUD_BASE_URL.trim_end_matches('/')
    );
    let resp = reqwest::Client::new()
        .get(&url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("subscription request failed: {e}")))?;

    if !resp.status().is_success() {
        return Err(AppError::Internal(format!(
            "subscription request returned {}",
            resp.status().as_u16()
        )));
    }

    let body = resp
        .json::<serde_json::Value>()
        .await
        .map_err(|e| AppError::Internal(format!("reading subscription response failed: {e}")))?;
    Ok(extract_subscription(&body))
}

/// Create a fresh Paddle customer-portal session and return its URLs for the
/// frontend to open in the system browser (`POST /account/portal`).
#[tauri::command]
pub async fn billing_portal_url() -> AppResult<BillingPortalUrls> {
    let token = auth::access_token_refreshing(CLOUD_BASE_URL)
        .await
        .ok_or_else(|| AppError::Internal("Sign in to PortBay to manage billing.".into()))?;

    let url = format!("{}/account/portal", CLOUD_BASE_URL.trim_end_matches('/'));
    let resp = reqwest::Client::new()
        .post(&url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("billing portal request failed: {e}")))?;

    map_portal_status(resp.status().as_u16())?;

    let body = resp
        .json::<serde_json::Value>()
        .await
        .map_err(|e| AppError::Internal(format!("reading portal response failed: {e}")))?;
    extract_portal_urls(&body)
}

/// Map an HTTP status from the portal endpoint to an error, or `Ok(())` when
/// the response should be processed further. Mirrors [`map_checkout_status`].
pub(crate) fn map_portal_status(status: u16) -> AppResult<()> {
    match status {
        503 => Err(AppError::Internal(
            "Billing management isn't available yet — please try again soon.".into(),
        )),
        404 => Err(AppError::Internal(
            "No subscription to manage on this account.".into(),
        )),
        s if !(200..300).contains(&s) => {
            Err(AppError::Internal(format!("portal request returned {s}")))
        }
        _ => Ok(()),
    }
}

/// Parse the issuer's `GET /account/subscription` body. `subscription: null`
/// (or a malformed body) maps to `None`; a missing `status` discards the row
/// rather than inventing one.
pub(crate) fn extract_subscription(body: &serde_json::Value) -> Option<SubscriptionStatus> {
    let sub = body.get("subscription")?;
    if sub.is_null() {
        return None;
    }
    let status = sub.get("status")?.as_str()?.to_string();
    Some(SubscriptionStatus {
        status,
        current_period_end: sub
            .get("current_period_end")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        cancel_at_period_end: sub
            .get("cancel_at_period_end")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    })
}

/// Parse the issuer's `POST /account/portal` body into typed URLs. The
/// overview URL is required; the deep links are optional.
pub(crate) fn extract_portal_urls(body: &serde_json::Value) -> AppResult<BillingPortalUrls> {
    let overview_url = body
        .get("overview_url")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| AppError::Internal("portal response had no URL".into()))?;
    let opt = |key: &str| {
        body.get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    };
    Ok(BillingPortalUrls {
        overview_url,
        cancel_url: opt("cancel_url"),
        update_payment_url: opt("update_payment_url"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── map_checkout_status ──────────────────────────────────────────────────

    #[test]
    fn status_200_is_ok() {
        assert!(map_checkout_status(200).is_ok());
    }

    #[test]
    fn status_201_is_ok() {
        assert!(map_checkout_status(201).is_ok());
    }

    #[test]
    fn status_503_emits_not_available_message() {
        let err = map_checkout_status(503).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("available yet"),
            "503 should say not available: {msg}"
        );
    }

    #[test]
    fn status_401_emits_generic_status_message() {
        let err = map_checkout_status(401).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("401"), "should include status code: {msg}");
    }

    #[test]
    fn status_500_emits_generic_status_message() {
        let err = map_checkout_status(500).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("500"), "should include status code: {msg}");
    }

    #[test]
    fn status_400_emits_generic_status_message() {
        let err = map_checkout_status(400).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("400"), "should include status code: {msg}");
    }

    // ── extract_checkout_url ─────────────────────────────────────────────────

    #[test]
    fn present_url_is_returned() {
        let body = serde_json::json!({ "checkout_url": "https://checkout.paddle.com/xyz" });
        let url = extract_checkout_url(&body).unwrap();
        assert_eq!(url, "https://checkout.paddle.com/xyz");
    }

    #[test]
    fn missing_checkout_url_key_is_an_error() {
        let body = serde_json::json!({ "session_id": "sess_abc" });
        let err = extract_checkout_url(&body).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("no URL"), "got: {msg}");
    }

    #[test]
    fn non_string_checkout_url_is_an_error() {
        // If the backend ever accidentally sends a number instead of a string.
        let body = serde_json::json!({ "checkout_url": 12345 });
        let err = extract_checkout_url(&body).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("no URL"), "got: {msg}");
    }

    #[test]
    fn null_checkout_url_is_an_error() {
        let body = serde_json::json!({ "checkout_url": null });
        let err = extract_checkout_url(&body).unwrap_err();
        assert!(err.to_string().contains("no URL"));
    }

    // ── map_portal_status ─────────────────────────────────────────────────────

    #[test]
    fn portal_status_200_is_ok() {
        assert!(map_portal_status(200).is_ok());
    }

    #[test]
    fn portal_status_503_emits_not_available_message() {
        let msg = map_portal_status(503).unwrap_err().to_string();
        assert!(msg.contains("available yet"), "got: {msg}");
    }

    #[test]
    fn portal_status_404_emits_no_subscription_message() {
        let msg = map_portal_status(404).unwrap_err().to_string();
        assert!(msg.contains("No subscription"), "got: {msg}");
    }

    #[test]
    fn portal_status_500_emits_generic_status_message() {
        let msg = map_portal_status(500).unwrap_err().to_string();
        assert!(msg.contains("500"), "got: {msg}");
    }

    // ── extract_subscription ──────────────────────────────────────────────────

    #[test]
    fn null_subscription_maps_to_none() {
        let body = serde_json::json!({ "subscription": null });
        assert_eq!(extract_subscription(&body), None);
    }

    #[test]
    fn missing_subscription_key_maps_to_none() {
        let body = serde_json::json!({ "something_else": 1 });
        assert_eq!(extract_subscription(&body), None);
    }

    #[test]
    fn active_subscription_is_parsed() {
        let body = serde_json::json!({ "subscription": {
            "status": "active",
            "current_period_end": "2026-07-04T00:00:00.000Z",
            "cancel_at_period_end": false
        }});
        let sub = extract_subscription(&body).expect("parses");
        assert_eq!(sub.status, "active");
        assert_eq!(
            sub.current_period_end.as_deref(),
            Some("2026-07-04T00:00:00.000Z")
        );
        assert!(!sub.cancel_at_period_end);
    }

    #[test]
    fn scheduled_cancel_is_parsed() {
        let body = serde_json::json!({ "subscription": {
            "status": "active",
            "current_period_end": "2026-07-04T00:00:00.000Z",
            "cancel_at_period_end": true
        }});
        let sub = extract_subscription(&body).expect("parses");
        assert!(sub.cancel_at_period_end);
    }

    #[test]
    fn subscription_without_status_is_discarded() {
        let body = serde_json::json!({ "subscription": { "cancel_at_period_end": true } });
        assert_eq!(extract_subscription(&body), None);
    }

    #[test]
    fn subscription_status_serializes_camel_case() {
        // The frontend consumes camelCase — a rename regression would silently
        // break the billing block's renewal/cancel display.
        let v = serde_json::to_value(SubscriptionStatus {
            status: "active".into(),
            current_period_end: Some("2026-07-04T00:00:00.000Z".into()),
            cancel_at_period_end: true,
        })
        .unwrap();
        assert!(v.get("currentPeriodEnd").is_some());
        assert!(v.get("cancelAtPeriodEnd").is_some());
        assert!(v.get("current_period_end").is_none());
    }

    // ── extract_portal_urls ───────────────────────────────────────────────────

    #[test]
    fn portal_urls_are_parsed() {
        let body = serde_json::json!({
            "overview_url": "https://customer-portal.paddle.com/cpl_x",
            "cancel_url": "https://customer-portal.paddle.com/cpl_x/cancel",
            "update_payment_url": "https://customer-portal.paddle.com/cpl_x/pay"
        });
        let urls = extract_portal_urls(&body).unwrap();
        assert_eq!(
            urls.overview_url,
            "https://customer-portal.paddle.com/cpl_x"
        );
        assert!(urls.cancel_url.as_deref().unwrap().ends_with("/cancel"));
        assert!(urls
            .update_payment_url
            .as_deref()
            .unwrap()
            .ends_with("/pay"));
    }

    #[test]
    fn portal_deep_links_are_optional() {
        let body = serde_json::json!({ "overview_url": "https://p/x" });
        let urls = extract_portal_urls(&body).unwrap();
        assert!(urls.cancel_url.is_none());
        assert!(urls.update_payment_url.is_none());
    }

    #[test]
    fn missing_overview_url_is_an_error() {
        let body = serde_json::json!({ "cancel_url": "https://p/x/cancel" });
        let msg = extract_portal_urls(&body).unwrap_err().to_string();
        assert!(msg.contains("no URL"), "got: {msg}");
    }

    #[test]
    fn portal_urls_serialize_camel_case() {
        let v = serde_json::to_value(BillingPortalUrls {
            overview_url: "https://p/x".into(),
            cancel_url: None,
            update_payment_url: None,
        })
        .unwrap();
        assert!(v.get("overviewUrl").is_some());
        assert!(v.get("updatePaymentUrl").is_some());
        assert!(v.get("overview_url").is_none());
    }

    // ── checkout URL construction ─────────────────────────────────────────────

    #[test]
    fn checkout_url_has_no_double_slash() {
        // CLOUD_BASE_URL may or may not have a trailing slash; trim_end_matches
        // must eliminate the double-slash regardless.
        let base_with_slash = "https://api.portbay.app/";
        let url = format!("{}/account/checkout", base_with_slash.trim_end_matches('/'));
        assert!(!url.contains("//account"), "double slash: {url}");
        assert_eq!(url, "https://api.portbay.app/account/checkout");
    }

    #[test]
    fn checkout_url_without_trailing_slash_is_correct() {
        let base = "https://api.portbay.app";
        let url = format!("{}/account/checkout", base.trim_end_matches('/'));
        assert_eq!(url, "https://api.portbay.app/account/checkout");
    }
}
