//! Phase-aware run state for mobile projects.
//!
//! The 6-state web taxonomy can't express a mobile run: with process
//! readiness, a row flips to "Running" while `xcodebuild` is still compiling.
//! This module derives a truthful sub-state — `resolving-device → booting →
//! building → installing → launching → connected` — and emits it on
//! `portbay://mobile-phase` *alongside* `PortbayStatus` (additive; the base
//! taxonomy is untouched, so no consumer churn).
//!
//! Mechanism: PortBay owns the generated launch scripts (`crate::mobile`), so
//! the iOS/Android scripts emit line-anchored `::portbay::phase=<p>` markers
//! between steps. Flutter and Expo phases are inferred from their CLIs' own
//! well-known milestone lines. The status poller (`commands::events`) calls
//! [`observe`] every tick for each mobile project; we incrementally read the
//! project's Process Compose log file from a per-project cursor and fold new
//! lines into the phase. Markers are namespaced and line-anchored so app log
//! output can't spoof a phase by mentioning one mid-line.
//!
//! State lives in a module-level map (not `AppState`) — it's a derived cache
//! of the log files, owned entirely by this module; `get_mobile_phases` lets
//! the frontend hydrate after a reload.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::process_compose::ProjectStatus;
use crate::registry::ProjectType;

pub const PHASE_CHANNEL: &str = "portbay://mobile-phase";

/// How long `simctl launch --console-pty` must stay attached after the
/// `launching` marker before we call the run Connected. The launch command
/// exits quickly (non-zero) when the app fails to start; staying attached is
/// the "the app is running and we hold its console" signal. Android, Flutter
/// and Expo all have explicit connected milestones instead.
const IOS_LAUNCH_SETTLE_SECS: u64 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MobilePhase {
    ResolvingDevice,
    BootingDevice,
    Building,
    Installing,
    Launching,
    Connected,
    BuildFailed,
}

impl MobilePhase {
    fn label(self) -> &'static str {
        match self {
            MobilePhase::ResolvingDevice => "resolving device",
            MobilePhase::BootingDevice => "booting device",
            MobilePhase::Building => "building",
            MobilePhase::Installing => "installing",
            MobilePhase::Launching => "launching",
            MobilePhase::Connected => "connected",
            MobilePhase::BuildFailed => "build failed",
        }
    }
}

/// Emitted on [`PHASE_CHANNEL`] whenever a project's phase changes. `phase:
/// null` clears the sub-state (project stopped).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MobilePhaseEvent {
    pub id: String,
    pub phase: Option<MobilePhase>,
    /// Free-form context: the resolved device id, or the step that failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub ts: u64,
}

#[derive(Debug)]
struct ProjState {
    cursor: u64,
    phase: Option<MobilePhase>,
    detail: Option<String>,
    /// Actionable cause recognized in provider output (e.g. a locked phone),
    /// folded into the failure detail when the run dies. First hit wins —
    /// the earliest recognized line is closest to the root cause.
    hint: Option<&'static str>,
    /// When the current phase was entered (drives the iOS launch-settle rule).
    entered_at: Instant,
}

impl Default for ProjState {
    fn default() -> Self {
        Self {
            cursor: 0,
            phase: None,
            detail: None,
            hint: None,
            entered_at: Instant::now(),
        }
    }
}

static TRACKER: Mutex<Option<HashMap<String, ProjState>>> = Mutex::new(None);

fn with_tracker<R>(f: impl FnOnce(&mut HashMap<String, ProjState>) -> R) -> R {
    let mut guard = TRACKER.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(HashMap::new))
}

/// Snapshot for `get_mobile_phases` — lets the frontend hydrate on mount.
pub fn snapshot() -> HashMap<String, (MobilePhase, Option<String>)> {
    with_tracker(|t| {
        t.iter()
            .filter_map(|(id, s)| s.phase.map(|p| (id.clone(), (p, s.detail.clone()))))
            .collect()
    })
}

