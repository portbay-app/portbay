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
