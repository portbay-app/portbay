//! Tauri command surface for the agent-activity notification bell.
//!
//! The list is populated by the background scan in [`crate::notifications`];
//! these commands let the frontend hydrate it on launch and mutate read/clear
//! state. All of them go through the `Mutex<NotificationCenter>` in `AppState`.

use tauri::State;

use crate::notifications::Notification;
use crate::state::AppState;

/// Lock the center, recovering a poisoned mutex (a panic in the scan task must
/// not wedge the bell).
macro_rules! center {
    ($state:expr) => {
        $state
            .notifications
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    };
}

/// All notifications, newest-first, for hydrating the bell on launch.
#[tauri::command]
pub fn notifications_list(state: State<'_, AppState>) -> Vec<Notification> {
    center!(state).list()
}

/// Mark a single notification read (e.g. when the user clicks through to it).
#[tauri::command]
pub fn notifications_mark_read(state: State<'_, AppState>, id: String) {
    center!(state).mark_read(&id);
}

/// Mark every notification read — clears the bell's unread badge.
#[tauri::command]
pub fn notifications_mark_all_read(state: State<'_, AppState>) {
    center!(state).mark_all_read();
}

/// Wipe notification history (keeps scan cursors so cleared activity isn't
/// re-surfaced).
#[tauri::command]
pub fn notifications_clear(state: State<'_, AppState>) {
    center!(state).clear();
}
