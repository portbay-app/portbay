//! Multi-device sync commands (Pro) — the IPC surface over `crate::sync`.
//!
//! All data is encrypted on this device with the recovery key before upload; the
//! server stores only ciphertext. Every command is gated on the `sync`
//! entitlement and a signed-in session.

use tauri::State;

use crate::auth::{self, CLOUD_BASE_URL};
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::sync;

fn require_sync_entitlement() -> AppResult<()> {
    if crate::entitlements::current().entitlements.sync {
        Ok(())
    } else {
        Err(AppError::Internal(
            "Multi-device sync is a Pro feature.".into(),
        ))
    }
}

async fn fresh_token() -> AppResult<String> {
    auth::access_token_refreshing(CLOUD_BASE_URL)
        .await
        .ok_or_else(|| AppError::Internal("Sign in to PortBay to sync.".into()))
}

fn device_name() -> String {
    sysinfo::System::host_name().unwrap_or_else(|| "PortBay device".to_string())
}

#[derive(serde::Serialize)]
pub struct SyncState {
    pub signed_in: bool,
    pub is_pro: bool,
    /// A recovery key is present on this device.
    pub enabled: bool,
    pub last_version: u64,
}

/// Current sync state for the Settings surface. No network — and no keychain:
/// `enabled` comes from the non-secret meta flag, because reading the recovery
/// key's secret can pop a macOS keychain prompt and opening Settings must never
/// do that. (`last_version > 0` grandfathers devices that synced before the
/// flag existed.)
#[tauri::command]
pub async fn sync_state() -> AppResult<SyncState> {
    let eff = crate::entitlements::current();
    let meta = sync::load_meta();
    Ok(SyncState {
        signed_in: eff.account.is_some(),
        is_pro: eff.entitlements.sync,
        enabled: meta.enabled || meta.last_version > 0,
        last_version: meta.last_version,
    })
}

/// Enable sync on this device: generate (or return the existing) recovery key.
/// The caller shows the returned string to the user to save.
#[tauri::command]
pub async fn enable_sync() -> AppResult<String> {
    require_sync_entitlement()?;
    if let Some(k) = sync::load_key() {
        sync::set_enabled_flag(true);
        return Ok(sync::key_to_string(&k));
    }
    let key = sync::generate_recovery_key();
    sync::store_key(&key).map_err(AppError::Internal)?;
    sync::set_enabled_flag(true);
    Ok(sync::key_to_string(&key))
}

/// Return the recovery key string for display/copy, if one exists.
#[tauri::command]
pub async fn get_recovery_key() -> AppResult<Option<String>> {
    Ok(sync::load_key().map(|k| sync::key_to_string(&k)))
}

/// Set this device's recovery key from the string shown on another device.
#[tauri::command]
pub async fn set_recovery_key(key: String) -> AppResult<()> {
    let parsed = sync::key_from_string(&key)
        .ok_or_else(|| AppError::BadInput("That doesn't look like a valid recovery key.".into()))?;
    sync::store_key(&parsed).map_err(AppError::Internal)?;
    // Reset local version so the next pull is treated as authoritative.
    let _ = sync::save_meta(&sync::SyncMeta {
        enabled: true,
        ..Default::default()
    });
    Ok(())
}

/// Disable sync on this device (forget the recovery key + local metadata). Does
/// not touch the remote blob or other devices.
#[tauri::command]
pub async fn disable_sync() -> AppResult<()> {
    sync::clear_key().map_err(AppError::Internal)?;
    sync::clear_meta();
    Ok(())
}

#[derive(serde::Serialize)]
#[serde(tag = "result", rename_all = "lowercase")]
pub enum PushOutcome {
    Ok {
        version: u64,
    },
    /// The remote is ahead of what this device last saw — pull first or force.
    Conflict {
        remote_version: u64,
    },
}

