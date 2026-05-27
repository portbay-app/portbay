//! Signed managed-runtime manifest: schema + signature verification.
//!
//! The list of downloadable runtimes lives in a JSON manifest hosted alongside
//! the release artifacts. A downloaded runtime runs **as the user** (there is no
//! process-boundary sandbox), so a tampered download would execute with the
//! user's privileges. The manifest is therefore signed with the **same minisign
//! key as the app updater** (`tauri.conf.json > plugins.updater.pubkey`): a
//! runtime PortBay serves is only ever as trusted as an app update.
//!
//! This module is the verify-and-parse gate — pure, no network, no app handle —
//! that every download must pass before the fetch/extract manager (a later
//! slice) touches the network.

use minisign_verify::{PublicKey, Signature};
use serde::{Deserialize, Serialize};

/// How a runtime archive is compressed. The download manager picks the
/// decompressor from this field, so the CI build can choose per-runtime without
/// a client change. (zstd vs xz as the default is a build-side decision; the
/// client understands both.)
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Compression {
    Zstd,
    Xz,
}

/// One downloadable runtime build, pinned to a single architecture. Archives are
/// arch-specific (no fat universal) to keep each download lean.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeEntry {
    /// Language id matching `LanguageRuntime::id` (e.g. "php"; "nginx"/"apache"
    /// once the web-server runtimes ship).
    pub lang: String,
    /// Full semantic version of the build, e.g. "8.3.14".
    pub version: String,
    /// Target architecture: "aarch64" or "x86_64" (see [`current_arch`]).
    pub arch: String,
    /// HTTPS URL of the compressed archive.
    pub url: String,
    /// Lowercase-hex SHA-256 of the **compressed archive as hosted**. The
    /// download manager verifies this after fetching, before extraction.
    pub sha256: String,
    /// Size of the archive download in bytes (drives the progress bar and a
    /// cheap pre-extract sanity check).
    pub size: u64,
    /// Compression codec of the archive.
    pub compression: Compression,
}

/// The signed catalogue of downloadable runtimes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeManifest {
    /// Bumped on an incompatible schema change. A manifest declaring a higher
    /// major than the client understands is rejected rather than half-parsed.
    pub schema_version: u32,
    /// RFC-3339 timestamp the manifest was generated (provenance/debugging).
    pub generated_at: String,
    pub entries: Vec<RuntimeEntry>,
}

/// The schema version this client understands.
pub const SUPPORTED_SCHEMA_VERSION: u32 = 1;

/// Errors from verifying or parsing a runtime manifest. Every variant means
/// "do not trust this manifest" — the caller must abort the download.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("manifest public key is malformed: {0}")]
    BadPublicKey(String),
    #[error("manifest signature is malformed: {0}")]
    BadSignature(String),
    #[error("manifest signature verification failed: {0}")]
    SignatureMismatch(String),
    #[error("manifest JSON is invalid: {0}")]
    BadJson(String),
    #[error("manifest schema version {found} is newer than supported {supported}")]
    UnsupportedSchema { found: u32, supported: u32 },
}

/// The architecture string the manifest uses for the running build. Matches the
/// `arch` field convention ("aarch64" / "x86_64").
pub fn current_arch() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "x86_64"
    }
}

/// Verify `manifest_bytes` against `signature` using `pubkey`, then parse it.
///
/// Signature is checked **before** the JSON is parsed: untrusted bytes never
/// reach `serde_json` until the signature proves they're ours.
pub fn verify_and_parse(
    manifest_bytes: &[u8],
    signature: &str,
    pubkey: &str,
) -> Result<RuntimeManifest, ManifestError> {
    verify_signature(manifest_bytes, signature, pubkey)?;
    parse(manifest_bytes)
}

