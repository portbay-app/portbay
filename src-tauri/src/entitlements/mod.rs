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

/// Fallback re-check window when an old or malformed entitlement lacks a
/// parseable `recheck_after`. Fresh server documents carry their own signed
/// absolute re-check timestamp, which is authoritative.
const DEFAULT_RECHECK_SECS: u64 = 30 * 24 * 60 * 60;

const CACHE_FILENAME: &str = "entitlement.json";

// ---------------------------------------------------------------------------
// Wire types — mirror the §6 contract
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    /// `None` for email-auth accounts.
    pub github_id: Option<i64>,
    pub login: String,
    /// User-set display name (schema ≥ 3); the source of the avatar initials.
    /// Absent on schema-2 docs — `default` maps a missing key to `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Canonical avatar to fetch — a custom upload (this issuer's `/avatar/{id}`)
    /// or the GitHub photo, resolved server-side (schema ≥ 3). Absent on schema-2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entitlements {
    /// `None` = unlimited (pro); free = `Some(6)`; anonymous = `Some(3)`.
    pub max_projects: Option<u32>,
    /// License activation cap (schema ≥ 3): how many devices may hold this
    /// entitlement at once. Pro = 2, free/anonymous = 1. `None` on a legacy
    /// schema-2 document (no `max_devices` key) — treated as unbounded so a
    /// pre-v3 cached Pro license never bricks. The server is authoritative for
    /// the *actual* device count (`/devices/register`); this is only the
    /// advertised cap the UI surfaces.
    #[serde(default)]
    pub max_devices: Option<u32>,
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
    /// How the entitlement was acquired (`"subscription"`, `"donate"`,
    /// `"contribute"`, `"manual"`, `"signup"`, …) — straight from the signed
    /// document. `None` for the synthetic states (anonymous fallback, free
    /// floor after a lapsed/revoked Pro). The UI uses it to decide whether
    /// there is a subscription to manage (billing portal) vs a perpetual grant.
    pub source: Option<String>,
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
        max_devices: Some(1),
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
        max_devices: Some(1),
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
        source: None,
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
        // Synthetic floor, not the signed document's tier — no source.
        source: None,
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
                source: None,
                entitlements: anonymous_entitlements(),
                account,
            };
        }
        return EffectiveEntitlement {
            state: EntitlementState::Free,
            tier: "free".into(),
            source: payload.source.clone(),
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
    let recheck_secs = recheck_window_secs(payload, fetched_at);
    let grace_secs = payload.grace_days.max(0) as u64 * 24 * 60 * 60;

    let state = if age <= recheck_secs {
        EntitlementState::Pro
    } else if age <= recheck_secs.saturating_add(grace_secs) {
        EntitlementState::ProGrace
    } else {
        // Grace expired — fall back to the signed-in free floor, never anonymous.
        return free_floor(account);
    };

    EffectiveEntitlement {
        state,
        tier: "pro".into(),
        source: payload.source.clone(),
        entitlements: payload.entitlements.clone(),
        account,
    }
}

fn recheck_window_secs(payload: &EntitlementPayload, fetched_at: u64) -> u64 {
    chrono::DateTime::parse_from_rfc3339(&payload.recheck_after)
        .ok()
        .and_then(|dt| u64::try_from(dt.timestamp()).ok())
        .map(|recheck_at| recheck_at.saturating_sub(fetched_at))
        .unwrap_or(DEFAULT_RECHECK_SECS)
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
    let body = serde_json::to_string_pretty(&cached)?;

    // The cached signed payload carries account PII (login, github_id, tier).
    // Tampering still fails signature verification, but the file must not be
    // world-readable on a shared machine, so write it 0600. On unix, create
    // with restrictive mode up front (and chmod any pre-existing 0644 file from
    // an older build); elsewhere fall back to a plain write.
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        // Repair permissions on a file left behind by an older build.
        if path.exists() {
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)?;
        f.write_all(body.as_bytes())
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, body)
    }
}