/// Encrypt the local registry and upload it. Unless `force`, refuses when the
/// remote version is ahead of this device's last-seen version (conflict).
#[tauri::command]
pub async fn sync_push(state: State<'_, AppState>, force: bool) -> AppResult<PushOutcome> {
    require_sync_entitlement()?;
    let key = sync::load_key()
        .ok_or_else(|| AppError::Internal("Sync isn't set up on this device.".into()))?;
    let token = fresh_token().await?;

    let mut meta = sync::load_meta();
    if !force {
        if let Some(remote) = sync::remote_version(CLOUD_BASE_URL, &token)
            .await
            .map_err(AppError::Internal)?
        {
            if remote > meta.last_version {
                return Ok(PushOutcome::Conflict {
                    remote_version: remote,
                });
            }
        }
    }

    let client_device_id = sync::ensure_client_device_id(&mut meta);
    if meta.device_id.is_none() {
        match sync::register_device(
            CLOUD_BASE_URL,
            &token,
            &device_name(),
            std::env::consts::OS,
            &client_device_id,
        )
        .await
        {
            Ok(id) => {
                meta.device_id = Some(id);
                let _ = sync::save_meta(&meta);
            }
            // A blocked 3rd device must not silently push without a slot.
            Err(sync::RegisterError::LimitReached { max }) => {
                return Err(AppError::DeviceLimitReached { max })
            }
            // Registration is best-effort for visibility; a transient failure
            // shouldn't block a push from an already-entitled device.
            Err(sync::RegisterError::Other(_)) => {}
        }
    }

    let registry = std::fs::read(&state.registry_path).map_err(AppError::Io)?;
    let blob = sync::encrypt(&key, &registry).map_err(AppError::Internal)?;
    let version = sync::push(CLOUD_BASE_URL, &token, &blob)
        .await
        .map_err(AppError::Internal)?;
    meta.last_version = version;
    let _ = sync::save_meta(&meta);
    Ok(PushOutcome::Ok { version })
}

/// Download + decrypt the remote registry and replace the local one, then kick
/// the reconciler. Returns false when there's nothing remote yet.
#[tauri::command]
pub async fn sync_pull(state: State<'_, AppState>) -> AppResult<bool> {
    require_sync_entitlement()?;
    let key = sync::load_key()
        .ok_or_else(|| AppError::Internal("Sync isn't set up on this device.".into()))?;
    let token = fresh_token().await?;

    let Some((blob, version)) = sync::pull(CLOUD_BASE_URL, &token)
        .await
        .map_err(AppError::Internal)?
    else {
        return Ok(false);
    };

    let plaintext = sync::decrypt(&key, &blob).map_err(AppError::Internal)?;
    // GCM authenticates the ciphertext (only a holder of the key wrote it); a
    // JSON sanity check guards against a corrupt blob before we overwrite.
    serde_json::from_slice::<serde_json::Value>(&plaintext)
        .map_err(|e| AppError::Internal(format!("synced registry is not valid JSON: {e}")))?;
    std::fs::write(&state.registry_path, &plaintext).map_err(AppError::Io)?;

    let mut meta = sync::load_meta();
    meta.last_version = version;
    let _ = sync::save_meta(&meta);
    state.reconciler.mark_dirty();
    Ok(true)
}

#[derive(serde::Serialize)]
pub struct DeviceActivation {
    /// The server device id for this install.
    pub device_id: String,
    /// The license's device cap (Pro = 2).
    pub max_devices: u32,
}

/// Activate this device against the account's 2-device license cap. Idempotent
/// on the stable `client_device_id`; returns `DeviceLimitReached` when a *new*
/// device would exceed the cap so the UI can prompt the user to deactivate one.
#[tauri::command]
pub async fn activate_device() -> AppResult<DeviceActivation> {
    require_sync_entitlement()?;
    let token = fresh_token().await?;
    let mut meta = sync::load_meta();
    let client_device_id = sync::ensure_client_device_id(&mut meta);

    match sync::register_device(
        CLOUD_BASE_URL,
        &token,
        &device_name(),
        std::env::consts::OS,
        &client_device_id,
    )
    .await
    {
        Ok(id) => {
            meta.device_id = Some(id.clone());
            let _ = sync::save_meta(&meta);
            let max = crate::entitlements::current()
                .entitlements
                .max_devices
                .unwrap_or(2);
            Ok(DeviceActivation {
                device_id: id,
                max_devices: max,
            })
        }
        Err(sync::RegisterError::LimitReached { max }) => Err(AppError::DeviceLimitReached { max }),
        Err(sync::RegisterError::Other(e)) => Err(AppError::Internal(e)),
    }
}

/// List this account's registered devices.
#[tauri::command]
pub async fn list_sync_devices() -> AppResult<Vec<sync::Device>> {
    require_sync_entitlement()?;
    let token = fresh_token().await?;
    sync::list_devices(CLOUD_BASE_URL, &token)
        .await
        .map_err(AppError::Internal)
}

/// Revoke (deauthorize) a device by id.
#[tauri::command]
pub async fn revoke_sync_device(device_id: String) -> AppResult<()> {
    require_sync_entitlement()?;
    let token = fresh_token().await?;
    sync::revoke_device(CLOUD_BASE_URL, &token, &device_id)
        .await
        .map_err(AppError::Internal)
}