/// Verify a detached minisign signature over `manifest_bytes`.
///
/// `pubkey` and `signature` are in the **Tauri-wrapped** form — base64 of the
/// whole minisign file — exactly as `tauri.conf.json` stores the updater pubkey
/// and as `tauri signer sign` emits a signature. We unwrap that base64 layer
/// and feed the inner minisign text to `minisign-verify`, mirroring how
/// `tauri-plugin-updater` checks an update (prehashed signatures; legacy
/// non-prehashed signatures are refused).
pub fn verify_signature(
    manifest_bytes: &[u8],
    signature: &str,
    pubkey: &str,
) -> Result<(), ManifestError> {
    let public_key = parse_wrapped_pubkey(pubkey)?;
    let sig_text = decode_wrapped(signature).map_err(ManifestError::BadSignature)?;
    let sig =
        Signature::decode(&sig_text).map_err(|e| ManifestError::BadSignature(e.to_string()))?;
    public_key
        .verify(manifest_bytes, &sig, false)
        .map_err(|e| ManifestError::SignatureMismatch(e.to_string()))
}

/// Parse already-verified manifest bytes and enforce the schema-version gate.
/// Never call this on bytes that haven't passed [`verify_signature`].
pub fn parse(manifest_bytes: &[u8]) -> Result<RuntimeManifest, ManifestError> {
    let manifest: RuntimeManifest = serde_json::from_slice(manifest_bytes)
        .map_err(|e| ManifestError::BadJson(e.to_string()))?;
    if manifest.schema_version > SUPPORTED_SCHEMA_VERSION {
        return Err(ManifestError::UnsupportedSchema {
            found: manifest.schema_version,
            supported: SUPPORTED_SCHEMA_VERSION,
        });
    }
    Ok(manifest)
}

impl RuntimeManifest {
    /// Pick the entry matching a runtime pin for the given arch. Version
    /// matching reuses the runtimes module's pin logic (exact, or a major/minor
    /// pin matching a fuller build), so a `.php-version` of "8.3" resolves to a
    /// hosted "8.3.14".
    pub fn select(&self, lang: &str, version: &str, arch: &str) -> Option<&RuntimeEntry> {
        self.entries.iter().find(|e| {
            e.lang == lang && e.arch == arch && super::super::version_matches(&e.version, version)
        })
    }
}

/// base64-decode a Tauri-wrapped minisign blob into its inner UTF-8 text.
fn decode_wrapped(b64: &str) -> Result<String, String> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .map_err(|e| e.to_string())?;
    String::from_utf8(bytes).map_err(|e| e.to_string())
}

