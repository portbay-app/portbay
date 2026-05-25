//! End-to-end encrypted multi-device sync of the project registry (Pro).
//!
//! The user's registry is encrypted **on this device** with a 256-bit recovery
//! key before it is uploaded; the server (PortBay Cloud R2) stores only opaque
//! ciphertext and a version number. The recovery key never leaves the device
//! except as the one-time string the user copies to set up another device — so
//! the server genuinely cannot read synced data (see docs/legal/privacy-policy.md).
//!
//! Crypto: AES-256-GCM, random 96-bit nonce prepended to the ciphertext. The
//! recovery key (32 random bytes, shown to the user as base64url) *is* the AES
//! key — it's already high-entropy, so no KDF is needed.
//!
//! Conflict model: the server holds a monotonic version. A push that isn't
//! `force` and finds the remote version ahead of what this device last saw
//! returns `Conflict`, so the UI can offer pull-first vs overwrite.

use std::path::PathBuf;

use aes_gcm::aead::rand_core::RngCore;
use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::{Deserialize, Serialize};

const SYNC_KEY_SERVICE: &str = "app.portbay.synckey";
const KEYCHAIN_USER: &str = "default";
const NONCE_LEN: usize = 12;
const META_FILENAME: &str = "sync.meta.json";

// ---------------------------------------------------------------------------
// Recovery key (keychain)
// ---------------------------------------------------------------------------

pub type RecoveryKey = [u8; 32];

pub fn generate_recovery_key() -> RecoveryKey {
    let mut k = [0u8; 32];
    OsRng.fill_bytes(&mut k);
    k
}

pub fn key_to_string(key: &RecoveryKey) -> String {
    URL_SAFE_NO_PAD.encode(key)
}

pub fn key_from_string(s: &str) -> Option<RecoveryKey> {
    let bytes = URL_SAFE_NO_PAD.decode(s.trim().as_bytes()).ok()?;
    bytes.as_slice().try_into().ok()
}

fn key_entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(SYNC_KEY_SERVICE, KEYCHAIN_USER).map_err(|e| e.to_string())
}

pub fn store_key(key: &RecoveryKey) -> Result<(), String> {
    key_entry()?
        .set_password(&key_to_string(key))
        .map_err(|e| e.to_string())
}

pub fn load_key() -> Option<RecoveryKey> {
    let s = key_entry().ok()?.get_password().ok()?;
    key_from_string(&s)
}

pub fn clear_key() -> Result<(), String> {
    match key_entry()?.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

pub fn has_key() -> bool {
    load_key().is_some()
}

// ---------------------------------------------------------------------------
// AES-256-GCM
// ---------------------------------------------------------------------------

pub fn encrypt(key: &RecoveryKey, plaintext: &[u8]) -> Result<Vec<u8>, String> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let mut nonce = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce);
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce), plaintext)
        .map_err(|_| "encryption failed".to_string())?;
    let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

pub fn decrypt(key: &RecoveryKey, blob: &[u8]) -> Result<Vec<u8>, String> {
    if blob.len() <= NONCE_LEN {
        return Err("sync blob is too short".into());
    }
    let (nonce, ct) = blob.split_at(NONCE_LEN);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    cipher
        .decrypt(Nonce::from_slice(nonce), ct)
        .map_err(|_| "decryption failed — wrong recovery key for this account's data".to_string())
}

// ---------------------------------------------------------------------------
// Local sync metadata (last-seen version + device id)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncMeta {
    /// Highest server version this device has pushed or pulled.
    pub last_version: u64,
    /// This device's id registered with the server (for the devices list).
    pub device_id: Option<String>,
}

fn meta_path() -> std::io::Result<PathBuf> {
    let mut path = dirs::data_dir()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no data dir"))?;
    path.push("PortBay");
    std::fs::create_dir_all(&path)?;
    path.push(META_FILENAME);
    Ok(path)
}

pub fn load_meta() -> SyncMeta {
    meta_path()
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_meta(meta: &SyncMeta) -> std::io::Result<()> {
    let path = meta_path()?;
    std::fs::write(path, serde_json::to_string_pretty(meta)?)
}

pub fn clear_meta() {
    if let Ok(path) = meta_path() {
        let _ = std::fs::remove_file(path);
    }
}

// ---------------------------------------------------------------------------
// HTTP — PortBay Cloud /sync + /devices
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub last_seen: String,
}

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

