//! The PortBay Model Catalog: the live, signed source of truth for downloadable
//! local models (speech-to-text today; text-to-speech next), so new models ship
//! without an app release.
//!
//! Mirrors two existing patterns:
//!   * the **runtimes manifest** (`runtimes::download::manifest`) for trust — the
//!     catalog is a minisign-signed JSON verified against the SAME updater key, so
//!     a model PortBay offers is only ever as trusted as an app update; and
//!   * the **ollama library cache** (`commands::ollama_library`) for freshness —
//!     fetched on a TTL, written to disk, and a failed refresh serves the stale
//!     cache instead of erroring.
//!
//! A bundled fallback (`resources/default-model-catalog.json`) means the catalog
//! is fully populated offline and before the hosted manifest exists; the live
//! manifest is merged over it (live wins per id, bundled fills gaps), so the list
//! is never *worse* than what shipped with the app.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::state::AppState;

use super::stt::SttCatalogModel;

/// New models land on a scale of weeks; a day-old catalog is "latest" in
/// practice. A manual refresh (the AI page button) bypasses this.
const CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);
/// Bump when the parsed shape changes so older cache files self-invalidate.
/// 2: added the `image` section (image-generation models).
const CACHE_SCHEMA: u32 = 2;

const MANIFEST_URL: &str =
    "https://github.com/portbay-app/portbay-model-catalog/releases/latest/download/model-catalog.json";
const SIGNATURE_URL: &str =
    "https://github.com/portbay-app/portbay-model-catalog/releases/latest/download/model-catalog.json.sig";

/// The bundled fallback, compiled in. Always present, so the catalog renders
/// offline and before the hosted manifest exists.
const BUNDLED: &str = include_str!("../../resources/default-model-catalog.json");

/// One text-to-speech voice option for a TTS model's playground picker.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsVoice {
    /// Engine voice id passed to synthesis (e.g. "af_heart").
    pub id: String,
    pub label: String,
}

/// One text-to-speech model (e.g. Kokoro). Same download/install machinery as
/// STT (the sidecar routes by `engine`); `voices` drives the playground picker.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsCatalogModel {
    pub id: String,
    /// "kokoro" today.
    pub engine: String,
    pub display_name: String,
    pub repo_model: String,
    pub approx_size_bytes: u64,
    pub languages: String,
    pub speed_note: String,
    pub recommended: bool,
    pub voices: Vec<TtsVoice>,
    #[serde(default)]
    pub default_voice: Option<String>,
    /// Model-weights license label — disclosed in the download UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// Where the license/model card lives.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license_url: Option<String>,
    /// Expected install-content digest (sidecar-verified before sealing).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_digest: Option<String>,
}

/// One on-device image-generation model (FLUX / SD3 today). Same
/// download/install machinery as STT/TTS (the `portbay-imagegen` sidecar routes
/// by `engine`); the extra knobs seed the playground's defaults per model.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageCatalogModel {
    pub id: String,
    /// "flux" | "sd" | "sdxl" | "stable-diffusion".
    pub engine: String,
    pub display_name: String,
    pub repo_model: String,
    pub approx_size_bytes: u64,
    /// Default diffusion steps (FLUX-schnell ≈ 4, SD ≈ 25).
    pub default_steps: u32,
    /// Native/recommended square resolution (e.g. 1024).
    pub default_size: u32,
    pub speed_note: String,
    pub recommended: bool,
    /// Optional override for the Hugging Face glob the sidecar fetches. Apple's
    /// repos vary by layout (SD under `split_einsum/compiled/`, SDXL under
    /// `compiled/`), and community conversions (e.g. SD-Turbo) ship under
    /// `original/compiled/`. When set, this wins over the sidecar's
    /// engine-derived default so the live catalog can carry new layouts without
    /// a sidecar rebuild. Omitted entries keep the engine default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compiled_glob: Option<String>,
    /// Model-weights license label — disclosed in the download UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// Where the license/model card lives.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license_url: Option<String>,
    /// Expected install-content digest (sidecar-verified before sealing).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_digest: Option<String>,
}

/// The signed catalog document (hosted + bundled share this shape).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalog {
    /// Rejected if newer than [`SUPPORTED_SCHEMA`] rather than half-parsed.
    pub schema_version: u32,
    #[serde(default)]
    pub generated_at: String,
    #[serde(default)]
    pub stt: Vec<SttCatalogModel>,
    #[serde(default)]
    pub tts: Vec<TtsCatalogModel>,
    /// Additive (schema 1 manifests omit it); the bundled fallback ships the
    /// launch image models until the hosted manifest carries them.
    #[serde(default)]
    pub image: Vec<ImageCatalogModel>,
}

