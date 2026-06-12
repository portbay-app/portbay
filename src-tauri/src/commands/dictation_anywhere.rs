//! Tauri commands for "dictate anywhere" (system-wide push-to-talk on the
//! local STT engine — see `crate::dictation_anywhere` for the machinery).
//!
//! Status commands: the settings panel polls `dictation_anywhere_status`
//! to render the Accessibility affordance, and calls
//! `dictation_anywhere_arm` after the user toggles the feature on or
//! returns from System Settings — it (re)installs the global monitors when
//! trust has appeared, without an app restart.
//!
//! History commands: the Smart Dictation panel's recent-dictations list
//! (`crate::dictation_history` — copy lives in the frontend via the
//! clipboard API; only list/clear need the backend).

use tauri::AppHandle;

use crate::dictation_anywhere::AnywhereStatus;
use crate::dictation_history::HistoryEntry;
use crate::error::{AppError, AppResult};

/// Current feature status: platform support, Accessibility trust, and
/// whether the global monitors are live this run. Never errors — an
/// untrusted state is something to display, not to toast.
#[tauri::command]
pub async fn dictation_anywhere_status() -> AnywhereStatus {
    crate::dictation_anywhere::status()
}

/// Play a start-cue sound once, so the Settings picker can echo a selection
/// (and the volume slider a new level) the way macOS's alert-sound list
/// does. Fire-and-forget through the exact playback path the live session
/// cue uses — bare-name sanitizing and volume clamp included, so what the
/// user hears here is what Fn-down will play. Never errors: an unknown name
/// or zero volume is simply silent.
#[tauri::command]
pub async fn dictation_preview_cue(sound: String, volume: f32) {
    crate::dictation_anywhere::preview_cue(&sound, volume);
}

/// (Re)install the global monitors if Accessibility trust allows. Monitor
/// installation must happen on the main thread, so this hops there and
/// reports the resulting status.
///
/// `prompt: true` additionally fires macOS's own Accessibility dialog when
/// trust is missing — which registers PortBay in the Accessibility list so
/// the user only flips a switch. The frontend passes it exactly once, when
/// the user turns "Dictate anywhere" ON (a user-initiated moment, never a
/// poll or launch path — the Re-check button arms without prompting).
#[tauri::command]
pub async fn dictation_anywhere_arm(
    app: AppHandle,
    prompt: Option<bool>,
) -> AppResult<AnywhereStatus> {
    #[cfg(target_os = "macos")]
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let handle = app.clone();
        let prompt = prompt.unwrap_or(false);
        app.run_on_main_thread(move || {
            let mtm =
                objc2::MainThreadMarker::new().expect("run_on_main_thread is the main thread");
            if prompt {
                crate::typing::ax_prompt(mtm);
            }
            let _ = tx.send(crate::dictation_anywhere::ensure_monitors(&handle, mtm));
        })
        .map_err(|e| AppError::Internal(format!("main-thread hop failed: {e}")))?;
        rx.await
            .map_err(|_| AppError::Internal("status channel dropped".into()))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = prompt;
        Ok(crate::dictation_anywhere::ensure_monitors(&app))
    }
}

/// One running scriptable browser and whether PortBay may read its active
/// tab's URL (the notch's site-favicon feature).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserConsent {
    pub name: String,
    pub bundle_id: String,
    pub consent: crate::favicon::AutomationConsent,
}

/// Running scriptable browsers (name + bundle id), main-thread NSWorkspace
/// enumeration filtered through the favicon module's browser tables.
#[cfg(target_os = "macos")]
async fn running_browsers(app: &AppHandle) -> AppResult<Vec<(String, String)>> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let mtm = objc2::MainThreadMarker::new().expect("run_on_main_thread is the main thread");
        let browsers: Vec<(String, String)> = crate::typing::running_app_identities(mtm)
            .into_iter()
            .filter(|(_, bundle_id)| crate::favicon::browser_family(bundle_id).is_some())
            .collect();
        let _ = tx.send(browsers);
    })
    .map_err(|e| AppError::Internal(format!("main-thread hop failed: {e}")))?;
    rx.await
        .map_err(|_| AppError::Internal("browser list channel dropped".into()))
}

/// Consent state for the notch's site favicons: every RUNNING scriptable
/// browser and whether PortBay may read its active-tab URL. Display only —
/// never prompts (the probe is `askUserIfNeeded: false`).
#[tauri::command]
pub async fn dictation_favicon_consent(app: AppHandle) -> AppResult<Vec<BrowserConsent>> {
    #[cfg(target_os = "macos")]
    {
        let browsers = running_browsers(&app).await?;
        // tccd round trips — off the async workers, like any sync IPC.
        tokio::task::spawn_blocking(move || {
            browsers
                .into_iter()
                .map(|(name, bundle_id)| BrowserConsent {
                    consent: crate::favicon::automation_consent(&bundle_id),
                    name,
                    bundle_id,
                })
                .collect()
        })
        .await
        .map_err(|e| AppError::Internal(format!("consent probe panicked: {e}")))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Ok(Vec::new())
    }
}

/// Fire macOS's own Automation consent dialog for every running scriptable
/// browser that hasn't answered yet, then report the refreshed states. THE
/// ONLY PLACE the dialog is allowed to originate — strictly user-initiated
/// (the settings panel's "enable site icons" button); the dictation path
/// itself never prompts. Dialogs are sequential (macOS queues them); the
/// call returns when the user has answered the last one.
#[tauri::command]
pub async fn dictation_favicon_consent_request(app: AppHandle) -> AppResult<Vec<BrowserConsent>> {
    #[cfg(target_os = "macos")]
    {
        let browsers = running_browsers(&app).await?;
        tokio::task::spawn_blocking(move || {
            browsers
                .into_iter()
                .map(|(name, bundle_id)| {
                    let consent = match crate::favicon::automation_consent(&bundle_id) {
                        crate::favicon::AutomationConsent::NotDetermined => {
                            crate::favicon::request_automation_consent(&bundle_id)
                        }
                        already => already,
                    };
                    BrowserConsent {
                        name,
                        bundle_id,
                        consent,
                    }
                })
                .collect()
        })
        .await
        .map_err(|e| AppError::Internal(format!("consent request panicked: {e}")))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Ok(Vec::new())
    }
}

/// The notch overlay's stop button. An anywhere session is finished (or
/// aborted mid-arming) directly; otherwise the running session belongs to
/// the in-app micSession, which owns the splice — forward the stop as an
/// event the main window listens for.
#[tauri::command]
pub async fn dictation_overlay_stop(app: AppHandle) {
    if crate::dictation_anywhere::finish_active(&app) {
        return;
    }
    use tauri::Emitter;
    let _ = app.emit("dictation://stop-request", ());
}

/// Recent system-wide dictations, newest first. Local data; never errors.
#[tauri::command]
pub async fn dictation_history_list() -> Vec<HistoryEntry> {
    crate::dictation_history::list()
}

/// Drop the whole history ring (memory + disk) and retire the tray's
/// paste-again item.
#[tauri::command]
pub async fn dictation_history_clear(app: AppHandle) {
    crate::dictation_history::clear();
    crate::tray::refresh_dictation_item(&app);
}
