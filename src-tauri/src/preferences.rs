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

use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::Timelike;
use serde::{Deserialize, Serialize};

use crate::registry::WebServer;

/// Filename used inside the PortBay data directory.
const FILENAME: &str = "preferences.json";

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NotificationCategory {
    Lifecycle,
    ProjectError,
    AgentBoard,
    Updates,
    Crash,
    Infrastructure,
    AccountSync,
}

impl NotificationCategory {
    pub const ALL: [Self; 7] = [
        Self::Lifecycle,
        Self::ProjectError,
        Self::AgentBoard,
        Self::Updates,
        Self::Crash,
        Self::Infrastructure,
        Self::AccountSync,
    ];
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationChannel {
    Toast,
    Bell,
    Banner,
    Sound,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationSeverity {
    Success,
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationSeverityFloor {
    ErrorsOnly,
    ErrorsAndWarnings,
    #[default]
    Everything,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationChannelPrefs {
    #[serde(default)]
    pub toast: bool,
    #[serde(default = "default_true")]
    pub bell: bool,
    #[serde(default)]
    pub banner: bool,
    #[serde(default)]
    pub sound: bool,
}

impl NotificationChannelPrefs {
    fn for_category(category: NotificationCategory) -> Self {
        match category {
            NotificationCategory::Lifecycle => Self {
                toast: false,
                bell: true,
                banner: false,
                sound: false,
            },
            NotificationCategory::ProjectError | NotificationCategory::Crash => Self {
                toast: true,
                bell: true,
                banner: false,
                sound: true,
            },
            NotificationCategory::AgentBoard => Self {
                toast: false,
                bell: true,
                banner: false,
                sound: true,
            },
            NotificationCategory::Updates
            | NotificationCategory::Infrastructure
            | NotificationCategory::AccountSync => Self {
                toast: false,
                bell: true,
                banner: false,
                sound: false,
            },
        }
    }

    pub fn enabled(&self, channel: NotificationChannel) -> bool {
        match channel {
            NotificationChannel::Toast => self.toast,
            NotificationChannel::Bell => self.bell,
            NotificationChannel::Banner => self.banner,
            NotificationChannel::Sound => self.sound,
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationQuietHours {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_quiet_start")]
    pub start: String,
    #[serde(default = "default_quiet_end")]
    pub end: String,
    #[serde(default = "default_true")]
    pub exempt_errors: bool,
}

impl Default for NotificationQuietHours {
    fn default() -> Self {
        Self {
            enabled: false,
            start: default_quiet_start(),
            end: default_quiet_end(),
            exempt_errors: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationCue {
    Done,
    Comment,
    Attention,
    Error,
}

/// The distinct agent-board events that each carry their own sound toggle.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentSoundEvent {
    Done,
    Error,
    Comment,
    /// An agent recorded a project learning. Low-urgency FYI, so its sound is
    /// off by default (the bell still shows it) — toggle it on like the others.
    Learning,
}

impl AgentSoundEvent {
    pub const ALL: [Self; 4] = [Self::Done, Self::Error, Self::Comment, Self::Learning];
}

/// Per-event sound: whether it plays and which cue it uses.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSoundSetting {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_agent_event_cue")]
    pub cue: NotificationCue,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationSoundPrefs {
    #[serde(default = "default_true")]
    pub volume_follows_os: bool,
    #[serde(default = "default_notification_cues")]
    pub cue_per_category: BTreeMap<NotificationCategory, NotificationCue>,
    #[serde(default = "default_agent_sound_events")]
    pub agent_events: BTreeMap<AgentSoundEvent, AgentSoundSetting>,
}

impl Default for NotificationSoundPrefs {
    fn default() -> Self {
        Self {
            volume_follows_os: true,
            cue_per_category: default_notification_cues(),
            agent_events: default_agent_sound_events(),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationPrefs {
    #[serde(default = "default_notification_schema_version")]
    pub schema_version: u32,
    #[serde(default = "default_notification_channels")]
    pub channels: BTreeMap<NotificationCategory, NotificationChannelPrefs>,
    #[serde(default)]
    pub severity_floor: NotificationSeverityFloor,
    #[serde(default)]
    pub quiet_hours: NotificationQuietHours,
    #[serde(default)]
    pub snooze_until: Option<u64>,
    #[serde(default)]
    pub sound: NotificationSoundPrefs,
}

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AccessibilityTextScale {
    #[default]
    Normal,
    Large,
    Larger,
}

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AccessibilityFocusMode {
    #[default]
    Standard,
    Strong,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessibilityPrefs {
    #[serde(default)]
    pub reduce_motion: bool,
    #[serde(default)]
    pub reduce_transparency: bool,
    #[serde(default)]
    pub high_contrast: bool,
    #[serde(default)]
    pub text_scale: AccessibilityTextScale,
    #[serde(default)]
    pub focus_mode: AccessibilityFocusMode,
    #[serde(default)]
    pub underline_links: bool,
    #[serde(default)]
    pub color_independent_status: bool,
}

impl Default for AccessibilityPrefs {
    fn default() -> Self {
        Self {
            reduce_motion: false,
            reduce_transparency: false,
            high_contrast: false,
            text_scale: AccessibilityTextScale::Normal,
            focus_mode: AccessibilityFocusMode::Standard,
            underline_links: false,
            color_independent_status: false,
        }
    }
}

impl Default for NotificationPrefs {
    fn default() -> Self {
        Self {
            schema_version: default_notification_schema_version(),
            channels: default_notification_channels(),
            severity_floor: NotificationSeverityFloor::Everything,
            quiet_hours: NotificationQuietHours::default(),
            snooze_until: None,
            sound: NotificationSoundPrefs::default(),
        }
    }
}

impl NotificationPrefs {
    pub fn normalised(mut self) -> Self {
        self.schema_version = default_notification_schema_version();
        for category in NotificationCategory::ALL {
            self.channels
                .entry(category)
                .or_insert_with(|| NotificationChannelPrefs::for_category(category));
            self.sound
                .cue_per_category
                .entry(category)
                .or_insert_with(|| default_cue_for_category(category));
        }
        for event in AgentSoundEvent::ALL {
            self.sound
                .agent_events
                .entry(event)
                .or_insert_with(|| default_agent_sound_setting(event));
        }
        self
    }

    pub fn with_legacy_desktop(mut self, legacy_enabled: bool) -> Self {
        if legacy_enabled {
            self.channels
                .entry(NotificationCategory::AgentBoard)
                .or_insert_with(|| {
                    NotificationChannelPrefs::for_category(NotificationCategory::AgentBoard)
                })
                .banner = true;
        }
        self
    }

    pub fn channel_enabled(
        &self,
        category: NotificationCategory,
        channel: NotificationChannel,
    ) -> bool {
        self.channels
            .get(&category)
            .unwrap_or(&NotificationChannelPrefs::for_category(category))
            .enabled(channel)
    }
}

/// Behavioural toggles exposed to the user.
///
/// All fields default to the most-conservative on-by-default values that
/// make the tray feature unobtrusively useful out of the box. Fields are
/// `#[serde(default)]` so adding a new toggle in a future build doesn't
/// invalidate older prefs files. Not `Eq` — `DictationPrefs` carries an f64.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preferences {
    /// When true, install the tray icon on launch. When toggled off at
    /// runtime, the existing icon is hidden via `TrayIcon::set_visible`.
    #[serde(default = "default_true")]
    pub show_tray_icon: bool,

    /// When true, PortBay shows its icon in the macOS Dock (the regular
    /// activation policy). When false, the app runs as an accessory — no
    /// Dock tile, present only in the menu-bar tray. On by default; toggled
    /// at runtime via `NSApplication`'s activation policy. macOS-only.
    #[serde(default = "default_true")]
    pub show_dock_icon: bool,

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

    /// Whether the user has been shown the one-time diagnostics consent
    /// prompt (the gcloud-style "may we collect anonymized usage + crashes?"
    /// shown after the first `portbay login`). Once true — regardless of the
    /// answer — we never prompt again; the answer itself lives in
    /// `telemetry_enabled`. Shared with the GUI so neither surface re-asks.
    #[serde(default)]
    pub telemetry_consent_prompted: bool,

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

    /// Per-category notification routing and interruption rules.
    #[serde(default)]
    pub notifications: NotificationPrefs,

    /// Accessibility display preferences applied by the shell at runtime.
    #[serde(default)]
    pub accessibility: AccessibilityPrefs,

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

    /// Terminal app used to host an *interactive* agent dispatch (the board's
    /// "Start with agent" / auto-on-To-Do). One of the detected terminal tool
    /// ids — `"warp"`, `"iterm"`, `"ghostty"`, `"terminal"`. `None` resolves at
    /// launch time to the first detected terminal, falling back to macOS
    /// Terminal.app. Lets the agent (the LLM/CLI) and the terminal (the host
    /// window) be chosen independently.
    #[serde(default)]
    pub preferred_terminal: Option<String>,

    /// Global default agent (kind id, e.g. `"claude"`) dispatched for project
    /// boards that haven't saved their own automation config yet. A project's
    /// own board config overrides this once edited. `None` → Claude.
    #[serde(default)]
    pub preferred_agent: Option<String>,

    /// Per-agent absolute binary path overrides, keyed by agent id. For agents
    /// installed outside PATH and the scanned dirs (external drives, custom
    /// prefixes) — the analogue of the runtimes "add by path" flow. Detection
    /// and dispatch prefer these when set and executable.
    #[serde(default)]
    pub agent_paths: BTreeMap<String, String>,

    /// Per-agent launch mode, keyed by agent id: `"cli"` runs the command-line
    /// tool (default), `"app"` opens the agent's desktop app/IDE at the project
    /// and hands the prompt over via the clipboard. A key is absent until the
    /// user changes it; a missing or unknown value reads as `"cli"`. Only
    /// honoured for agents whose app form is actually detected.
    #[serde(default)]
    pub agent_launch_modes: BTreeMap<String, String>,

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

    // -------- AI / local Ollama --------
    /// Local Ollama manager configuration. Owns the endpoint shared by local
    /// AI consumers; `dictation.endpoint` is retained for older prefs files and
    /// mirrored from this value during normalisation.
    #[serde(default)]
    pub ai: AiPrefs,

    // -------- Dictation --------
    /// Smart Dictation — the optional rewrite layer over macOS dictation.
    /// macOS stays the recognizer; these settings only govern what happens
    /// to the transcript text afterwards.
    #[serde(default)]
    pub dictation: DictationPrefs,

    // -------- Local speech-to-text --------
    /// Storage settings for the local STT engine's models (the `portbay-stt`
    /// sidecar — Whisper/Parakeet). Which engine transcribes lives on
    /// `dictation` (it's a dictation behavior); this is the models' home.
    #[serde(default)]
    pub stt: SttPrefs,
}

/// Local speech-to-text model storage. Mirrors `AiPrefs.models_dir`'s role
/// for Ollama: one user-configurable directory the AI page manages.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SttPrefs {
    /// Where downloaded STT models live. Each model owns one subdirectory
    /// named by its catalog id (see stt/Sources/portbay-stt/main.swift).
    #[serde(default = "default_stt_models_dir")]
    pub models_dir: String,
}

impl Default for SttPrefs {
    fn default() -> Self {
        Self {
            models_dir: default_stt_models_dir(),
        }
    }
}

/// Smart Dictation post-processing settings. Off by default: sending the
/// transcript to a model — even a local one — is strictly opt-in, matching
/// the telemetry posture. The backend stays stateless; the frontend passes
/// these per rewrite call.
///
/// Not `Eq`: `overlay_noise_floor` is an f64 (nothing compares whole prefs
/// structs anyway — only individual fields, see `commands::preferences`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DictationPrefs {
    /// "off" | "light" | "smart". Unknown values read as "off".
    #[serde(default = "default_dictation_mode")]
    pub mode: String,

    /// Rewrite provider id — `"ollama"` is the only one today. A string (not
    /// an enum) so a prefs file written by a newer build with more providers
    /// still parses here.
    #[serde(default = "default_dictation_provider")]
    pub provider: String,

    /// Provider base URL. Defaults to the local Ollama server; transcript
    /// text never leaves the machine unless the user points this elsewhere.
    #[serde(default = "default_dictation_endpoint")]
    pub endpoint: String,

    /// Model name. Empty = auto-pick from the provider's installed models.
    #[serde(default)]
    pub model: String,

    /// Push-to-talk: hold the Fn (🌐) key with a dictation field focused to
    /// dictate; release to stop. On by default — it's only an alternate
    /// trigger for the same mic, nothing leaves the machine.
    #[serde(default = "default_true")]
    pub push_to_talk: bool,

    /// User-curated dictation terms ("refactor", "Tailwind", "Shopify") fed
    /// into the rewrite vocabulary AHEAD of every automatic source — these
    /// are the plain words and niche brands macOS dictation garbles that no
    /// harvest can supply (the automatic sources only collect
    /// identifier-shaped tokens by design). Stored backend-side like every
    /// preference; the prompt takes the first few (see
    /// `commands::dictation::CUSTOM_TERMS_CAP`).
    #[serde(default)]
    pub custom_terms: Vec<String>,

    /// Transcription engine: `"macos"` (system dictation types into the
    /// field, the default) or `"local"` (the `portbay-stt` sidecar captures
    /// the mic and runs a downloaded Whisper/Parakeet model on-device). A
    /// string for the same forward-compat reason as `provider`.
    #[serde(default = "default_stt_engine")]
    pub stt_engine: String,

    /// Local STT model (catalog id, e.g. `"parakeet-tdt-v3"`). Only read
    /// when `stt_engine == "local"`. Empty = no model chosen yet, which the
    /// UI treats as "macOS engine until one is picked".
    #[serde(default)]
    pub stt_model: String,

    /// "Dictate anywhere": hold Fn in ANY app and the local engine's
    /// transcript is pasted into it (see `crate::dictation_anywhere`).
    /// Off by default — it needs the Accessibility grant and a local model,
    /// both explicit user choices. Only read when `stt_engine == "local"`.
    #[serde(default)]
    pub anywhere: bool,

    /// Hands-free variant of "dictate anywhere": double-tap Fn to start a
    /// session that stays live without holding the key; a single Fn tap (or
    /// Esc) stops it. On by default within the anywhere opt-in — the gesture
    /// mirrors macOS dictation's own double-press idiom. Off is the escape
    /// hatch for users whose Fn key already double-taps into something else.
    #[serde(default = "default_true")]
    pub anywhere_double_tap: bool,

    /// Where the recording overlay sits: `"notch"` (the camera-housing HUD,
    /// the default) or `"bottom"` (a floating pill near the bottom of the
    /// pointer's screen — the option for Macs without a notch, where the
    /// virtual-notch fallback floats under the menu bar). A string for the
    /// same forward-compat reason as `provider`; unknown values read as
    /// notch.
    #[serde(default = "default_overlay_position")]
    pub overlay_position: String,

    /// Raw mic-RMS floor below which the overlay's waveform stays flat —
    /// FluidVoice's visualizer noise threshold, configurable so the bars
    /// don't dance to a noisy room. Speech RMS sits ~0.01–0.3; the default
    /// matches the previously hardcoded floor. Clamped to 0.0–0.05 on
    /// load/save.
    #[serde(default = "default_overlay_noise_floor")]
    pub overlay_noise_floor: f64,

    /// How much of the live transcript the overlay's preview keeps —
    /// the last N characters (head-truncated, so the newest words are
    /// always visible). FluidVoice's default is 150; clamped to 50–800 on
    /// load/save.
    #[serde(default = "default_overlay_preview_chars")]
    pub overlay_preview_chars: u32,

    /// "Polish dictation everywhere": run the Smart Dictation rewrite engine
    /// over the system-wide ("anywhere") transcript before pasting it, so
    /// rambly speech lands clean and paragraphed in any app — the same engine
    /// (providers + sanitizer + no-invention guards) the in-app surfaces use.
    /// Off by default and only read inside the anywhere opt-in; a failed or
    /// timed-out rewrite degrades to the raw transcript (zero data loss).
    #[serde(default)]
    pub anywhere_polish: bool,

    /// Per-app `RewriteContext` overrides for the polished anywhere path: the
    /// frontmost app's bundle id → context. Resolution is user rule →
    /// built-in default (terminals map to `terminal_command`) → `GeneralNote`,
    /// so an empty list still does the right thing (see
    /// `dictation_anywhere::resolve_context`). A list (not a map) so the
    /// settings UI keeps the user's ordering.
    #[serde(default)]
    pub anywhere_app_contexts: Vec<AppContextRule>,
}

/// One per-app rewrite-context override: when this bundle id is frontmost,
/// the anywhere rewrite uses `context` (a `RewriteContext` wire string —
/// snake_case, e.g. `git_commit`). Unknown context strings fall back to the
/// built-in resolution, so a newer build's context value never breaks here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppContextRule {
    pub bundle_id: String,
    pub context: String,
}

/// Local Ollama manager settings. These map directly to Ollama's supported
/// environment variables when PortBay starts its own `ollama serve`.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiPrefs {
    #[serde(default = "default_dictation_endpoint")]
    pub endpoint: String,
    #[serde(default = "default_ollama_models_dir")]
    pub models_dir: String,
    #[serde(default)]
    pub binary_path: String,
    #[serde(default = "default_ollama_keep_alive")]
    pub keep_alive: String,
    #[serde(default)]
    pub flash_attention: bool,
    #[serde(default = "default_ollama_origins")]
    pub origins: String,
    #[serde(default)]
    pub num_parallel: Option<u32>,
    #[serde(default)]
    pub debug: bool,
    #[serde(default)]
    pub model_download_threads: Option<u32>,
    #[serde(default)]
    pub no_history: bool,
    #[serde(default)]
    pub no_prune: bool,
    #[serde(default)]
    pub schedule_spread: bool,
    #[serde(default)]
    pub multi_user_cache: bool,
    #[serde(default)]
    pub kv_cache_type: String,
    #[serde(default)]
    pub gpu_overhead: Option<u64>,
    #[serde(default)]
    pub load_timeout: String,
    #[serde(default)]
    pub max_loaded_models: Option<u32>,
    #[serde(default)]
    pub max_queue: Option<u32>,
    #[serde(default)]
    pub llm_library: String,
    #[serde(default)]
    pub http_proxy: String,
    #[serde(default)]
    pub https_proxy: String,
    #[serde(default)]
    pub no_proxy: String,
}

impl Default for AiPrefs {
    fn default() -> Self {
        Self {
            endpoint: default_dictation_endpoint(),
            models_dir: default_ollama_models_dir(),
            binary_path: String::new(),
            keep_alive: default_ollama_keep_alive(),
            flash_attention: false,
            origins: default_ollama_origins(),
            num_parallel: None,
            debug: false,
            model_download_threads: None,
            no_history: false,
            no_prune: false,
            schedule_spread: false,
            multi_user_cache: false,
            kv_cache_type: String::new(),
            gpu_overhead: None,
            load_timeout: String::new(),
            max_loaded_models: None,
            max_queue: None,
            llm_library: String::new(),
            http_proxy: String::new(),
            https_proxy: String::new(),
            no_proxy: String::new(),
        }
    }
}

impl Default for DictationPrefs {
    fn default() -> Self {
        Self {
            mode: default_dictation_mode(),
            provider: default_dictation_provider(),
            endpoint: default_dictation_endpoint(),
            model: String::new(),
            push_to_talk: true,
            custom_terms: Vec::new(),
            stt_engine: default_stt_engine(),
            stt_model: String::new(),
            anywhere: false,
            anywhere_double_tap: true,
            overlay_position: default_overlay_position(),
            overlay_noise_floor: default_overlay_noise_floor(),
            overlay_preview_chars: default_overlay_preview_chars(),
            anywhere_polish: false,
            anywhere_app_contexts: Vec::new(),
        }
    }
}

// Smart by default via the Apple provider (matches DEFAULTS.dictation in
// src/lib/stores/preferences.svelte.ts — the backend materializes these
// serde defaults into `get_preferences`, so the frontend's defaults never
// apply on their own). On machines without Apple Intelligence the rewrite
// resolves `no_model` and the frontend latches the provider off for the
// session, silently — dictation keeps working raw.
fn default_dictation_mode() -> String {
    "smart".to_string()
}

fn default_dictation_provider() -> String {
    "apple".to_string()
}

fn default_dictation_endpoint() -> String {
    "http://127.0.0.1:11434".to_string()
}

// One PortBay-recommended AI models root with per-engine subdirectories —
// Ollama and speech-to-text downloads live side by side so a single
// "Download location" knob (the AI page sets both prefs from one folder
// pick) manages everything. Safe to brand: the AI manager ships first in
// this release, so no existing install has models at an older default.
fn default_ollama_models_dir() -> String {
    dirs::data_dir()
        .map(|p| p.join("PortBay/ai-models/ollama").to_string_lossy().into_owned())
        .unwrap_or_else(|| "~/Library/Application Support/PortBay/ai-models/ollama".to_string())
}

// macOS dictation stays the transcription default: zero download, zero
// setup, and the local engine needs a model installed before it can work.
fn default_stt_engine() -> String {
    "macos".to_string()
}

// The notch HUD is the overlay's identity; the bottom pill is the opt-in
// for non-notch Macs.
fn default_overlay_position() -> String {
    "notch".to_string()
}

// The previously hardcoded RMS_FLOOR in the overlay webview.
fn default_overlay_noise_floor() -> f64 {
    0.01
}

// FluidVoice's preview tail default.
fn default_overlay_preview_chars() -> u32 {
    150
}

fn default_stt_models_dir() -> String {
    dirs::data_dir()
        .map(|p| p.join("PortBay/ai-models/speech").to_string_lossy().into_owned())
        .unwrap_or_else(|| "~/Library/Application Support/PortBay/ai-models/speech".to_string())
}

fn default_ollama_keep_alive() -> String {
    "5m".to_string()
}

fn default_ollama_origins() -> String {
    "http://localhost,https://localhost,http://127.0.0.1,https://127.0.0.1".to_string()
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

fn default_quiet_start() -> String {
    "22:00".to_string()
}

fn default_quiet_end() -> String {
    "07:00".to_string()
}

fn default_notification_schema_version() -> u32 {
    1
}

fn default_notification_channels() -> BTreeMap<NotificationCategory, NotificationChannelPrefs> {
    NotificationCategory::ALL
        .into_iter()
        .map(|category| (category, NotificationChannelPrefs::for_category(category)))
        .collect()
}

fn default_cue_for_category(category: NotificationCategory) -> NotificationCue {
    match category {
        NotificationCategory::AgentBoard => NotificationCue::Comment,
        NotificationCategory::ProjectError | NotificationCategory::Crash => NotificationCue::Error,
        _ => NotificationCue::Done,
    }
}

fn default_notification_cues() -> BTreeMap<NotificationCategory, NotificationCue> {
    NotificationCategory::ALL
        .into_iter()
        .map(|category| (category, default_cue_for_category(category)))
        .collect()
}

fn default_agent_event_cue() -> NotificationCue {
    NotificationCue::Comment
}

fn default_agent_sound_setting(event: AgentSoundEvent) -> AgentSoundSetting {
    match event {
        AgentSoundEvent::Done => AgentSoundSetting {
            enabled: true,
            cue: NotificationCue::Done,
        },
        AgentSoundEvent::Error => AgentSoundSetting {
            enabled: true,
            cue: NotificationCue::Error,
        },
        AgentSoundEvent::Comment => AgentSoundSetting {
            enabled: true,
            cue: NotificationCue::Comment,
        },
        // A recorded learning is informational; default its sound off so it
        // doesn't interrupt — it still lands in the bell.
        AgentSoundEvent::Learning => AgentSoundSetting {
            enabled: false,
            cue: NotificationCue::Attention,
        },
    }
}

fn default_agent_sound_events() -> BTreeMap<AgentSoundEvent, AgentSoundSetting> {
    AgentSoundEvent::ALL
        .into_iter()
        .map(|event| (event, default_agent_sound_setting(event)))
        .collect()
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            show_tray_icon: true,
            show_dock_icon: true,
            close_to_menu_bar: true,
            close_to_menu_bar_toast_seen: false,
            telemetry_enabled: false,
            telemetry_consent_prompted: false,
            early_access_opt_in: false,
            launch_at_login: false,
            reopen_previous_projects: false,
            confirm_before_stop_all: true,
            desktop_notifications: false,
            notifications: NotificationPrefs::default(),
            accessibility: AccessibilityPrefs::default(),
            accent_color: default_accent_color(),
            default_workspace_folder: String::new(),
            auto_detect_projects: false,
            default_sort: default_sort(),
            default_start_behavior: default_start_behavior(),
            default_web_server: None,
            preferred_terminal: None,
            preferred_agent: None,
            agent_paths: BTreeMap::new(),
            agent_launch_modes: BTreeMap::new(),
            manage_hosts_automatically: true,
            auto_renew_certificates: true,
            store_logs_locally: true,
            log_retention_days: default_log_retention_days(),
            cli_path: default_cli_path(),
            auto_clean_schedule: default_auto_clean_schedule(),
            last_auto_clean: 0,
            auto_clean_extra_dirs: Vec::new(),
            ai: AiPrefs::default(),
            dictation: DictationPrefs::default(),
            stt: SttPrefs::default(),
        }
    }
}

impl Preferences {
    pub fn normalise_ai_endpoint(mut self) -> Self {
        let default_endpoint = default_dictation_endpoint();
        if self.ai.endpoint.trim().is_empty()
            || (self.ai.endpoint == default_endpoint && self.dictation.endpoint != default_endpoint)
        {
            self.ai.endpoint = if self.dictation.endpoint.trim().is_empty() {
                default_dictation_endpoint()
            } else {
                self.dictation.endpoint.clone()
            };
        }
        self.dictation.endpoint = self.ai.endpoint.clone();
        self
    }

    /// Clamp the dictation-overlay knobs into their documented ranges (a
    /// hand-edited prefs file must not produce a permanently-flat waveform
    /// or an unbounded preview). Applied on every load and save, like the
    /// endpoint mirror.
    pub fn normalise_dictation_overlay(mut self) -> Self {
        if self.dictation.overlay_position != "bottom" {
            self.dictation.overlay_position = default_overlay_position();
        }
        let floor = self.dictation.overlay_noise_floor;
        self.dictation.overlay_noise_floor = if floor.is_finite() {
            floor.clamp(0.0, 0.05)
        } else {
            default_overlay_noise_floor()
        };
        self.dictation.overlay_preview_chars = self.dictation.overlay_preview_chars.clamp(50, 800);
        self
    }

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
            Ok(mut prefs) => {
                let missing_notifications = serde_json::from_str::<serde_json::Value>(&raw)
                    .ok()
                    .and_then(|v| v.as_object().map(|o| !o.contains_key("notifications")))
                    .unwrap_or(false);
                prefs.notifications = if missing_notifications {
                    NotificationPrefs::default()
                        .with_legacy_desktop(prefs.desktop_notifications)
                        .normalised()
                } else {
                    prefs.notifications.normalised()
                };
                prefs.normalise_ai_endpoint().normalise_dictation_overlay()
            }
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
        let serialised =
            serde_json::to_vec_pretty(&self.clone().normalise_ai_endpoint().normalise_dictation_overlay())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        std::fs::write(&tmp, &serialised)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }
}

pub fn notification_allowed(
    prefs: &NotificationPrefs,
    category: NotificationCategory,
    severity: NotificationSeverity,
    channel: NotificationChannel,
    now_ms: u64,
) -> bool {
    prefs.channel_enabled(category, channel)
        && passes_severity_floor(severity, prefs.severity_floor)
        && !notification_suppressed(prefs, severity, now_ms)
}

pub fn notification_suppressed(
    prefs: &NotificationPrefs,
    severity: NotificationSeverity,
    now_ms: u64,
) -> bool {
    if prefs.quiet_hours.exempt_errors && severity == NotificationSeverity::Error {
        return false;
    }
    if let Some(until) = prefs.snooze_until {
        if now_ms < until {
            return true;
        }
    }
    if !prefs.quiet_hours.enabled {
        return false;
    }
    quiet_hours_active(&prefs.quiet_hours, now_ms)
}

fn passes_severity_floor(severity: NotificationSeverity, floor: NotificationSeverityFloor) -> bool {
    match floor {
        NotificationSeverityFloor::Everything => true,
        NotificationSeverityFloor::ErrorsAndWarnings => {
            matches!(
                severity,
                NotificationSeverity::Error | NotificationSeverity::Warning
            )
        }
        NotificationSeverityFloor::ErrorsOnly => severity == NotificationSeverity::Error,
    }
}

fn quiet_hours_active(quiet: &NotificationQuietHours, now_ms: u64) -> bool {
    let Some(start) = parse_hh_mm(&quiet.start) else {
        return false;
    };
    let Some(end) = parse_hh_mm(&quiet.end) else {
        return false;
    };
    if start == end {
        return true;
    }
    let minute = local_minute_of_day(now_ms);
    if start < end {
        minute >= start && minute < end
    } else {
        minute >= start || minute < end
    }
}

fn parse_hh_mm(value: &str) -> Option<u32> {
    let (h, m) = value.split_once(':')?;
    let hour: u32 = h.parse().ok()?;
    let minute: u32 = m.parse().ok()?;
    if hour > 23 || minute > 59 {
        return None;
    }
    Some(hour * 60 + minute)
}

fn local_minute_of_day(now_ms: u64) -> u32 {
    let secs = (now_ms / 1_000) as i64;
    let local = chrono::DateTime::<chrono::Local>::from(
        std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs.max(0) as u64),
    );
    local.hour() * 60 + local.minute()
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
        assert!(!p.telemetry_consent_prompted);
        assert_eq!(p.notifications.schema_version, 1);
        assert!(p.notifications.channel_enabled(
            NotificationCategory::ProjectError,
            NotificationChannel::Toast
        ));
        assert!(p
            .notifications
            .channel_enabled(NotificationCategory::AgentBoard, NotificationChannel::Sound));
        assert_eq!(p.accessibility, AccessibilityPrefs::default());
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
        assert!(!p.telemetry_consent_prompted);
        // New default-web-server preference is absent in old files → None,
        // which `Project::web_server_effective` reads as Caddy.
        assert_eq!(p.default_web_server, None);
        assert_eq!(
            p.notifications.severity_floor,
            NotificationSeverityFloor::Everything
        );
        assert_eq!(p.accessibility.text_scale, AccessibilityTextScale::Normal);
        // Custom dictation terms are new — old prefs files read as none.
        assert!(p.dictation.custom_terms.is_empty());
        // STT fields are new — old prefs files read as the macOS engine
        // with the default models home.
        assert_eq!(p.dictation.stt_engine, "macos");
        assert!(p.dictation.stt_model.is_empty());
        assert!(p.stt.models_dir.ends_with("PortBay/ai-models/speech"));
        // Overlay knobs are new — old prefs files read as the previous
        // hardcoded behavior (notch HUD, 0.01 floor, 150-char tail).
        assert_eq!(p.dictation.overlay_position, "notch");
        assert_eq!(p.dictation.overlay_noise_floor, 0.01);
        assert_eq!(p.dictation.overlay_preview_chars, 150);
    }

    #[test]
    fn overlay_knobs_clamp_on_normalise() {
        let mut p = Preferences::default();
        p.dictation.overlay_position = "sideways".to_string();
        p.dictation.overlay_noise_floor = 7.5;
        p.dictation.overlay_preview_chars = 12;
        let p = p.normalise_dictation_overlay();
        assert_eq!(p.dictation.overlay_position, "notch");
        assert_eq!(p.dictation.overlay_noise_floor, 0.05);
        assert_eq!(p.dictation.overlay_preview_chars, 50);

        let mut p = Preferences::default();
        p.dictation.overlay_position = "bottom".to_string();
        p.dictation.overlay_noise_floor = f64::NAN;
        p.dictation.overlay_preview_chars = 5000;
        let p = p.normalise_dictation_overlay();
        assert_eq!(p.dictation.overlay_position, "bottom");
        assert_eq!(p.dictation.overlay_noise_floor, 0.01);
        assert_eq!(p.dictation.overlay_preview_chars, 800);
    }

    #[test]
    fn round_trip_camel_case() {
        let p = Preferences {
            show_tray_icon: false,
            show_dock_icon: true,
            close_to_menu_bar: true,
            close_to_menu_bar_toast_seen: true,
            telemetry_enabled: true,
            telemetry_consent_prompted: true,
            early_access_opt_in: true,
            launch_at_login: true,
            reopen_previous_projects: true,
            confirm_before_stop_all: false,
            desktop_notifications: true,
            notifications: NotificationPrefs {
                schema_version: 1,
                channels: BTreeMap::from([(
                    NotificationCategory::AgentBoard,
                    NotificationChannelPrefs {
                        toast: false,
                        bell: true,
                        banner: true,
                        sound: true,
                    },
                )]),
                severity_floor: NotificationSeverityFloor::ErrorsAndWarnings,
                quiet_hours: NotificationQuietHours {
                    enabled: true,
                    start: "21:30".to_string(),
                    end: "06:45".to_string(),
                    exempt_errors: true,
                },
                snooze_until: Some(1_800_000_000_000),
                sound: NotificationSoundPrefs::default(),
            },
            accessibility: AccessibilityPrefs {
                reduce_motion: true,
                reduce_transparency: true,
                high_contrast: true,
                text_scale: AccessibilityTextScale::Large,
                focus_mode: AccessibilityFocusMode::Strong,
                underline_links: true,
                color_independent_status: true,
            },
            accent_color: "purple".to_string(),
            default_workspace_folder: "/Users/dev/Projects".to_string(),
            auto_detect_projects: true,
            default_sort: "status".to_string(),
            default_start_behavior: "auto".to_string(),
            default_web_server: Some(WebServer::Nginx),
            preferred_terminal: Some("warp".to_string()),
            preferred_agent: Some("codex".to_string()),
            agent_paths: BTreeMap::from([(
                "codex".to_string(),
                "/Volumes/Ext/bin/codex".to_string(),
            )]),
            agent_launch_modes: BTreeMap::from([("codex".to_string(), "app".to_string())]),
            manage_hosts_automatically: false,
            auto_renew_certificates: false,
            store_logs_locally: false,
            log_retention_days: 30,
            cli_path: "/opt/local/bin/portbay".to_string(),
            auto_clean_schedule: "weekly".to_string(),
            last_auto_clean: 1_700_000_000,
            auto_clean_extra_dirs: vec![".turbo".to_string(), ".cache".to_string()],
            ai: AiPrefs::default(),
            dictation: DictationPrefs {
                mode: "smart".to_string(),
                provider: "ollama".to_string(),
                endpoint: "http://127.0.0.1:11434".to_string(),
                model: "qwen2.5:3b".to_string(),
                push_to_talk: false,
                custom_terms: vec!["refactor".to_string(), "Tailwind".to_string()],
                stt_engine: "local".to_string(),
                stt_model: "parakeet-tdt-v3".to_string(),
                anywhere: true,
                anywhere_double_tap: true,
                overlay_position: "bottom".to_string(),
                overlay_noise_floor: 0.02,
                overlay_preview_chars: 300,
                anywhere_polish: true,
                anywhere_app_contexts: vec![AppContextRule {
                    bundle_id: "com.apple.Terminal".to_string(),
                    context: "terminal_command".to_string(),
                }],
            },
            stt: SttPrefs {
                models_dir: "/Volumes/DevSSD/system/ai/stt".to_string(),
            },
        };
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("\"showTrayIcon\":false"));
        assert!(json.contains("\"showDockIcon\":true"));
        assert!(json.contains("\"earlyAccessOptIn\":true"));
        assert!(json.contains("\"telemetryConsentPrompted\":true"));
        assert!(json.contains("\"closeToMenuBar\":true"));
        assert!(json.contains("\"launchAtLogin\":true"));
        assert!(json.contains("\"notifications\""));
        assert!(json.contains("\"severityFloor\":\"errors_and_warnings\""));
        assert!(json.contains("\"agent-board\""));
        assert!(json.contains("\"accessibility\""));
        assert!(json.contains("\"textScale\":\"large\""));
        assert!(json.contains("\"focusMode\":\"strong\""));
        assert!(json.contains("\"accentColor\":\"purple\""));
        assert!(json.contains("\"logRetentionDays\":30"));
        assert!(json.contains("\"autoCleanSchedule\":\"weekly\""));
        assert!(json.contains("\"lastAutoClean\":1700000000"));
        assert!(json.contains("\"defaultWebServer\":\"nginx\""));
        assert!(json.contains("\"preferredTerminal\":\"warp\""));
        assert!(json.contains("\"preferredAgent\":\"codex\""));
        assert!(json.contains("\"dictation\""));
        assert!(json.contains("\"mode\":\"smart\""));
        assert!(json.contains("\"pushToTalk\":false"));
        assert!(json.contains("\"customTerms\":[\"refactor\",\"Tailwind\"]"));
        assert!(json.contains("\"sttEngine\":\"local\""));
        assert!(json.contains("\"sttModel\":\"parakeet-tdt-v3\""));
        assert!(json.contains("\"stt\""));
        assert!(json.contains("\"modelsDir\":\"/Volumes/DevSSD/system/ai/stt\""));
        assert!(json.contains("\"agentPaths\":{\"codex\":\"/Volumes/Ext/bin/codex\"}"));
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

    #[test]
    fn ai_endpoint_is_single_source_for_dictation() {
        let raw = r#"{
          "dictation": {
            "provider": "ollama",
            "endpoint": "http://127.0.0.1:11500",
            "model": "qwen2.5:7b"
          }
        }"#;
        let p: Preferences = serde_json::from_str(raw).unwrap();
        let normalised = p.normalise_ai_endpoint();
        assert_eq!(normalised.ai.endpoint, "http://127.0.0.1:11500");
        assert_eq!(normalised.dictation.endpoint, normalised.ai.endpoint);

        let mut explicit = Preferences::default();
        explicit.ai.endpoint = "http://127.0.0.1:11600".to_string();
        explicit.dictation.endpoint = "http://127.0.0.1:11434".to_string();
        let normalised = explicit.normalise_ai_endpoint();
        assert_eq!(normalised.ai.endpoint, "http://127.0.0.1:11600");
        assert_eq!(normalised.dictation.endpoint, "http://127.0.0.1:11600");
    }

    #[test]
    fn partial_notification_prefs_backfill_defaults() {
        let raw = r#"{
          "notifications": {
            "channels": {
              "updates": { "bell": false }
            },
            "quietHours": { "enabled": true }
          }
        }"#;
        let mut p: Preferences = serde_json::from_str(raw).unwrap();
        p.notifications = p.notifications.normalised();
        assert!(!p
            .notifications
            .channel_enabled(NotificationCategory::Updates, NotificationChannel::Bell));
        assert!(p.notifications.channel_enabled(
            NotificationCategory::ProjectError,
            NotificationChannel::Toast
        ));
        assert_eq!(p.notifications.quiet_hours.start, "22:00");
        assert_eq!(p.notifications.quiet_hours.end, "07:00");
    }

    #[test]
    fn severity_floor_filters_lower_severities() {
        let prefs = NotificationPrefs {
            severity_floor: NotificationSeverityFloor::ErrorsOnly,
            ..NotificationPrefs::default()
        };
        assert!(notification_allowed(
            &prefs,
            NotificationCategory::ProjectError,
            NotificationSeverity::Error,
            NotificationChannel::Toast,
            0,
        ));
        assert!(!notification_allowed(
            &prefs,
            NotificationCategory::ProjectError,
            NotificationSeverity::Warning,
            NotificationChannel::Toast,
            0,
        ));
    }

    #[test]
    fn quiet_hours_crossing_midnight_suppress_non_errors() {
        let prefs = NotificationPrefs {
            quiet_hours: NotificationQuietHours {
                enabled: true,
                start: "22:00".to_string(),
                end: "07:00".to_string(),
                exempt_errors: true,
            },
            ..NotificationPrefs::default()
        };
        let two_am_local = local_now_for_minute(2 * 60);
        assert!(notification_suppressed(
            &prefs,
            NotificationSeverity::Warning,
            two_am_local
        ));
        assert!(!notification_suppressed(
            &prefs,
            NotificationSeverity::Error,
            two_am_local
        ));
    }

    #[test]
    fn snooze_suppresses_until_it_expires() {
        let prefs = NotificationPrefs {
            snooze_until: Some(20_000),
            ..NotificationPrefs::default()
        };
        assert!(notification_suppressed(
            &prefs,
            NotificationSeverity::Warning,
            10_000
        ));
        assert!(!notification_suppressed(
            &prefs,
            NotificationSeverity::Warning,
            20_000
        ));
    }

    fn local_now_for_minute(minute_of_day: u32) -> u64 {
        let now = chrono::Local::now();
        let midnight = now
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_local_timezone(chrono::Local)
            .single()
            .unwrap();
        (midnight.timestamp_millis() as u64) + (minute_of_day as u64 * 60_000)
    }
}
