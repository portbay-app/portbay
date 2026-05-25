//! Client-side Pro entitlement layer (3-tier: anonymous / free / pro).
//!
//! Verifies the Ed25519-signed entitlement document issued by PortBay Cloud
//! (`/license`, see `docs/pro/entitlements.md` §6), caches it on disk, and
//! computes the effective entitlement with an **offline grace window** so a Pro
//! user never loses access when the license server is unreachable.
//!
//! Tiers:
//! - **anonymous** — no account/token: the built-in *unsigned* fallback, cap 3.
//! - **free** — a signed `free` entitlement (cap 6); honored offline indefinitely
//!   (it grants no paid features, so there's nothing to expire).
//! - **pro** — a signed `pro` entitlement (unlimited + paid features), re-checked
//!   periodically with a grace window. A lapsed/revoked Pro falls back to **free**
//!   (the signed-in floor), never to anonymous — we never punish a downgrade.
//!
//! The signature is verified against a canonical JSON form that **must match the
//! server's `canonicalize()` byte-for-byte** (recursively sorted keys, compact,
//! top-level `sig` excluded). We rebuild that form explicitly here rather than
//! relying on `serde_json` map ordering, so it is independent of crate features.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Ed25519 public key (raw 32-byte, base64) that the issuer signs with.
/// The matching private key lives only in PortBay Cloud's `LICENSE_SIGNING_KEY`
/// Workers secret (+ gitignored `portbay-cloud/.secrets-local`).
/// **Production key, rotated 2026-05-24** (the earlier dev key is retired).
const PUBLIC_KEY_B64: &str = "LvM9qZwq1tH0gv871R1qPCTIN9WsUyeKHWcacHLKs/w=";

/// How long a freshly verified entitlement is trusted before a re-check is due.
/// Mirrors the server's `RECHECK_DAYS` default.
const RECHECK_SECS: u64 = 30 * 24 * 60 * 60;

const CACHE_FILENAME: &str = "entitlement.json";

// ---------------------------------------------------------------------------
// Wire types — mirror the §6 contract
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    /// `None` for email-auth accounts.
    pub github_id: Option<i64>,
    pub login: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entitlements {
    /// `None` = unlimited (pro); free = `Some(6)`; anonymous = `Some(3)`.
    pub max_projects: Option<u32>,
    pub sync: bool,
    pub custom_port_cors: bool,
    pub mail: String, // "limited" | "full"
    pub early_access: bool,
    pub priority_support: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntitlementPayload {
    pub schema: u32,
    pub account: Account,
    pub tier: String, // "free" | "pro" (server never signs "anonymous")
    pub source: Option<String>,
    pub issued_at: String,
    pub recheck_after: String,
    pub grace_days: i64,
    pub revoked: bool,
    pub entitlements: Entitlements,
    pub sig: String,
}

/// What the frontend consumes. `account` is absent for the anonymous fallback.
#[derive(Debug, Clone, Serialize)]
pub struct EffectiveEntitlement {
    pub state: EntitlementState,
    pub tier: String,
    pub entitlements: Entitlements,
    pub account: Option<Account>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EntitlementState {
    /// No account / no token. Cap 3.
    Anonymous,
    /// Signed-in free account. Cap 6.
    Free,
    /// Signed-in with an active Pro entitlement.
    Pro,
    /// Server unreachable, but the cached Pro entitlement is still inside its grace window.
    ProGrace,
    /// Reserved for a future refresh-failure surface.
    UnknownOffline,
}

/// On-disk cache: the raw signed document plus when we last fetched it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedEntitlement {
    /// The exact signed JSON returned by `/license` (re-verified on load).
    pub raw: String,
    /// Unix seconds at which `raw` was fetched + verified.
    pub fetched_at: u64,
}

// ---------------------------------------------------------------------------
// Tier entitlement blocks + fallbacks
// ---------------------------------------------------------------------------

pub fn anonymous_entitlements() -> Entitlements {
    Entitlements {
        max_projects: Some(3),
        sync: false,
        custom_port_cors: false,
        mail: "limited".into(),
        early_access: false,
        priority_support: false,
    }
}

pub fn free_entitlements() -> Entitlements {
    Entitlements {
        max_projects: Some(6),
        sync: false,
        custom_port_cors: false,
        mail: "limited".into(),
        early_access: false,
        priority_support: false,
    }
}

/// The built-in anonymous fallback (no account / no token). Cap 3, never signed.
pub fn anonymous_fallback() -> EffectiveEntitlement {
    EffectiveEntitlement {
        state: EntitlementState::Anonymous,
        tier: "anonymous".into(),
        entitlements: anonymous_entitlements(),
        account: None,
    }
}

/// The signed-in free floor — used when a Pro entitlement lapses or is revoked
/// but the user is still signed in. Keeps the account, drops to cap 6.
fn free_floor(account: Option<Account>) -> EffectiveEntitlement {
    EffectiveEntitlement {
        state: EntitlementState::Free,
        tier: "free".into(),
        entitlements: free_entitlements(),
        account,
    }
}

// ---------------------------------------------------------------------------
// Canonical JSON — must byte-match the server's canonicalize()
// ---------------------------------------------------------------------------

/// Recursively serialize `value` with object keys sorted, compact (no
/// whitespace). Scalars go through `serde_json` so number/string formatting
/// matches `JSON.stringify`.
fn canonical(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let parts: Vec<String> = keys
                .into_iter()
                .map(|k| {
                    let key = serde_json::to_string(k).expect("string key serializes");
                    format!("{}:{}", key, canonical(&map[k]))
                })
                .collect();
            format!("{{{}}}", parts.join(","))
        }
        Value::Array(items) => {
            let parts: Vec<String> = items.iter().map(canonical).collect();
            format!("[{}]", parts.join(","))
        }
        scalar => serde_json::to_string(scalar).expect("scalar serializes"),
    }
}

