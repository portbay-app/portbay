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

use crate::registry::WebServer;

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

    /// Opt into early-access (experimental) features. Only meaningful for a
    /// Pro account with the `early_access` entitlement; the Settings toggle is
    /// Pro-gated. Read by `flags::enabled` (core) and the client flags store.
    #[serde(default)]
    pub early_access_opt_in: bool,

    // -------- General --------
    /// Register a LaunchAgent so PortBay starts at login. Off by
    /// default; the agent is provisioned the first time this flips on.
    #[serde(default)]
    pub launch_at_login: bool,

    /// On launch, re-start every project that was running when the app
    /// last quit. Off by default — the conservative choice for a tool
    /// that orchestrates real listeners on real ports.
    #[serde(default)]
    pub reopen_previous_projects: bool,

    /// Drives the StopAll button's confirm step. On by default — the
    /// universal kill switch is too easy to fat-finger otherwise.
    #[serde(default = "default_true")]
    pub confirm_before_stop_all: bool,

    /// macOS Notification Center toasts (separate from the in-app
    /// toast bus). Off by default.
    #[serde(default)]
    pub desktop_notifications: bool,

    // -------- Appearance --------
    /// Named accent colour. Drives `--color-accent`; the swatch grid
    /// in /settings is the canonical writer.
    #[serde(default = "default_accent_color")]
    pub accent_color: String,

    // -------- Workspace --------
    /// Path the Add Project wizard pre-fills with. Empty string means
    /// "let the OS suggest" (typically `~`).
    #[serde(default)]
    pub default_workspace_folder: String,

    /// Periodically scan `default_workspace_folder` for new project
    /// folders and prompt to register them. Off by default; opt-in
    /// because the scan is surprising the first time it triggers.
    #[serde(default)]
    pub auto_detect_projects: bool,

    /// Initial sort key for the projects table on cold launch.
    /// "name-asc" | "name-desc" | "status" | "port".
    #[serde(default = "default_sort")]
    pub default_sort: String,

    /// Whether newly-added projects auto-start by default.
    /// "manual" | "auto".
    #[serde(default = "default_start_behavior")]
    pub default_start_behavior: String,

    /// Web server pre-selected for *new* PHP projects in the Add Project
    /// wizard. `None` falls back to Caddy (PortBay's edge default). Set from
    /// the Web Server page; not applied retroactively — existing projects
    /// keep their own `web_server` (or the Caddy fallback in
    /// `Project::web_server_effective`).
    #[serde(default)]
    pub default_web_server: Option<WebServer>,

    // -------- Domains & HTTPS --------
    /// Permit PortBay to write managed entries to /etc/hosts. On by
    /// default for new installs; turning this off pins the user to a
    /// dnsmasq-only setup.
    #[serde(default = "default_true")]
    pub manage_hosts_automatically: bool,

    /// Auto-reissue local TLS certs before they expire. On by default.
    #[serde(default = "default_true")]
    pub auto_renew_certificates: bool,

    // -------- Advanced --------
    /// Persist project stdout/stderr to disk. On by default; turning
    /// off saves disk space but loses post-mortem debugging.
    #[serde(default = "default_true")]
    pub store_logs_locally: bool,

    /// How many days of logs to keep before rolling. 0 means "never
    /// auto-rotate"; the default trims aggressively.
    #[serde(default = "default_log_retention_days")]
    pub log_retention_days: u32,

    /// Filesystem path the bundled CLI is symlinked to (or copied to,
    /// when SIP forbids symlink). Exposed read-only with a copy button.
    #[serde(default = "default_cli_path")]
    pub cli_path: String,

    // -------- Artifacts --------
    /// Background auto-clean cadence for build artifacts across every
    /// registered project: "off" | "weekly" | "monthly". Off by default —
    /// auto-deleting `node_modules`/`vendor` is strictly opt-in.
    #[serde(default = "default_auto_clean_schedule")]
    pub auto_clean_schedule: String,

    /// Unix seconds of the last completed auto-clean pass; 0 = never. The
    /// scheduler stamps this after each pass, and enabling a schedule also
    /// stamps it, so the first auto pass is one cadence away — never an
    /// immediate surprise wipe the moment the user flips it on.
    #[serde(default)]
    pub last_auto_clean: u64,

    /// Extra project-relative directory names treated as artifacts on top of
    /// the built-in per-type catalogue (e.g. `.turbo`, `.cache`). Applied to
    /// every project type; honoured by both scan and clean.
    #[serde(default)]
    pub auto_clean_extra_dirs: Vec<String>,
}