/// GET the remote version without downloading the blob, or `None` if no doc yet.
pub async fn remote_version(base_url: &str, token: &str) -> Result<Option<u64>, String> {
    let url = format!("{}/sync", base_url.trim_end_matches('/'));
    let resp = client()
        .get(&url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| format!("sync check failed: {e}"))?;
    if resp.status().as_u16() == 404 {
        return Ok(None);
    }
    if !resp.status().is_success() {
        return Err(format!("sync check returned {}", resp.status()));
    }
    let v = resp
        .headers()
        .get("X-Sync-Version")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok());
    Ok(v)
}

/// PUT the encrypted blob; returns the new server version.
pub async fn push(base_url: &str, token: &str, blob: &[u8]) -> Result<u64, String> {
    let url = format!("{}/sync", base_url.trim_end_matches('/'));
    let resp = client()
        .put(&url)
        .bearer_auth(token)
        .header("Content-Type", "application/octet-stream")
        .body(blob.to_vec())
        .send()
        .await
        .map_err(|e| format!("sync upload failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("sync upload returned {}", resp.status()));
    }
    let body = resp
        .json::<serde_json::Value>()
        .await
        .map_err(|e| format!("reading sync result failed: {e}"))?;
    body.get("version")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| "sync upload returned no version".to_string())
}

/// GET the encrypted blob + version, or `None` if no document exists yet.
pub async fn pull(base_url: &str, token: &str) -> Result<Option<(Vec<u8>, u64)>, String> {
    let url = format!("{}/sync", base_url.trim_end_matches('/'));
    let resp = client()
        .get(&url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| format!("sync download failed: {e}"))?;
    if resp.status().as_u16() == 404 {
        return Ok(None);
    }
    if !resp.status().is_success() {
        return Err(format!("sync download returned {}", resp.status()));
    }
    let version = resp
        .headers()
        .get("X-Sync-Version")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("reading sync blob failed: {e}"))?;
    Ok(Some((bytes.to_vec(), version)))
}

pub async fn register_device(
    base_url: &str,
    token: &str,
    name: &str,
    platform: &str,
) -> Result<String, String> {
    let url = format!("{}/devices/register", base_url.trim_end_matches('/'));
    let resp = client()
        .post(&url)
        .bearer_auth(token)
        .json(&serde_json::json!({ "name": name, "platform": platform }))
        .send()
        .await
        .map_err(|e| format!("device registration failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("device registration returned {}", resp.status()));
    }
    let body = resp
        .json::<serde_json::Value>()
        .await
        .map_err(|e| format!("reading device id failed: {e}"))?;
    body.get("device_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "device registration returned no id".to_string())
}

pub async fn list_devices(base_url: &str, token: &str) -> Result<Vec<Device>, String> {
    let url = format!("{}/devices", base_url.trim_end_matches('/'));
    let resp = client()
        .get(&url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| format!("listing devices failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("listing devices returned {}", resp.status()));
    }
    let body = resp
        .json::<serde_json::Value>()
        .await
        .map_err(|e| format!("reading devices failed: {e}"))?;
    let devices = body
        .get("devices")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|d| serde_json::from_value::<Device>(d.clone()).ok())
                .collect()
        })
        .unwrap_or_default();
    Ok(devices)
}

pub async fn revoke_device(base_url: &str, token: &str, device_id: &str) -> Result<(), String> {
    let url = format!("{}/devices/{}", base_url.trim_end_matches('/'), device_id);
    let resp = client()
        .delete(&url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| format!("revoking device failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("revoking device returned {}", resp.status()));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests — crypto roundtrip (headless, no network/keychain)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_then_decrypt_roundtrips() {
        let key = generate_recovery_key();
        let plaintext = br#"{"projects":[{"id":"site","port":3000}]}"#;
        let blob = encrypt(&key, plaintext).unwrap();
        // Nonce is prepended, so the blob is longer than the plaintext.
        assert!(blob.len() > plaintext.len());
        let out = decrypt(&key, &blob).unwrap();
        assert_eq!(out, plaintext);
    }

    #[test]
    fn wrong_key_fails_to_decrypt() {
        let key = generate_recovery_key();
        let other = generate_recovery_key();
        let blob = encrypt(&key, b"secret registry").unwrap();
        assert!(decrypt(&other, &blob).is_err());
    }

    #[test]
    fn recovery_key_string_roundtrips() {
        let key = generate_recovery_key();
        let s = key_to_string(&key);
        assert_eq!(key_from_string(&s), Some(key));
        assert!(key_from_string("not-valid-base64!!").is_none());
    }

    #[test]
    fn nonce_is_random_per_encryption() {
        let key = generate_recovery_key();
        let a = encrypt(&key, b"same input").unwrap();
        let b = encrypt(&key, b"same input").unwrap();
        assert_ne!(a, b, "each encryption must use a fresh nonce");
    }
}