fn verifying_key() -> VerifyingKey {
    let bytes = STANDARD
        .decode(PUBLIC_KEY_B64)
        .expect("embedded public key is valid base64");
    let arr: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .expect("embedded public key is 32 bytes");
    VerifyingKey::from_bytes(&arr).expect("embedded public key is a valid Ed25519 key")
}

/// Verify a signed entitlement document. Returns the typed payload only if the
/// signature checks out against the embedded public key.
pub fn verify_signed(signed_json: &str) -> Option<EntitlementPayload> {
    let mut value: Value = serde_json::from_str(signed_json).ok()?;
    let sig_b64 = value.get("sig")?.as_str()?.to_owned();

    // Canonicalize over everything except the top-level `sig`.
    if let Value::Object(map) = &mut value {
        map.remove("sig");
    }
    let message = canonical(&value);

    let sig_bytes = URL_SAFE_NO_PAD.decode(sig_b64.as_bytes()).ok()?;
    let signature = Signature::from_slice(&sig_bytes).ok()?;
    verifying_key()
        .verify_strict(message.as_bytes(), &signature)
        .ok()?;

    serde_json::from_str::<EntitlementPayload>(signed_json).ok()
}

// ---------------------------------------------------------------------------
// Effective entitlement + offline grace
// ---------------------------------------------------------------------------

/// Compute the effective entitlement from a verified payload and the time it
/// was fetched, as of `now` (unix seconds). Pure, so it is unit-testable.
///
/// - tier `free`:
///     - revoked                                  → Anonymous (abuse → cap 3)
///     - otherwise                                → Free (honored offline)
/// - tier `pro`:
///     - revoked                                  → Free (signed-in floor, cap 6)
///     - age ≤ recheck window                     → Pro
///     - recheck < age ≤ recheck + grace_days     → ProGrace
///     - age > recheck + grace_days               → Free (signed-in floor)
pub fn effective_from(
    payload: &EntitlementPayload,
    fetched_at: u64,
    now: u64,
) -> EffectiveEntitlement {
    let account = Some(payload.account.clone());

    if payload.tier == "free" {
        if payload.revoked {
            // Revoked free account (abuse) — drop to the anonymous cap, keep identity.
            return EffectiveEntitlement {
                state: EntitlementState::Anonymous,
                tier: "anonymous".into(),
                entitlements: anonymous_entitlements(),
                account,
            };
        }
        return EffectiveEntitlement {
            state: EntitlementState::Free,
            tier: "free".into(),
            entitlements: free_entitlements(),
            account,
        };
    }

    if payload.tier != "pro" {
        // Unknown tier — be conservative.
        return free_floor(account);
    }

    if payload.revoked {
        return free_floor(account);
    }

    let age = now.saturating_sub(fetched_at);
    let grace_secs = payload.grace_days.max(0) as u64 * 24 * 60 * 60;

    let state = if age <= RECHECK_SECS {
        EntitlementState::Pro
    } else if age <= RECHECK_SECS + grace_secs {
        EntitlementState::ProGrace
    } else {
        // Grace expired — fall back to the signed-in free floor, never anonymous.
        return free_floor(account);
    };

    EffectiveEntitlement {
        state,
        tier: "pro".into(),
        entitlements: payload.entitlements.clone(),
        account,
    }
}

// ---------------------------------------------------------------------------
// Cache I/O
// ---------------------------------------------------------------------------