/// Fold one poller observation for a mobile project into its phase state and
/// emit on change. Called every poller tick (750 ms) per running mobile
/// project — the incremental read makes that cheap (a seek + the new bytes).
pub fn observe(
    app: &AppHandle,
    logs_dir: &Path,
    id: &str,
    kind: ProjectType,
    status: ProjectStatus,
) {
    let change = with_tracker(|t| fold(t, logs_dir, id, kind, status));
    if let Some((phase, detail)) = change {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let _ = app.emit(
            PHASE_CHANNEL,
            MobilePhaseEvent {
                id: id.to_string(),
                phase,
                detail,
                ts,
            },
        );
    }
}

/// Pure-ish core (only file I/O). Returns the new `(phase, detail)` when it
/// changed, `None` when nothing should be emitted.
#[allow(clippy::type_complexity)]
fn fold(
    tracker: &mut HashMap<String, ProjState>,
    logs_dir: &Path,
    id: &str,
    kind: ProjectType,
    status: ProjectStatus,
) -> Option<(Option<MobilePhase>, Option<String>)> {
    match status {
        ProjectStatus::Stopped => {
            // Clear on stop — but keep a BuildFailed verdict visible until the
            // next run so the user can still see *why* it died.
            let keep_failed = tracker
                .get(id)
                .is_some_and(|s| s.phase == Some(MobilePhase::BuildFailed));
            if keep_failed {
                return None;
            }
            let had_phase = tracker.remove(id).is_some_and(|s| s.phase.is_some());
            had_phase.then_some((None, None))
        }
        ProjectStatus::Crashed => {
            let state = tracker.entry(id.to_string()).or_default();
            // Drain any tail output first so the last phase is accurate.
            drain_new_lines(state, logs_dir, id, kind);
            match state.phase {
                Some(MobilePhase::Connected) | Some(MobilePhase::BuildFailed) | None => None,
                Some(failed_during) => {
                    state.detail = Some(match state.hint {
                        Some(hint) => format!("failed while {}: {hint}", failed_during.label()),
                        None => format!("failed while {}", failed_during.label()),
                    });
                    state.phase = Some(MobilePhase::BuildFailed);
                    state.entered_at = Instant::now();
                    Some((state.phase, state.detail.clone()))
                }
            }
        }
        // Starting / Running / Unhealthy / PortConflict — the process is (or
        // should be) alive; read new output and advance the phase.
        _ => {
            let state = tracker.entry(id.to_string()).or_default();
            let before = (state.phase, state.detail.clone());
            drain_new_lines(state, logs_dir, id, kind);

            // iOS has no post-`exec` marker: `launching` + still attached past
            // the settle window ⇒ connected.
            if kind == ProjectType::Xcode
                && state.phase == Some(MobilePhase::Launching)
                && state.entered_at.elapsed().as_secs() >= IOS_LAUNCH_SETTLE_SECS
            {
                set_phase(state, MobilePhase::Connected);
            }

            // A run that just (re)started with no marker yet: surface
            // resolving-device immediately so the pill never lies "Running".
            if state.phase.is_none() || state.phase == Some(MobilePhase::BuildFailed) {
                set_phase(state, MobilePhase::ResolvingDevice);
                state.detail = None;
                state.hint = None;
            }

            let after = (state.phase, state.detail.clone());
            (after != before).then_some(after)
        }
    }
}

/// Read everything new past the cursor and fold each complete line. Handles
/// truncation (project restart rewrites the log) by resetting to a fresh run.
fn drain_new_lines(state: &mut ProjState, logs_dir: &Path, id: &str, kind: ProjectType) {
    let path = logs_dir.join(format!("{id}.log"));
    let Ok(mut file) = std::fs::File::open(&path) else {
        return; // PC creates the file lazily; try again next tick.
    };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    if len < state.cursor {
        // Truncated — a new run started. Reset to a clean slate.
        state.cursor = 0;
        state.phase = None;
        state.detail = None;
        state.hint = None;
        state.entered_at = Instant::now();
    }
    if len == state.cursor {
        return;
    }
    if file.seek(SeekFrom::Start(state.cursor)).is_err() {
        return;
    }
    let mut buf = String::new();
    let mut take = file.take(len - state.cursor);
    if take.read_to_string(&mut buf).is_err() {
        // Mid-line UTF-8 boundary or transient error — retry next tick.
        return;
    }
    // Only consume complete lines; a partial trailing line stays for the next
    // tick so markers are never split.
    let consumed = match buf.rfind('\n') {
        Some(i) => i + 1,
        None => return,
    };
    for line in buf[..consumed].lines() {
        fold_line(state, kind, line);
    }
    state.cursor += consumed as u64;
}