/// The on-disk cache wrapper (catalog + provenance for staleness/UI).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CachedCatalog {
    #[serde(default)]
    schema: u32,
    fetched_at: String,
    catalog: ModelCatalog,
}

/// The STT catalog plus where it came from, for `stt_overview` and the UI's
/// "updated / refresh" affordance.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SttCatalogResult {
    pub models: Vec<SttCatalogModel>,
    /// Served from cache/bundled after a failed live refresh.
    pub stale: bool,
    /// "live" (verified manifest), "cache" (fresh disk cache), or "bundled".
    pub source: String,
}

/// The TTS catalog plus provenance metadata, mirroring [`SttCatalogResult`].
/// Lets `tts_overview` surface the same stale-cache banner the STT section
/// already shows rather than hardcoding `catalog_stale: false`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsCatalogResult {
    pub models: Vec<TtsCatalogModel>,
    /// Served from cache/bundled after a failed live refresh.
    pub stale: bool,
    /// "live" (verified manifest), "cache" (fresh disk cache), or "bundled".
    pub source: String,
}

/// Highest manifest schema this build understands.
const SUPPORTED_SCHEMA: u32 = 1;

fn parse_catalog(bytes: &[u8]) -> AppResult<ModelCatalog> {
    let catalog: ModelCatalog = serde_json::from_slice(bytes)
        .map_err(|e| AppError::Internal(format!("model catalog JSON invalid: {e}")))?;
    if catalog.schema_version > SUPPORTED_SCHEMA {
        return Err(AppError::Internal(format!(
            "model catalog schema {} is newer than supported {SUPPORTED_SCHEMA}",
            catalog.schema_version
        )));
    }
    Ok(catalog)
}

/// The compiled-in fallback. Infallible in practice (the file is validated at
/// build by this parse running in tests); an empty list is the worst case.
fn bundled() -> ModelCatalog {
    parse_catalog(BUNDLED.as_bytes()).unwrap_or(ModelCatalog {
        schema_version: SUPPORTED_SCHEMA,
        generated_at: String::new(),
        stt: Vec::new(),
        tts: Vec::new(),
        image: Vec::new(),
    })
}

/// Merge live over bundled: live entries win by id, bundled fills any id the
/// live manifest doesn't carry — the list is never worse than what shipped.
fn merge_stt(bundled: Vec<SttCatalogModel>, live: Vec<SttCatalogModel>) -> Vec<SttCatalogModel> {
    let live_ids: std::collections::HashSet<&str> = live.iter().map(|m| m.id.as_str()).collect();
    let mut out = live.clone();
    for b in bundled {
        if !live_ids.contains(b.id.as_str()) {
            out.push(b);
        }
    }
    out
}

/// Fetch + verify the hosted manifest (minisign against the updater key, like
/// the runtimes manifest). Env-overridable for local fixtures/tests — debug
/// builds only: even though the signature still gates content, a release
/// .app shouldn't let an env var redirect what it fetches (metadata leak +
/// replay surface).
async fn fetch_signed() -> AppResult<ModelCatalog> {
    #[cfg(debug_assertions)]
    let manifest_url =
        std::env::var("PORTBAY_MODEL_CATALOG_URL").unwrap_or_else(|_| MANIFEST_URL.to_string());
    #[cfg(not(debug_assertions))]
    let manifest_url = MANIFEST_URL.to_string();
    #[cfg(debug_assertions)]
    let signature_url = std::env::var("PORTBAY_MODEL_CATALOG_SIGNATURE_URL")
        .unwrap_or_else(|_| SIGNATURE_URL.to_string());
    #[cfg(not(debug_assertions))]
    let signature_url = SIGNATURE_URL.to_string();
    let manifest_bytes = reqwest::get(&manifest_url)
        .await
        .map_err(|e| AppError::Internal(format!("model catalog fetch failed: {e}")))?
        .error_for_status()
        .map_err(|e| AppError::Internal(format!("model catalog fetch failed: {e}")))?
        .bytes()
        .await
        .map_err(|e| AppError::Internal(format!("model catalog read failed: {e}")))?;
    let signature = reqwest::get(&signature_url)
        .await
        .map_err(|e| AppError::Internal(format!("model catalog signature fetch failed: {e}")))?
        .error_for_status()
        .map_err(|e| AppError::Internal(format!("model catalog signature fetch failed: {e}")))?
        .text()
        .await
        .map_err(|e| AppError::Internal(format!("model catalog signature read failed: {e}")))?;
    // Signature is checked BEFORE the JSON is parsed — untrusted bytes never
    // reach serde until proven ours.
    crate::runtimes::download::manifest::verify_signature(
        &manifest_bytes,
        &signature,
        crate::commands::runtimes::UPDATER_PUBKEY,
    )
    .map_err(|e| AppError::Internal(format!("model catalog signature invalid: {e}")))?;
    parse_catalog(&manifest_bytes)
}

