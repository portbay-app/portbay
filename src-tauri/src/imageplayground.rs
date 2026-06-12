//! Apple Image Playground — in-process bridge to the system `ImageCreator`.
//!
//! Apple's `ImageCreator` generates images programmatically but throws
//! `backgroundCreationForbidden` unless the calling process is the frontmost,
//! active GUI app — so, unlike the headless Stable Diffusion sidecar, this runs
//! INSIDE the PortBay process via a tiny Swift static library
//! (swift/PortBayImagePlayground.swift, compiled + linked by build.rs). Prompts
//! and images stay in-app; the user types a prompt and sees the result in
//! PortBay, with no separate window. The Tauri commands live in
//! `commands::imageplayground`.

#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

/// Availability the AI page renders. `reason` is one of: `requires_macos_15_4`,
/// `apple_intelligence_unavailable`, `unsupported_device`, `unavailable`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImagePlaygroundStatus {
    pub available: bool,
    #[serde(default)]
    pub reason: Option<String>,
}

#[cfg(target_os = "macos")]
mod ffi {
    use std::os::raw::c_char;
    extern "C" {
        pub fn portbay_ip_check() -> *mut c_char;
        pub fn portbay_ip_generate(prompt: *const c_char) -> *mut c_char;
        pub fn portbay_ip_free(p: *mut c_char);
    }
}

/// Copy a malloc'd C string from the Swift side into an owned `String`, then
/// hand the pointer back to Swift to free.
#[cfg(target_os = "macos")]
fn take_cstring(ptr: *mut std::os::raw::c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: `ptr` is a NUL-terminated C string allocated by the Swift bridge
    // (strdup); we copy it, then free it through the matching deallocator.
    let owned = unsafe { std::ffi::CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned();
    unsafe { ffi::portbay_ip_free(ptr) };
    Some(owned)
}

/// Probe whether Apple Image Playground can generate here (macOS 15.4+, Apple
/// Intelligence enabled, supported device). Runs off the main thread.
#[cfg(target_os = "macos")]
pub async fn check() -> ImagePlaygroundStatus {
    let reason = tokio::task::spawn_blocking(|| {
        // SAFETY: the bridge returns a malloc'd C string (or null); freed below.
        take_cstring(unsafe { ffi::portbay_ip_check() }).unwrap_or_else(|| "unavailable".into())
    })
    .await
    .unwrap_or_else(|_| "unavailable".into());

    if reason == "ok" {
        ImagePlaygroundStatus {
            available: true,
            reason: None,
        }
    } else {
        ImagePlaygroundStatus {
            available: false,
            reason: Some(reason),
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub async fn check() -> ImagePlaygroundStatus {
    ImagePlaygroundStatus {
        available: false,
        reason: Some("unsupported".to_string()),
    }
}

/// Generate one image with Apple Image Playground from `prompt`, returning a
/// base64 PNG. `Ok(None)` means generation was cancelled (not an error). Runs on
/// a blocking thread — the FFI call blocks until the system finishes, while the
/// app's main run loop keeps pumping (PortBay stays frontmost/active, which is
/// what lets `ImageCreator` run at all).
#[cfg(target_os = "macos")]
pub async fn generate(prompt: Option<&str>) -> Result<Option<String>, String> {
    let prompt = prompt.unwrap_or("").to_string();
    let result = tokio::task::spawn_blocking(move || {
        let c = std::ffi::CString::new(prompt)
            .map_err(|_| "prompt contained a NUL byte".to_string())?;
        // SAFETY: pass a valid NUL-terminated string in; receive a malloc'd C
        // string out (freed by take_cstring).
        take_cstring(unsafe { ffi::portbay_ip_generate(c.as_ptr()) })
            .ok_or_else(|| "Image Playground returned nothing".to_string())
    })
    .await
    .map_err(|e| format!("Image Playground task failed: {e}"))??;

    if result == "CANCEL" {
        return Ok(None);
    }
    // Apple's image-model resources aren't downloaded on this Mac (the device
    // itself is supported — the startup check passed). Stable token the
    // playground UI matches to offer the system download flow.
    if result == "NOTREADY" {
        return Err("model_not_downloaded".to_string());
    }
    if let Some(b64) = result.strip_prefix("OK:") {
        return Ok(Some(b64.to_string()));
    }
    if let Some(msg) = result.strip_prefix("ERR:") {
        return Err(msg.to_string());
    }
    Err(result)
}

#[cfg(not(target_os = "macos"))]
pub async fn generate(_prompt: Option<&str>) -> Result<Option<String>, String> {
    Err("Apple Image Playground is macOS-only".to_string())
}