fn set_phase(state: &mut ProjState, phase: MobilePhase) {
    if state.phase != Some(phase) {
        state.phase = Some(phase);
        state.entered_at = Instant::now();
    }
}

/// Fold one raw log line (possibly PC's JSON envelope) into the state.
fn fold_line(state: &mut ProjState, kind: ProjectType, raw: &str) {
    let line = crate::commands::events::extract_pc_message(raw).unwrap_or_else(|| raw.to_string());
    let line = line.trim();

    if let Some(device) = line.strip_prefix("::portbay::device=") {
        state.detail = Some(device.trim().to_string());
        return;
    }
    if let Some(phase) = parse_marker(line) {
        set_phase(state, phase);
        return;
    }
    if let Some(phase) = parse_milestone(kind, line) {
        // Milestones only move forward (CLI output can repeat earlier-sounding
        // lines, e.g. Gradle output mid-sync) — never demote Connected.
        if state.phase != Some(MobilePhase::Connected) || phase == MobilePhase::Connected {
            set_phase(state, phase);
        }
        return;
    }
    if state.hint.is_none() {
        state.hint = parse_hint(line);
    }
}

/// Actionable causes recognized in provider output (devicectl, xcodebuild,
/// ios-deploy, `flutter run`), matched case-insensitively as substrings.
/// Today: a locked iPhone — devicectl fails install/launch with a CoreDevice
/// "locked"/"passcode protected" error and, unlike Xcode, shows no unlock
/// dialog, so the pill must carry the instruction. The signature list is
/// best-effort (provider wording varies by Xcode release); extend as real
/// logs surface new variants. Android needs no entry: adb installs and
/// launches fine through a locked screen.
pub(crate) fn parse_hint(line: &str) -> Option<&'static str> {
    let l = line.to_ascii_lowercase();
    if l.contains("device is locked")
        || l.contains("passcode protected")
        || l.contains("could not be, unlocked")
        || l.contains("please unlock")
    {
        return Some("device locked — unlock the phone, then press Play again");
    }
    None
}

/// Line-anchored `::portbay::phase=<p>` markers from our own scripts.
pub(crate) fn parse_marker(line: &str) -> Option<MobilePhase> {
    let value = line.strip_prefix("::portbay::phase=")?;
    match value.trim() {
        "resolving-device" => Some(MobilePhase::ResolvingDevice),
        "booting-device" => Some(MobilePhase::BootingDevice),
        "building" => Some(MobilePhase::Building),
        "installing" => Some(MobilePhase::Installing),
        "launching" => Some(MobilePhase::Launching),
        "connected" => Some(MobilePhase::Connected),
        _ => None,
    }
}