/// Load the STT catalog: fresh disk cache → live fetch (cached on success) →
/// stale cache → bundled. Always returns a populated list.
pub async fn load_stt(state: &AppState, refresh: bool) -> SttCatalogResult {
    let path = cache_dir(state).join("model-catalog.json");
    let bundled = bundled();

    if !refresh {
        if let Some(c) = read_fresh::<CachedCatalog>(&path).filter(|c| c.schema == CACHE_SCHEMA) {
            return SttCatalogResult {
                models: merge_stt(bundled.stt, c.catalog.stt),
                stale: false,
                source: "cache".into(),
            };
        }
    }

    match fetch_signed().await {
        Ok(live) => {
            let cached = CachedCatalog {
                schema: CACHE_SCHEMA,
                fetched_at: chrono::Utc::now().to_rfc3339(),
                catalog: live.clone(),
            };
            write_cache(&path, &cached);
            SttCatalogResult {
                models: merge_stt(bundled.stt, live.stt),
                stale: false,
                source: "live".into(),
            }
        }
        Err(_) => {
            // Fetch failed (offline, or the manifest repo doesn't exist yet):
            // serve any cache, else the bundled fallback. Never an error — a
            // stale-but-present catalog beats an empty page.
            if let Some(c) = read_any::<CachedCatalog>(&path).filter(|c| c.schema == CACHE_SCHEMA) {
                SttCatalogResult {
                    models: merge_stt(bundled.stt, c.catalog.stt),
                    stale: true,
                    source: "cache".into(),
                }
            } else {
                SttCatalogResult {
                    models: bundled.stt,
                    stale: true,
                    source: "bundled".into(),
                }
            }
        }
    }
}

/// One catalog entry by id (for the download path: Rust resolves the spec and
/// passes engine/repoModel/version to the sidecar). Uses the cached/bundled
/// list — no network on the hot download path.
pub async fn stt_entry(state: &AppState, id: &str) -> Option<SttCatalogModel> {
    load_stt(state, false)
        .await
        .models
        .into_iter()
        .find(|m| m.id == id)
}

/// The TTS catalog, sourced the same way as STT (live manifest → cache →
/// bundled). The `tts` section is small and curated; merge is by id.
/// Returns a [`TtsCatalogResult`] with provenance so the UI can show the
/// same stale-cache banner the STT section does.
pub async fn load_tts(state: &AppState, refresh: bool) -> TtsCatalogResult {
    let path = cache_dir(state).join("model-catalog.json");
    let bundled_models = bundled().tts;
    let merge = |live: Vec<TtsCatalogModel>| {
        let live_ids: std::collections::HashSet<String> =
            live.iter().map(|m| m.id.clone()).collect();
        let mut out = live;
        for b in bundled_models.clone() {
            if !live_ids.contains(&b.id) {
                out.push(b);
            }
        }
        out
    };
    if !refresh {
        if let Some(c) = read_fresh::<CachedCatalog>(&path).filter(|c| c.schema == CACHE_SCHEMA) {
            return TtsCatalogResult {
                models: merge(c.catalog.tts),
                stale: false,
                source: "cache".into(),
            };
        }
    }
    match fetch_signed().await {
        Ok(live) => {
            let cached = CachedCatalog {
                schema: CACHE_SCHEMA,
                fetched_at: chrono::Utc::now().to_rfc3339(),
                catalog: live.clone(),
            };
            write_cache(&path, &cached);
            TtsCatalogResult {
                models: merge(live.tts),
                stale: false,
                source: "live".into(),
            }
        }
        Err(_) => {
            if let Some(c) = read_any::<CachedCatalog>(&path).filter(|c| c.schema == CACHE_SCHEMA) {
                TtsCatalogResult {
                    models: merge(c.catalog.tts),
                    stale: true,
                    source: "cache".into(),
                }
            } else {
                TtsCatalogResult {
                    models: bundled_models,
                    stale: true,
                    source: "bundled".into(),
                }
            }
        }
    }
}

/// One TTS catalog entry by id (download spec resolution).
pub async fn tts_entry(state: &AppState, id: &str) -> Option<TtsCatalogModel> {
    load_tts(state, false)
        .await
        .models
        .into_iter()
        .find(|m| m.id == id)
}

/// The image catalog plus provenance metadata, mirroring [`SttCatalogResult`].
/// Lets `imagegen_overview` surface the same stale-cache banner instead of
/// hardcoding `catalog_stale: false`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageCatalogResult {
    pub models: Vec<ImageCatalogModel>,
    /// Served from cache/bundled after a failed live refresh.
    pub stale: bool,
    /// "live" (verified manifest), "cache" (fresh disk cache), or "bundled".
    pub source: String,
}

