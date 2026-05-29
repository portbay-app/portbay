//! Account avatar fetch + on-disk cache.
//!
//! The topbar profile chip shows the signed-in user's real GitHub avatar when
//! we can resolve one. GitHub avatars are publicly addressable by the numeric
//! account id (`https://avatars.githubusercontent.com/u/{id}`), so no API token
//! or extra scope is needed — but the app's CSP forbids loading remote images
//! directly (`img-src 'self' data: asset:`). So we fetch the bytes here in the
//! backend, cache them under the app-data dir, and hand the frontend a `data:`
//! URL it can drop straight into an `<img src>`.
//!
//! Caching matters for two reasons: it keeps the avatar visible offline (the
//! local-first promise — the app must not look broken without a network), and
//! it avoids hitting GitHub's CDN on every render. Email-auth accounts have no
//! `github_id` and resolve to `None`; the frontend then renders initials.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use base64::Engine;

/// Re-fetch the remote avatar at most this often. A cached copy younger than
/// this is served as-is; older than this we try the network and fall back to
/// the stale copy if the fetch fails.
const CACHE_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Hard ceiling on a fetched avatar. GitHub avatars at `s=128` are a few KB;
/// anything past this is almost certainly not the image we asked for.
const MAX_BYTES: u64 = 1024 * 1024;

/// Render size requested from GitHub. Larger than the topbar chip (~28px) and
/// menu header (~36px) so it stays crisp on retina displays.
const REQUEST_SIZE: u32 = 128;

/// `~/Library/Application Support/PortBay/avatars` (and platform equivalents).
/// Mirrors the entitlement cache's `data_dir()/PortBay` layout.
fn cache_dir() -> std::io::Result<PathBuf> {
    let mut path = dirs::data_dir()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no data dir"))?;
    path.push("PortBay");
    path.push("avatars");
    std::fs::create_dir_all(&path)?;
    Ok(path)
}

/// Cache file for a given key. See [`cache_key_for`].
fn cache_file(key: &str) -> std::io::Result<PathBuf> {
    Ok(cache_dir()?.join(format!("{key}.img")))
}

/// Stable, filename-safe cache key for an avatar URL. Keying on the full URL —
/// which carries the `?v=` cache-buster for custom uploads — means a changed
/// avatar lands in a fresh slot and is re-fetched immediately, while a switched
/// account (different URL) never serves the previous user's face.
fn cache_key_for(url: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut h);
    format!("{:016x}", h.finish())
}

/// Whether a cached file exists and was written within [`CACHE_TTL`].
fn is_fresh(path: &PathBuf) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    let Ok(modified) = meta.modified() else {
        return false;
    };
    SystemTime::now()
        .duration_since(modified)
        .map(|age| age < CACHE_TTL)
        .unwrap_or(false)
}

/// Sniff an image mime from the leading magic bytes, defaulting to PNG (the
/// format GitHub serves most avatars in). Only the handful of formats GitHub
/// actually emits are recognised.
fn sniff_mime(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        "image/png"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if bytes.starts_with(b"GIF8") {
        "image/gif"
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else {
        "image/png"
    }
}

/// Content-type for an avatar *upload*, or `None` when the bytes aren't one of
/// the formats the issuer accepts (PNG / JPEG / WebP). Unlike [`sniff_mime`],
/// this rejects unknown input rather than defaulting, so a bad file is caught
/// before it's sent.
pub fn detect_upload_mime(bytes: &[u8]) -> Option<&'static str> {
    let mime = sniff_mime(bytes);
    // sniff_mime defaults unknown bytes to PNG; only accept a *positively*
    // recognised magic number.
    let recognised = bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47])
        || bytes.starts_with(&[0xFF, 0xD8, 0xFF])
        || (bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP");
    recognised.then_some(mime)
}

/// Encode raw image bytes as an `<img>`-ready `data:` URL.
fn to_data_url(bytes: &[u8]) -> String {
    let mime = sniff_mime(bytes);
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    format!("data:{mime};base64,{b64}")
}

/// Fetch an avatar from `url`. Bounded by a timeout and a max body size; returns
/// `None` on any network/HTTP/size failure so callers degrade gracefully to a
/// cached copy or initials.
async fn fetch_remote(url: &str) -> Option<Vec<u8>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .ok()?;
    let resp = client.get(url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    // Reject oversized bodies up front when the server advertises a length.
    if resp
        .content_length()
        .map(|n| n > MAX_BYTES)
        .unwrap_or(false)
    {
        return None;
    }
    let bytes = resp.bytes().await.ok()?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_BYTES {
        return None;
    }
    Some(bytes.to_vec())
}

/// Resolve the current account's avatar as a `data:` URL, or `None` when there
/// is no resolvable avatar (signed out, an email-auth account with no
/// `github_id`, or the fetch failed with no usable cache).
///
/// Network is best-effort: a fresh cache short-circuits it entirely, and a
/// failed refresh falls back to a stale cached copy before giving up.
pub async fn account_avatar_data_url() -> Option<String> {
    let account = crate::entitlements::current().account?;
    // Prefer the server-resolved avatar URL (custom upload, else GitHub) carried
    // in the signed entitlement (schema ≥ 3). Fall back to the GitHub-by-id guess
    // for older docs that don't carry `avatar_url`.
    let url = account.avatar_url.clone().or_else(|| {
        account
            .github_id
            .map(|id| format!("https://avatars.githubusercontent.com/u/{id}?v=4&s={REQUEST_SIZE}"))
    })?;
    let path = cache_file(&cache_key_for(&url)).ok()?;

    if is_fresh(&path) {
        if let Ok(bytes) = std::fs::read(&path) {
            return Some(to_data_url(&bytes));
        }
    }

    match fetch_remote(&url).await {
        Some(bytes) => {
            // Best-effort persist — a write failure just means we re-fetch next
            // time, not that the avatar is unavailable now.
            let _ = std::fs::write(&path, &bytes);
            Some(to_data_url(&bytes))
        }
        // Offline or transient error: serve the stale copy if we have one.
        None => std::fs::read(&path).ok().map(|bytes| to_data_url(&bytes)),
    }
}

/// Drop all cached avatars. Called on sign-out so a shared machine doesn't keep
/// the previous user's face on disk.
pub fn clear_cache() {
    if let Ok(dir) = cache_dir() {
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_known_image_formats() {
        assert_eq!(sniff_mime(&[0x89, 0x50, 0x4E, 0x47, 0x0D]), "image/png");
        assert_eq!(sniff_mime(&[0xFF, 0xD8, 0xFF, 0xE0]), "image/jpeg");
        assert_eq!(sniff_mime(b"GIF89a"), "image/gif");
        let mut webp = b"RIFF".to_vec();
        webp.extend_from_slice(&[0, 0, 0, 0]);
        webp.extend_from_slice(b"WEBP");
        assert_eq!(sniff_mime(&webp), "image/webp");
    }

    #[test]
    fn unknown_bytes_default_to_png() {
        assert_eq!(sniff_mime(&[0x00, 0x01, 0x02]), "image/png");
        assert_eq!(sniff_mime(&[]), "image/png");
    }

    #[test]
    fn data_url_is_img_ready() {
        let url = to_data_url(&[0x89, 0x50, 0x4E, 0x47]);
        assert!(url.starts_with("data:image/png;base64,"));
    }
}