/// Milestones from CLIs whose output we don't control (Flutter / Expo).
/// Anchored to known prefixes of those tools' progress lines.
pub(crate) fn parse_milestone(kind: ProjectType, line: &str) -> Option<MobilePhase> {
    match kind {
        ProjectType::Flutter => {
            if (line.starts_with("Launching ") && line.contains(" on "))
                || line.starts_with("Running Gradle task")
                || line.starts_with("Running Xcode build")
                || line.starts_with("Building ")
            {
                Some(MobilePhase::Building)
            } else if line.starts_with("Installing and launching")
                || line.starts_with("Installing build")
            {
                Some(MobilePhase::Installing)
            } else if line.starts_with("Syncing files to device")
                || line.contains("Dart VM Service") && line.contains("available at")
                || line.starts_with("Flutter run key commands")
                // App-side prints are forwarded as "flutter: …" only while the
                // tool is attached to the running app — the strongest possible
                // Connected signal. On physical iOS devices the VM-service /
                // syncing lines can be swallowed by the \r-overwritten progress
                // spinner (observed: kitabi 2026-06-11), leaving the pill stuck
                // on "Installing…" while the app is plainly up and logging.
                || line.starts_with("flutter: ")
            {
                Some(MobilePhase::Connected)
            } else {
                None
            }
        }
        ProjectType::Expo => {
            if line.starts_with("Starting Metro Bundler") || line.starts_with("Starting project at")
            {
                Some(MobilePhase::Building)
            } else if line.starts_with("Waiting on http")
                || line.contains("Metro waiting on")
                || line.starts_with("Opening on iOS")
                || line.starts_with("Opening on Android")
            {
                Some(MobilePhase::Launching)
            } else if line.contains("Bundled ") {
                Some(MobilePhase::Connected)
            } else {
                None
            }
        }
        // iOS / Android phases come exclusively from our own markers.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_parse_and_reject_spoofing() {
        assert_eq!(
            parse_marker("::portbay::phase=building"),
            Some(MobilePhase::Building)
        );
        assert_eq!(
            parse_marker("::portbay::phase=connected"),
            Some(MobilePhase::Connected)
        );
        // Mid-line mentions never match (markers are line-anchored upstream
        // via strip_prefix on the trimmed line).
        assert_eq!(parse_marker("app says ::portbay::phase=building"), None);
        assert_eq!(parse_marker("::portbay::phase=nonsense"), None);
    }

    #[test]
    fn flutter_milestones_map_to_phases() {
        let k = ProjectType::Flutter;
        assert_eq!(
            parse_milestone(k, "Launching lib/main.dart on iPhone 16 in debug mode..."),
            Some(MobilePhase::Building)
        );
        assert_eq!(
            parse_milestone(k, "Running Gradle task 'assembleDebug'..."),
            Some(MobilePhase::Building)
        );
        assert_eq!(
            parse_milestone(k, "Installing and launching..."),
            Some(MobilePhase::Installing)
        );
        assert_eq!(
            parse_milestone(k, "Syncing files to device iPhone 16..."),
            Some(MobilePhase::Connected)
        );
        assert_eq!(
            parse_milestone(
                k,
                "A Dart VM Service on iPhone 16 is available at: http://127.0.0.1:50012/abc/"
            ),
            Some(MobilePhase::Connected)
        );
        assert_eq!(parse_milestone(k, "random app output"), None);
        // App-side prints prove the tool is attached to the running app —
        // Connected even when the VM-service line was eaten by the spinner.
        assert_eq!(
            parse_milestone(k, "flutter: 2026-06-11 02:49:50.405 Logger initialized"),
            Some(MobilePhase::Connected)
        );
        // Mid-line mentions don't count (line-anchored prefix).
        assert_eq!(parse_milestone(k, "note: flutter: is great"), None);
    }

    #[test]
    fn expo_milestones_map_to_phases() {
        let k = ProjectType::Expo;
        assert_eq!(
            parse_milestone(k, "Starting Metro Bundler"),
            Some(MobilePhase::Building)
        );
        assert_eq!(
            parse_milestone(k, "Waiting on http://localhost:8081"),
            Some(MobilePhase::Launching)
        );
        assert_eq!(
            parse_milestone(k, "Opening on iOS..."),
            Some(MobilePhase::Launching)
        );
        assert_eq!(
            parse_milestone(k, "iOS Bundled 2387ms node_modules/expo/AppEntry.js"),
            Some(MobilePhase::Connected)
        );
    }

    #[test]
    fn fold_line_unwraps_pc_envelope_and_tracks_device() {
        let mut s = ProjState::default();
        fold_line(
            &mut s,
            ProjectType::Xcode,
            r#"{"level":"info","process":"kitabi","message":"::portbay::phase=building"}"#,
        );
        assert_eq!(s.phase, Some(MobilePhase::Building));
        fold_line(&mut s, ProjectType::Xcode, "::portbay::device=AAAA-1111");
        assert_eq!(s.detail.as_deref(), Some("AAAA-1111"));
    }

    #[test]
    fn milestones_never_demote_connected() {
        let mut s = ProjState::default();
        fold_line(&mut s, ProjectType::Flutter, "Flutter run key commands.");
        assert_eq!(s.phase, Some(MobilePhase::Connected));
        // Hot-reload triggers a fresh "Building ..." style line; stay connected.
        fold_line(&mut s, ProjectType::Flutter, "Building flutter tool...");
        assert_eq!(s.phase, Some(MobilePhase::Connected));
    }

    #[test]
    fn crash_during_build_becomes_build_failed_with_step() {
        let mut tracker = HashMap::new();
        let dir = std::env::temp_dir().join(format!("pb-phase-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let id = "demo";
        std::fs::write(dir.join(format!("{id}.log")), "::portbay::phase=building\n").unwrap();
        let ev = fold(
            &mut tracker,
            &dir,
            id,
            ProjectType::Xcode,
            ProjectStatus::Running,
        );
        assert_eq!(ev, Some((Some(MobilePhase::Building), None)));
        let ev = fold(
            &mut tracker,
            &dir,
            id,
            ProjectType::Xcode,
            ProjectStatus::Crashed,
        );
        assert_eq!(
            ev,
            Some((
                Some(MobilePhase::BuildFailed),
                Some("failed while building".to_string())
            ))
        );
        // The verdict survives the subsequent Stopped observation.
        let ev = fold(
            &mut tracker,
            &dir,
            id,
            ProjectType::Xcode,
            ProjectStatus::Stopped,
        );
        assert_eq!(ev, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn locked_device_signatures_are_recognized() {
        // devicectl install/launch variants across Xcode releases.
        for line in [
            "Error Domain=com.apple.dt.CoreDeviceError Code=3001 \"The device is locked.\"",
            "ERROR: The device is passcode protected.",
            "Unable to launch com.x because the device was not, or could not be, unlocked.",
            "Your device is locked. Please unlock and try again.",
        ] {
            assert!(parse_hint(line).is_some(), "missed: {line}");
        }
        assert_eq!(parse_hint("Compiling Runner..."), None);
        assert_eq!(parse_hint("file locked by another process"), None);
    }

    #[test]
    fn crash_after_locked_device_error_carries_unlock_hint() {
        let mut tracker = HashMap::new();
        let dir = std::env::temp_dir().join(format!("pb-phase-lock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let id = "locked";
        std::fs::write(
            dir.join(format!("{id}.log")),
            "::portbay::phase=installing\nERROR: The device is passcode protected.\n",
        )
        .unwrap();
        fold(
            &mut tracker,
            &dir,
            id,
            ProjectType::Xcode,
            ProjectStatus::Running,
        );
        let ev = fold(
            &mut tracker,
            &dir,
            id,
            ProjectType::Xcode,
            ProjectStatus::Crashed,
        );
        assert_eq!(
            ev,
            Some((
                Some(MobilePhase::BuildFailed),
                Some(
                    "failed while installing: device locked — unlock the phone, \
                     then press Play again"
                        .to_string()
                )
            ))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn truncation_resets_to_a_fresh_run() {
        let mut tracker = HashMap::new();
        let dir = std::env::temp_dir().join(format!("pb-phase-trunc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let id = "demo2";
        let log = dir.join(format!("{id}.log"));
        std::fs::write(
            &log,
            "::portbay::phase=building\n::portbay::phase=installing\n",
        )
        .unwrap();
        fold(
            &mut tracker,
            &dir,
            id,
            ProjectType::Xcode,
            ProjectStatus::Running,
        );
        assert_eq!(
            tracker.get(id).unwrap().phase,
            Some(MobilePhase::Installing)
        );
        // Restart: PC truncates and the new run writes fresh markers.
        std::fs::write(&log, "::portbay::phase=resolving-device\n").unwrap();
        fold(
            &mut tracker,
            &dir,
            id,
            ProjectType::Xcode,
            ProjectStatus::Running,
        );
        assert_eq!(
            tracker.get(id).unwrap().phase,
            Some(MobilePhase::ResolvingDevice)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