fn default_true() -> bool {
    true
}

fn default_accent_color() -> String {
    "blue".to_string()
}

fn default_sort() -> String {
    "name-asc".to_string()
}

fn default_start_behavior() -> String {
    "manual".to_string()
}

fn default_log_retention_days() -> u32 {
    7
}

fn default_cli_path() -> String {
    "/usr/local/bin/portbay".to_string()
}

fn default_auto_clean_schedule() -> String {
    "off".to_string()
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            show_tray_icon: true,
            close_to_menu_bar: true,
            close_to_menu_bar_toast_seen: false,
            telemetry_enabled: false,
            early_access_opt_in: false,
            launch_at_login: false,
            reopen_previous_projects: false,
            confirm_before_stop_all: true,
            desktop_notifications: false,
            accent_color: default_accent_color(),
            default_workspace_folder: String::new(),
            auto_detect_projects: false,
            default_sort: default_sort(),
            default_start_behavior: default_start_behavior(),
            default_web_server: None,
            manage_hosts_automatically: true,
            auto_renew_certificates: true,
            store_logs_locally: true,
            log_retention_days: default_log_retention_days(),
            cli_path: default_cli_path(),
            auto_clean_schedule: default_auto_clean_schedule(),
            last_auto_clean: 0,
            auto_clean_extra_dirs: Vec::new(),
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
        let serialised = serde_json::to_vec_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
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
        // New default-web-server preference is absent in old files → None,
        // which `Project::web_server_effective` reads as Caddy.
        assert_eq!(p.default_web_server, None);
    }

    #[test]
    fn round_trip_camel_case() {
        let p = Preferences {
            show_tray_icon: false,
            close_to_menu_bar: true,
            close_to_menu_bar_toast_seen: true,
            telemetry_enabled: true,
            early_access_opt_in: true,
            launch_at_login: true,
            reopen_previous_projects: true,
            confirm_before_stop_all: false,
            desktop_notifications: true,
            accent_color: "purple".to_string(),
            default_workspace_folder: "/Users/dev/Projects".to_string(),
            auto_detect_projects: true,
            default_sort: "status".to_string(),
            default_start_behavior: "auto".to_string(),
            default_web_server: Some(WebServer::Nginx),
            manage_hosts_automatically: false,
            auto_renew_certificates: false,
            store_logs_locally: false,
            log_retention_days: 30,
            cli_path: "/opt/local/bin/portbay".to_string(),
            auto_clean_schedule: "weekly".to_string(),
            last_auto_clean: 1_700_000_000,
            auto_clean_extra_dirs: vec![".turbo".to_string(), ".cache".to_string()],
        };
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("\"showTrayIcon\":false"));
        assert!(json.contains("\"earlyAccessOptIn\":true"));
        assert!(json.contains("\"closeToMenuBar\":true"));
        assert!(json.contains("\"launchAtLogin\":true"));
        assert!(json.contains("\"accentColor\":\"purple\""));
        assert!(json.contains("\"logRetentionDays\":30"));
        assert!(json.contains("\"autoCleanSchedule\":\"weekly\""));
        assert!(json.contains("\"lastAutoClean\":1700000000"));
        assert!(json.contains("\"defaultWebServer\":\"nginx\""));
        let back: Preferences = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn auto_clean_defaults_are_off_and_unscheduled() {
        let p = Preferences::default();
        assert_eq!(p.auto_clean_schedule, "off");
        assert_eq!(p.last_auto_clean, 0);
        assert!(p.auto_clean_extra_dirs.is_empty());
    }
}