/// The image-generation catalog, sourced the same way as STT/TTS (live manifest
/// → cache → bundled). The bundled fallback carries the launch models until the
/// hosted manifest's `image` section ships. Merge is by id.
pub async fn load_image(state: &AppState, refresh: bool) -> ImageCatalogResult {
    let path = cache_dir(state).join("model-catalog.json");
    let bundled = bundled().image;
    let merge = |live: Vec<ImageCatalogModel>| {
        let live_ids: std::collections::HashSet<String> =
            live.iter().map(|m| m.id.clone()).collect();
        let mut out = live;
        for b in bundled.clone() {
            if !live_ids.contains(&b.id) {
                out.push(b);
            }
        }
        out
    };
    if !refresh {
        if let Some(c) = read_fresh::<CachedCatalog>(&path).filter(|c| c.schema == CACHE_SCHEMA) {
            return ImageCatalogResult {
                models: merge(c.catalog.image),
                stale: false,
                source: "cache".into(),
            };
        }
    }
    match fetch_signed().await {
        Ok(live) => {
            let cached = CachedCatalog {
                schema: CACHE_SCHEMA,
                fetched_at: chrono::Utc::now().to_rfc3339(),
                catalog: live.clone(),
            };
            write_cache(&path, &cached);
            ImageCatalogResult {
                models: merge(live.image),
                stale: false,
                source: "live".into(),
            }
        }
        Err(_) => {
            if let Some(c) = read_any::<CachedCatalog>(&path).filter(|c| c.schema == CACHE_SCHEMA) {
                ImageCatalogResult {
                    models: merge(c.catalog.image),
                    stale: true,
                    source: "cache".into(),
                }
            } else {
                ImageCatalogResult {
                    models: bundled,
                    stale: true,
                    source: "bundled".into(),
                }
            }
        }
    }
}

/// One image catalog entry by id (download/generate spec resolution).
pub async fn image_entry(state: &AppState, id: &str) -> Option<ImageCatalogModel> {
    load_image(state, false)
        .await
        .models
        .into_iter()
        .find(|m| m.id == id)
}

// --- Cache (same shape as commands::ollama_library) ----------------------

fn cache_dir(state: &AppState) -> PathBuf {
    state
        .logs_dir
        .parent()
        .unwrap_or(&state.logs_dir)
        .join("model-catalog")
}

fn read_fresh<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let age = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())?;
    if age > CACHE_TTL {
        return None;
    }
    read_any(path)
}

fn read_any<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn write_cache<T: Serialize>(path: &Path, value: &T) {
    let Ok(bytes) = serde_json::to_vec(value) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
        // Owner-only (default umask would leave 0755/0644) — cache contents
        // aren't secret but there's no reason to expose them either.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
        }
    }
    let _ = std::fs::write(path, bytes);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_catalog_parses_and_is_populated() {
        let c = parse_catalog(BUNDLED.as_bytes()).expect("bundled catalog must parse");
        assert_eq!(c.schema_version, SUPPORTED_SCHEMA);
        assert!(c.stt.len() >= 8, "expected the full STT ladder");
        // Parakeet entries must carry a version (the sidecar needs it).
        for m in c.stt.iter().filter(|m| m.engine == "parakeet") {
            assert!(
                m.parakeet_version.is_some(),
                "parakeet entry {} missing parakeetVersion",
                m.id
            );
        }
        // Whisper ladder present.
        for id in ["whisper-tiny", "whisper-base", "whisper-small"] {
            assert!(c.stt.iter().any(|m| m.id == id), "missing {id}");
        }
        // Image-generation models present, each with a usable step/size default.
        assert!(!c.image.is_empty(), "expected bundled image models");
        for m in &c.image {
            assert!(m.default_steps > 0, "image {} has no default steps", m.id);
            assert!(
                m.default_size >= 256,
                "image {} has an implausible size",
                m.id
            );
        }
    }

    #[test]
    fn merge_prefers_live_and_fills_gaps() {
        let bundled = bundled().stt;
        let mut live = bundled[0].clone();
        live.display_name = "LIVE OVERRIDE".into();
        let merged = merge_stt(bundled.clone(), vec![live]);
        // Same count (override replaces, doesn't add) and the override wins.
        assert_eq!(merged.len(), bundled.len());
        assert_eq!(
            merged
                .iter()
                .find(|m| m.id == bundled[0].id)
                .unwrap()
                .display_name,
            "LIVE OVERRIDE"
        );
    }
}
