//! User-visible app preferences persisted to disk.
//!
//! Scope: behavioural toggles that don't belong in the registry (the
//! registry describes *what projects exist*; preferences describe *how
//! the shell behaves*). The current surface is the menu bar tray
//! (P3 — macOS menu bar tray mode) but the file is a forward-looking
//! home for any future window-level toggle (auto-launch at login, etc).
//!
//! Storage: a single JSON file under `<data_dir>/PortBay/preferences.json`.
//! Missing-file and parse failures fall back to defaults — the app must
//! boot even if the prefs file is corrupted by a disk fault.
//!
//! Concurrency: held behind a `std::sync::Mutex` in `AppState`. Reads
//! and writes are sub-millisecond; no async needed.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Filename used inside the PortBay data directory.
const FILENAME: &str = "preferences.json";

/// Behavioural toggles exposed to the user.
///
/// All fields default to the most-conservative on-by-default values that
/// make the tray feature unobtrusively useful out of the box. Fields are
/// `#[serde(default)]` so adding a new toggle in a future build doesn't
/// invalidate older prefs files.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preferences {
    /// When true, install the tray icon on launch. When toggled off at
    /// runtime, the existing icon is hidden via `TrayIcon::set_visible`.
    #[serde(default = "default_true")]
    pub show_tray_icon: bool,

    /// When true, clicking the window's close button hides the window
    /// instead of exiting the app. The tray-menu's "Quit PortBay" item
    /// (and ⌘Q in the app menu) remain the only ways to actually exit.
    #[serde(default = "default_true")]
    pub close_to_menu_bar: bool,

    /// Marker set the first time the user closes the window with
    /// `close_to_menu_bar` active. Prevents the "still running" toast
    /// from firing more than once.
    #[serde(default)]
    pub close_to_menu_bar_toast_seen: bool,

    /// Explicit opt-in. When false, PortBay never sends usage telemetry
    /// or crash reports over the network.
    #[serde(default)]
    pub telemetry_enabled: bool,
}

fn default_true() -> bool {
    true
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            show_tray_icon: true,
            close_to_menu_bar: true,
            close_to_menu_bar_toast_seen: false,
            telemetry_enabled: false,
        }
    }
}

impl Preferences {
    /// Resolve the on-disk path. Creates the parent directory on first
    /// call so a subsequent `save()` can't fail on a missing folder.
    pub fn path() -> std::io::Result<PathBuf> {
        let mut dir = dirs::data_dir().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "no platform data dir")
        })?;
        dir.push("PortBay");
        std::fs::create_dir_all(&dir)?;
        Ok(dir.join(FILENAME))
    }

    /// Load preferences from disk, returning defaults on missing file or
    /// any parse error. We log parse failures but never propagate them —
    /// boot must not depend on this file being intact.
    pub fn load() -> Self {
        let Ok(path) = Self::path() else {
            return Self::default();
        };
        let Ok(raw) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        match serde_json::from_str::<Preferences>(&raw) {
            Ok(prefs) => prefs,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    path = %path.display(),
                    "preferences.json corrupt — falling back to defaults"
                );
                Self::default()
            }
        }
    }

    /// Persist atomically: write to a temp file in the same directory,
    /// then rename. Avoids leaving a half-written file if the process
    /// is killed mid-write.
    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::path()?;
        let tmp = path.with_extension("json.tmp");
        let serialised = serde_json::to_vec_pretty(self).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
        })?;
        std::fs::write(&tmp, &serialised)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_on_for_both_tray_toggles() {
        let p = Preferences::default();
        assert!(p.show_tray_icon);
        assert!(p.close_to_menu_bar);
        assert!(!p.close_to_menu_bar_toast_seen);
        assert!(!p.telemetry_enabled);
    }

    #[test]
    fn missing_fields_default_via_serde() {
        // A prefs file written by an older build that only knows about
        // `showTrayIcon` must still deserialise cleanly.
        let raw = r#"{ "showTrayIcon": false }"#;
        let p: Preferences = serde_json::from_str(raw).unwrap();
        assert!(!p.show_tray_icon);
        assert!(p.close_to_menu_bar);
        assert!(!p.close_to_menu_bar_toast_seen);
        assert!(!p.telemetry_enabled);
    }

    #[test]
    fn round_trip_camel_case() {
        let p = Preferences {
            show_tray_icon: false,
            close_to_menu_bar: true,
            close_to_menu_bar_toast_seen: true,
            telemetry_enabled: true,
        };
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("\"showTrayIcon\":false"));
        assert!(json.contains("\"closeToMenuBar\":true"));
        assert!(json.contains("\"closeToMenuBarToastSeen\":true"));
        assert!(json.contains("\"telemetryEnabled\":true"));
        let back: Preferences = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
    }
}