pub fn clear_cache() -> std::io::Result<()> {
    let path = cache_path()?;
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Whether the current effective entitlement grants Pro (including the offline
/// grace window). The client-side gate for Pro-only features whose entitlement
/// isn't a dedicated boolean in the signed block (e.g. custom tunnel).
pub fn is_pro() -> bool {
    matches!(
        current().state,
        EntitlementState::Pro | EntitlementState::ProGrace
    )
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

/// Community sandbox allowance. Both the anonymous and signed-in free tiers may
/// have Sandboxed Run enabled on up to this many projects at once, so the
/// feature is usable without Pro; Pro lifts the cap entirely.
pub const SANDBOX_COMMUNITY_CAP: u32 = 2;

/// Per-file size cap for task-board attachments on the community tiers
/// (anonymous and signed-in free): 10 MB.
pub const TASK_ATTACHMENT_COMMUNITY_CAP_BYTES: u64 = 10 * 1024 * 1024;

/// Per-file size cap for task-board attachments on Pro: 250 MB.
pub const TASK_ATTACHMENT_PRO_CAP_BYTES: u64 = 250 * 1024 * 1024;

impl Entitlements {
    /// How many projects may have Sandboxed Run enabled at once. `None` =
    /// unlimited (Pro). Tied to the same "unlimited projects" signal as
    /// [`Entitlements::max_projects`], so it tracks the paid tier without
    /// needing a new field in the signed entitlement document — the community
    /// cap is enforced client-side, consistent with the project cap (not DRM).
    pub fn max_sandbox_projects(&self) -> Option<u32> {
        // A capped project tier (anonymous/free) ⇒ the community sandbox cap;
        // unlimited projects (Pro) ⇒ unlimited sandboxed runs.
        self.max_projects.map(|_| SANDBOX_COMMUNITY_CAP)
    }

    /// Per-file size cap for task-board attachments. Tied to the same
    /// "unlimited projects" signal as [`Entitlements::max_projects`] — a
    /// capped project tier (anonymous/free) gets the 10 MB community cap,
    /// Pro gets the 250 MB cap — so it tracks the paid tier without needing
    /// a new field in the signed entitlement document. Enforced client-side,
    /// consistent with the project cap (not DRM).
    pub fn max_task_attachment_bytes(&self) -> u64 {
        match self.max_projects {
            Some(_) => TASK_ATTACHMENT_COMMUNITY_CAP_BYTES,
            None => TASK_ATTACHMENT_PRO_CAP_BYTES,
        }
    }
}

/// Defense-in-depth sandbox-cap check, mirroring [`check_can_add`].
/// `current_count` is how many *other* projects already have Sandboxed Run
/// enabled; returns `Err(cap)` when enabling one more would exceed the tier's
/// allowance (Pro is uncapped). Like the project cap, this is an honest limit,
/// not DRM — it's bypassable by rebuilding.
pub fn check_can_sandbox(current_count: usize) -> Result<(), u32> {
    match current().entitlements.max_sandbox_projects() {
        Some(cap) if current_count as u32 >= cap => Err(cap),
        _ => Ok(()),
    }
}

/// Defense-in-depth attachment-size check, mirroring [`check_can_add`].
/// Returns `Err(cap_bytes)` when a `size_bytes` task-board attachment exceeds
/// the current tier's per-file cap (community 10 MB, Pro 250 MB). Like the
/// project cap, an honest limit, not DRM — it's bypassable by rebuilding.
pub fn check_attachment_size(size_bytes: u64) -> Result<(), u64> {
    let cap = current().entitlements.max_task_attachment_bytes();
    if size_bytes > cap {
        Err(cap)
    } else {
        Ok(())
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

    /// Real signed **schema-3 subscription** document emitted by `portbay-cloud`'s
    /// `signPayload` with the prod key — the cross-impl guard for the `max_devices`
    /// field. Generated by `portbay-cloud/scripts/sign-test-vector.mjs`. If the
    /// canonicalizers ever drift over the new field, this signature stops
    /// verifying and this test fails before any client ships a broken v3 read.
    const PRO_V3_SUBSCRIPTION_VECTOR: &str = r#"{"account":{"github_id":12345,"login":"octocat"},"entitlements":{"custom_port_cors":true,"early_access":true,"mail":"full","max_devices":2,"max_projects":null,"priority_support":true,"sync":true},"grace_days":21,"issued_at":"2026-05-28T00:00:00.000Z","recheck_after":"2026-06-27T00:00:00.000Z","revoked":false,"schema":3,"sig":"FiiQgN4lnd1UyEY1urhQjt-EVTGaFkBdxhA0ALTCHWJdQqzO7yMnaxcT7t3JS95bft0MBqMKJgtW9YCX49lfAA","source":"subscription","tier":"pro"}"#;

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
    fn verifies_real_server_v3_subscription_signature() {
        // The cross-impl signature-parity guard: a schema-3 doc carrying the new
        // `max_devices` field, signed by the cloud signer, must verify here.
        let payload = verify_signed(PRO_V3_SUBSCRIPTION_VECTOR)
            .expect("server schema-3 subscription vector must verify");
        assert_eq!(payload.schema, 3);
        assert_eq!(payload.tier, "pro");
        assert_eq!(payload.source.as_deref(), Some("subscription"));
        assert_eq!(payload.entitlements.max_devices, Some(2));
        assert_eq!(payload.entitlements.max_projects, None);
        assert!(payload.entitlements.sync);
    }

    #[test]
    fn schema2_doc_defaults_max_devices_to_none() {
        // A legacy schema-2 doc has no `max_devices` key — serde default → None,
        // and it must still verify + parse (back-compat with cached v2 licenses).
        let payload = verify_signed(PRO_VECTOR).expect("schema-2 pro vector must verify");
        assert_eq!(payload.entitlements.max_devices, None);
    }

    #[test]
    fn rejects_tampered_v3_max_devices() {
        // Tampering with the new field must break the signature.
        let tampered = PRO_V3_SUBSCRIPTION_VECTOR.replace("\"max_devices\":2", "\"max_devices\":9");
        assert!(verify_signed(&tampered).is_none());
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

    #[test]
    fn canonical_handles_schema3_account_fields() {
        // The schema-3 `account` carries optional `display_name` + `avatar_url`.
        // Canonical form must sort the account keys (avatar_url < display_name <
        // github_id < login) byte-identically to the cloud's canonicalize() — this
        // is the cross-impl guard that signatures over schema-3 docs still verify.
        let doc = r#"{"schema":3,"account":{"login":"octocat","github_id":12345,"display_name":"The Octocat","avatar_url":"https://cloud.portbay.app/avatar/1?v=x"},"tier":"free"}"#;
        let v: Value = serde_json::from_str(doc).unwrap();
        assert_eq!(
            canonical(&v),
            r#"{"account":{"avatar_url":"https://cloud.portbay.app/avatar/1?v=x","display_name":"The Octocat","github_id":12345,"login":"octocat"},"schema":3,"tier":"free"}"#
        );
    }

    #[test]
    fn account_parses_schema3_fields_and_defaults_schema2() {
        // Schema-3 account fields deserialize…
        let a3: Account = serde_json::from_str(
            r#"{"github_id":1,"login":"octocat","display_name":"The Octocat","avatar_url":"https://x/a"}"#,
        )
        .unwrap();
        assert_eq!(a3.display_name.as_deref(), Some("The Octocat"));
        assert_eq!(a3.avatar_url.as_deref(), Some("https://x/a"));
        // …and a schema-2 account (no such keys) defaults them to None.
        let a2: Account = serde_json::from_str(r#"{"github_id":1,"login":"octocat"}"#).unwrap();
        assert!(a2.display_name.is_none());
        assert!(a2.avatar_url.is_none());
    }

    fn pro_payload() -> EntitlementPayload {
        verify_signed(PRO_VECTOR).unwrap()
    }

    fn unix(s: &str) -> u64 {
        u64::try_from(chrono::DateTime::parse_from_rfc3339(s).unwrap().timestamp()).unwrap()
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
        // The signed document's source passes through — the UI keys billing
        // management ("subscription") vs perpetual grants ("donate", …) on it.
        assert_eq!(e.source.as_deref(), Some("donate"));
    }

    #[test]
    fn subscription_pro_surfaces_subscription_source() {
        let p = verify_signed(PRO_V3_SUBSCRIPTION_VECTOR).unwrap();
        let e = effective_from(&p, 1000, 1001);
        assert_eq!(e.state, EntitlementState::Pro);
        assert_eq!(e.source.as_deref(), Some("subscription"));
    }

    #[test]
    fn synthetic_states_carry_no_source() {
        // Anonymous fallback…
        assert!(anonymous_fallback().source.is_none());
        // …and the free floor after a lapsed Pro (synthetic, not the signed tier).
        let p = pro_payload();
        let fetched_at = unix("2026-05-24T00:00:00Z");
        let now = unix("2026-07-23T00:00:00Z"); // past recheck + grace
        let e = effective_from(&p, fetched_at, now);
        assert_eq!(e.state, EntitlementState::Free);
        assert!(e.source.is_none());
    }

    #[test]
    fn stale_pro_within_grace_is_pro_grace() {
        let p = pro_payload();
        let fetched_at = unix("2026-05-24T00:00:00Z");
        let now = unix("2026-06-28T00:00:00Z"); // 5 days past signed recheck
        let e = effective_from(&p, fetched_at, now);
        assert_eq!(e.state, EntitlementState::ProGrace);
        assert_eq!(e.tier, "pro"); // still entitled during grace
    }

    #[test]
    fn pro_recheck_uses_signed_recheck_after_not_hardcoded_window() {
        let mut p = pro_payload();
        p.recheck_after = "2026-05-31T00:00:00.000Z".into();
        let fetched_at = unix("2026-05-24T00:00:00Z");
        let now = unix("2026-06-04T00:00:00Z"); // 4 days past signed recheck
        let e = effective_from(&p, fetched_at, now);
        assert_eq!(e.state, EntitlementState::ProGrace);
    }

    #[test]
    fn malformed_recheck_after_falls_back_to_default_window() {
        let mut p = pro_payload();
        p.recheck_after = "not-a-date".into();
        let now = 1000 + DEFAULT_RECHECK_SECS + 5 * 24 * 60 * 60;
        let e = effective_from(&p, 1000, now);
        assert_eq!(e.state, EntitlementState::ProGrace);
    }

    #[test]
    fn expired_grace_falls_back_to_free_not_anonymous() {
        let p = pro_payload();
        let fetched_at = unix("2026-05-24T00:00:00Z");
        let now = unix("2026-07-23T00:00:00Z"); // past signed recheck + grace
        let e = effective_from(&p, fetched_at, now);
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

    #[test]
    fn community_tiers_get_10mb_attachments_pro_250mb() {
        // Anonymous and free share the 10 MB per-file attachment cap …
        assert_eq!(
            anonymous_entitlements().max_task_attachment_bytes(),
            TASK_ATTACHMENT_COMMUNITY_CAP_BYTES
        );
        assert_eq!(
            free_entitlements().max_task_attachment_bytes(),
            TASK_ATTACHMENT_COMMUNITY_CAP_BYTES
        );
        // … and Pro (unlimited projects) gets the 250 MB cap.
        let pro = verify_signed(PRO_VECTOR).unwrap().entitlements;
        assert_eq!(
            pro.max_task_attachment_bytes(),
            TASK_ATTACHMENT_PRO_CAP_BYTES
        );
        // Sanity on the advertised numbers themselves.
        assert_eq!(TASK_ATTACHMENT_COMMUNITY_CAP_BYTES, 10 * 1024 * 1024);
        assert_eq!(TASK_ATTACHMENT_PRO_CAP_BYTES, 250 * 1024 * 1024);
    }

    #[test]
    fn community_tiers_get_two_sandbox_projects_pro_unlimited() {
        // Anonymous and free both get the community cap …
        assert_eq!(
            anonymous_entitlements().max_sandbox_projects(),
            Some(SANDBOX_COMMUNITY_CAP)
        );
        assert_eq!(
            free_entitlements().max_sandbox_projects(),
            Some(SANDBOX_COMMUNITY_CAP)
        );
        // … and Pro (unlimited projects) is uncapped for sandboxed runs too.
        let pro = verify_signed(PRO_VECTOR).unwrap().entitlements;
        assert_eq!(pro.max_projects, None);
        assert_eq!(pro.max_sandbox_projects(), None);
    }
}