/// Parse a Tauri-wrapped minisign **public key** into a verifier. The unwrapped
/// file is two lines — a comment then the base64 key — so we take the first
/// non-comment, non-empty line as the key.
fn parse_wrapped_pubkey(pubkey: &str) -> Result<PublicKey, ManifestError> {
    let text = decode_wrapped(pubkey).map_err(ManifestError::BadPublicKey)?;
    let key_line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("untrusted comment:"))
        .ok_or_else(|| ManifestError::BadPublicKey("no key line in public key file".into()))?;
    PublicKey::from_base64(key_line).map_err(|e| ManifestError::BadPublicKey(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real fixtures generated with `tauri signer` (the same tool the release
    // pipeline uses), so these tests prove interop with actual minisign output,
    // not a hand-rolled signature. The key is a throwaway test key.
    const MANIFEST: &[u8] = include_bytes!("fixtures/manifest.json");
    const SIGNATURE: &str = include_str!("fixtures/manifest.json.sig");
    const TEST_PUBKEY: &str = include_str!("fixtures/test-pubkey.b64");
    // The real app-updater pubkey from tauri.conf.json — a manifest signed by
    // the test key must fail against it (you can't swap in another signer).
    const PROD_UPDATER_PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDNBNEI4QjdFQzA4NkFBQjUKUldTMXFvYkFmb3RMT3J1MlZFdm51bDVlb3ZOU0cyNy94d0MvNjRKWGQ4eDRWUkxWR1poZ3VZMTgK";

    #[test]
    fn valid_manifest_verifies_and_parses() {
        let m = verify_and_parse(MANIFEST, SIGNATURE, TEST_PUBKEY).expect("should verify");
        assert_eq!(m.schema_version, 1);
        assert_eq!(m.entries.len(), 2);
        let first = &m.entries[0];
        assert_eq!(first.lang, "php");
        assert_eq!(first.version, "8.3.14");
        assert_eq!(first.arch, "aarch64");
        assert_eq!(first.compression, Compression::Zstd);
        assert!(first.url.starts_with("https://"));
    }

    #[test]
    fn tampered_manifest_is_rejected() {
        let mut bad = MANIFEST.to_vec();
        // Flip a byte in the body; the signature no longer matches.
        let i = bad.len() / 2;
        bad[i] ^= 0x01;
        let err = verify_and_parse(&bad, SIGNATURE, TEST_PUBKEY).unwrap_err();
        assert!(
            matches!(err, ManifestError::SignatureMismatch(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn tampered_signature_is_rejected() {
        // Corrupt the base64 wrapper so it no longer decodes to a valid sig.
        let bad_sig = format!("@@@{SIGNATURE}");
        let err = verify_and_parse(MANIFEST, &bad_sig, TEST_PUBKEY).unwrap_err();
        assert!(matches!(err, ManifestError::BadSignature(_)), "got {err:?}");
    }

    #[test]
    fn wrong_key_is_rejected() {
        // A valid manifest+signature pair, checked against a different (the real
        // production) key, must not verify.
        let err = verify_and_parse(MANIFEST, SIGNATURE, PROD_UPDATER_PUBKEY).unwrap_err();
        assert!(
            matches!(err, ManifestError::SignatureMismatch(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn newer_schema_version_is_rejected() {
        let body = br#"{"schemaVersion":2,"generatedAt":"2026-05-27T00:00:00Z","entries":[]}"#;
        let err = parse(body).unwrap_err();
        assert!(
            matches!(
                err,
                ManifestError::UnsupportedSchema {
                    found: 2,
                    supported: 1
                }
            ),
            "got {err:?}"
        );
    }

    #[test]
    fn malformed_json_is_rejected() {
        let err = parse(b"not json").unwrap_err();
        assert!(matches!(err, ManifestError::BadJson(_)), "got {err:?}");
    }

    #[test]
    fn select_matches_arch_and_version_pin() {
        let m = verify_and_parse(MANIFEST, SIGNATURE, TEST_PUBKEY).unwrap();
        // Exact arch + a major/minor pin resolves to the fuller build.
        let arm = m.select("php", "8.3", "aarch64").expect("arm64 8.3");
        assert_eq!(arm.arch, "aarch64");
        assert_eq!(arm.version, "8.3.14");
        // Same pin, other arch, picks the other archive.
        let intel = m.select("php", "8.3.14", "x86_64").expect("x86_64 exact");
        assert_eq!(intel.arch, "x86_64");
        assert_ne!(arm.url, intel.url);
        // No build for an unlisted version or arch.
        assert!(m.select("php", "8.4", "aarch64").is_none());
        assert!(m.select("php", "8.3", "armv7").is_none());
        assert!(m.select("node", "8.3", "aarch64").is_none());
    }

    #[test]
    fn schema_roundtrips_through_serde() {
        let m = RuntimeManifest {
            schema_version: 1,
            generated_at: "2026-05-27T00:00:00Z".into(),
            entries: vec![RuntimeEntry {
                lang: "php".into(),
                version: "8.4.1".into(),
                arch: current_arch().into(),
                url: "https://example.test/php.tar.xz".into(),
                sha256: "00".repeat(32),
                size: 123,
                compression: Compression::Xz,
            }],
        };
        let json = serde_json::to_vec(&m).unwrap();
        let back = parse(&json).unwrap();
        assert_eq!(back.entries[0].version, "8.4.1");
        assert_eq!(back.entries[0].compression, Compression::Xz);
    }
}