fn cache_path() -> std::io::Result<PathBuf> {
    let mut path = dirs::data_dir()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no data dir"))?;
    path.push("PortBay");
    std::fs::create_dir_all(&path)?;
    path.push(CACHE_FILENAME);
    Ok(path)
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Load + verify the cached entitlement, returning `(payload, fetched_at)` only
/// when the cached signature is still valid.
pub fn load_cache() -> Option<(EntitlementPayload, u64)> {
    let path = cache_path().ok()?;
    let raw = std::fs::read_to_string(path).ok()?;
    let cached: CachedEntitlement = serde_json::from_str(&raw).ok()?;
    let payload = verify_signed(&cached.raw)?;
    Some((payload, cached.fetched_at))
}

pub fn store_cache(signed_json: &str) -> std::io::Result<()> {
    let cached = CachedEntitlement {
        raw: signed_json.to_owned(),
        fetched_at: now_unix(),
    };
    let path = cache_path()?;
    std::fs::write(path, serde_json::to_string_pretty(&cached)?)
}

pub fn clear_cache() -> std::io::Result<()> {
    let path = cache_path()?;
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// The current effective entitlement from cache (no network). Falls back to
/// the anonymous tier when there is no valid cache.
pub fn current() -> EffectiveEntitlement {
    match load_cache() {
        Some((payload, fetched_at)) => effective_from(&payload, fetched_at, now_unix()),
        None => anonymous_fallback(),
    }
}

/// Defense-in-depth project-cap check, shared by the GUI `add_project` command
/// and the CLI `add`. Reads the current cached effective entitlement (anonymous
/// = 3, free = 6, pro = unlimited) and returns `Err(cap)` when adding one more
/// project would exceed it. The frontend's proactive gate is the primary UX;
/// this also covers the CLI and any non-gated path. Not DRM — it's bypassable
/// by rebuilding (see entitlements.md §1) — just an honest, consistent limit.
pub fn check_can_add(current_count: usize) -> Result<(), u32> {
    match current().entitlements.max_projects {
        Some(cap) if current_count as u32 >= cap => Err(cap),
        _ => Ok(()),
    }
}

/// Fetch a fresh signed entitlement from the issuer using the caller's bearer
/// token, verify it, and cache it. Returns the new effective entitlement.
pub async fn refresh(base_url: &str, token: &str) -> Result<EffectiveEntitlement, String> {
    let url = format!("{}/license", base_url.trim_end_matches('/'));
    let resp = reqwest::Client::new()
        .get(&url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| format!("license fetch failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("license endpoint returned {}", resp.status()));
    }
    let body = resp
        .text()
        .await
        .map_err(|e| format!("reading license body failed: {e}"))?;

    if verify_signed(&body).is_none() {
        return Err("license signature did not verify".into());
    }
    store_cache(&body).map_err(|e| format!("caching license failed: {e}"))?;
    Ok(current())
}

// ---------------------------------------------------------------------------
// Tests — cross-impl vectors produced by PortBay Cloud's own signer (dev key)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Real signed `pro` document emitted by `portbay-cloud`'s `signPayload` with
    /// the dev key (schema 2). Proves the Rust canonicalizer reproduces the
    /// server's signed bytes exactly.
    const PRO_VECTOR: &str = r#"{"schema":2,"account":{"github_id":12345,"login":"octocat"},"tier":"pro","source":"donate","issued_at":"2026-05-24T00:00:00.000Z","recheck_after":"2026-06-23T00:00:00.000Z","grace_days":21,"revoked":false,"entitlements":{"max_projects":null,"sync":true,"custom_port_cors":true,"mail":"full","early_access":true,"priority_support":true},"sig":"NZu-Qz1uDhnYfC4BwMYeNrLqUI0q-EytOst3YYAPWveVy3MhaKyXV0Vcgpu6euUw90WbL8pMoRpBEifj95AIDw"}"#;

    /// Real signed `free` document (schema 2, source "signup").
    const FREE_VECTOR: &str = r#"{"schema":2,"account":{"github_id":12345,"login":"octocat"},"tier":"free","source":"signup","issued_at":"2026-05-24T00:00:00.000Z","recheck_after":"2026-06-23T00:00:00.000Z","grace_days":21,"revoked":false,"entitlements":{"max_projects":6,"sync":false,"custom_port_cors":false,"mail":"limited","early_access":false,"priority_support":false},"sig":"WImod7ZCs7z4OlW9EH4RjLeJgXNklabSlfcrpb9Hmnd0OaNy1RQpHag9jgkayHQepyOmZk3rHnRKB9FlEPqSDQ"}"#;

    #[test]
    fn canonical_sorts_keys_recursively_and_excludes_sig() {
        let mut v: Value = serde_json::from_str(PRO_VECTOR).unwrap();
        if let Value::Object(m) = &mut v {
            m.remove("sig");
        }
        let c = canonical(&v);
        assert!(c.starts_with(
            r#"{"account":{"github_id":12345,"login":"octocat"},"entitlements":{"custom_port_cors":true,"#
        ));
        assert!(!c.contains(", "));
        assert!(!c.contains("\"sig\""));
    }

    #[test]
    fn verifies_real_server_pro_signature() {
        let payload = verify_signed(PRO_VECTOR).expect("server pro vector must verify");
        assert_eq!(payload.schema, 2);
        assert_eq!(payload.tier, "pro");
        assert_eq!(payload.account.login, "octocat");
        assert_eq!(payload.account.github_id, Some(12345));
        assert_eq!(payload.entitlements.max_projects, None);
        assert!(payload.entitlements.sync);
    }

    #[test]
    fn verifies_real_server_free_signature() {
        let payload = verify_signed(FREE_VECTOR).expect("server free vector must verify");
        assert_eq!(payload.tier, "free");
        assert_eq!(payload.source.as_deref(), Some("signup"));
        assert_eq!(payload.entitlements.max_projects, Some(6));
        assert!(!payload.entitlements.sync);
    }

    #[test]
    fn rejects_tampered_payload() {
        let tampered = PRO_VECTOR.replace("\"max_projects\":null", "\"max_projects\":999");
        assert!(verify_signed(&tampered).is_none());
    }

    #[test]
    fn rejects_tampered_signature() {
        let tampered = PRO_VECTOR.replace("NZu-Qz", "NZu-Qa");
        assert!(verify_signed(&tampered).is_none());
    }

    fn pro_payload() -> EntitlementPayload {
        verify_signed(PRO_VECTOR).unwrap()
    }

    #[test]
    fn no_cache_is_anonymous() {
        let e = anonymous_fallback();
        assert_eq!(e.state, EntitlementState::Anonymous);
        assert_eq!(e.entitlements.max_projects, Some(3));
        assert!(e.account.is_none());
    }

    #[test]
    fn signed_free_is_free() {
        let p = verify_signed(FREE_VECTOR).unwrap();
        let e = effective_from(&p, 1000, 1000 + 10 * 24 * 60 * 60);
        assert_eq!(e.state, EntitlementState::Free);
        assert_eq!(e.entitlements.max_projects, Some(6));
        assert!(e.account.is_some());
    }

    #[test]
    fn signed_free_honored_offline_regardless_of_age() {
        let p = verify_signed(FREE_VECTOR).unwrap();
        // Far past any recheck window — free still holds.
        let e = effective_from(&p, 1000, 1000 + 400 * 24 * 60 * 60);
        assert_eq!(e.state, EntitlementState::Free);
        assert_eq!(e.entitlements.max_projects, Some(6));
    }

    #[test]
    fn fresh_pro_is_pro() {
        let p = pro_payload();
        let e = effective_from(&p, 1000, 1000 + 10 * 24 * 60 * 60); // 10 days old
        assert_eq!(e.state, EntitlementState::Pro);
        assert_eq!(e.tier, "pro");
        assert_eq!(e.entitlements.max_projects, None);
    }

    #[test]
    fn stale_pro_within_grace_is_pro_grace() {
        let p = pro_payload();
        let now = 1000 + RECHECK_SECS + 5 * 24 * 60 * 60; // 5 days past recheck, grace is 21
        let e = effective_from(&p, 1000, now);
        assert_eq!(e.state, EntitlementState::ProGrace);
        assert_eq!(e.tier, "pro"); // still entitled during grace
    }

    #[test]
    fn expired_grace_falls_back_to_free_not_anonymous() {
        let p = pro_payload();
        let now = 1000 + RECHECK_SECS + 30 * 24 * 60 * 60; // past recheck + 21d grace
        let e = effective_from(&p, 1000, now);
        // Signed-in floor is Free(6), never Anonymous(3).
        assert_eq!(e.state, EntitlementState::Free);
        assert_eq!(e.entitlements.max_projects, Some(6));
        assert!(!e.entitlements.sync);
        assert!(e.account.is_some());
    }

    #[test]
    fn revoked_pro_falls_back_to_free() {
        let mut p = pro_payload();
        p.revoked = true;
        let e = effective_from(&p, 1000, 1001);
        assert_eq!(e.state, EntitlementState::Free);
        assert_eq!(e.entitlements.max_projects, Some(6));
    }

    #[test]
    fn revoked_free_falls_back_to_anonymous() {
        let mut p = verify_signed(FREE_VECTOR).unwrap();
        p.revoked = true;
        let e = effective_from(&p, 1000, 1001);
        assert_eq!(e.state, EntitlementState::Anonymous);
        assert_eq!(e.entitlements.max_projects, Some(3));
    }
}
