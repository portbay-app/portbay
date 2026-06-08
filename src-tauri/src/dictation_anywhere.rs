//! "Dictate anywhere" — system-wide push-to-talk on the local STT engine.
//!
//! Hold Fn (🌐) in **any** app and the `portbay-stt` sidecar captures the
//! mic, the notch overlay (`crate::overlay_window`) shows the live session,
//! and on release the final transcript is pasted into the app that was
//! frontmost at Fn-down (`crate::typing`). Opt-in
//! (`preferences.dictation.anywhere`), local-engine only (`stt_engine ==
//! "local"` with a model chosen — for the macOS engine the OS already
//! offers its own system-wide shortcut), and gated on Accessibility trust.
//!
//! ## How the trigger works
//!
//! The in-app push-to-talk (`dictation_session::init_fn_monitor`) is a
//! *local* NSEvent monitor — it only sees events while PortBay is the
//! active app. This module installs the *global* twin
//! (`addGlobalMonitorForEventsMatchingMask`), which fires **only while
//! some other app is active** — so the two triggers partition cleanly and
//! can never double-fire. Global monitors are observe-only (events are
//! never consumed — the Fn keypress still does whatever the user's
//! "Press 🌐 to" setting says) and require Accessibility trust to receive
//! anything, the same grant the paste-injection needs anyway.
//!
//! ## Session state machine
//!
//! ```text
//!        Fn down                  300 ms held              mic hot
//! Idle ──────────▶ Pending ─────────────────▶ Arming ───────────────▶ Capturing
//!  ▲                  │ Fn up / other key       │ Fn up                 │ Fn up
//!  │                  ▼ (tap — note the time)   ▼ (abort when start     ▼
//!  │               Idle                      AbortAfterArm  lands)   Finishing
//!  │                                            │                       │ stop →
//!  └────────────────────────────────────────────┴───────────────────────┘ insert →
//!                                                                         hide
//! ```
//!
//! Two trigger modes share the machine:
//!
//! - **Hold** (the diagram above): Fn held past the 300 ms gate, released
//!   to finish — push-to-talk.
//! - **Toggle** (hands-free): a quick tap followed by a second press within
//!   400 ms jumps Idle → Arming directly; the session then ignores Fn
//!   releases, and the NEXT Fn press (or Esc) finishes it. Long dictations
//!   don't require pinning a finger on the key. The double-tap mirrors
//!   macOS dictation's own "press 🌐 twice" idiom;
//!   `prefs.dictation.anywhere_double_tap` turns it off for users whose Fn
//!   key already double-taps into something else.
//!
//! The 300 ms hold gate mirrors the in-app push-to-talk's disambiguator
//! (`src/lib/dictation/pushToTalk.ts`): a quick Fn tap is the emoji
//! picker / input-source switch, never a dictation request — which is also
//! why toggle requires a double tap rather than claiming the single tap.
//! Esc cancels a live session and discards the words (observe-only monitor
//! — the Esc also reaches the focused app, same trade FluidVoice ships).
//!
//! Audio stays in the sidecar; the transcript goes to the target app and
//! nowhere else — plus a short local history ring
//! (`crate::dictation_history`) so a paste the target app ate is
//! recoverable instead of destroyed. The rewrite layer is deliberately NOT
//! in this loop yet — it is tuned for fields PortBay owns (caret context,
//! surface vocabulary); raw transcripts match what macOS dictation would
//! type.

#[cfg(target_os = "macos")]
use std::sync::Mutex;

#[cfg(target_os = "macos")]
use once_cell::sync::Lazy;
use serde::Serialize;

#[cfg(target_os = "macos")]
use crate::typing::FrontTarget;

/// Where a system-wide session currently is. See the module diagram.
#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    /// Fn is down, the 300 ms hold gate hasn't elapsed.
    Pending,
    /// Hold confirmed; `stt::start_capture` is in flight (model load).
    Arming,
    /// Fn released while Arming — cancel as soon as the start resolves.
    AbortAfterArm,
    /// Mic hot, words landing.
    Capturing,
    /// Fn released; stop → transcribe → insert in flight.
    Finishing,
}

/// How the running session is driven. Hold finishes on Fn release; Toggle
/// (started by a double tap) ignores releases and finishes on the next Fn
/// press.
#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Hold,
    Toggle,
}

#[cfg(target_os = "macos")]
struct Session {
    phase: Phase,
    mode: Mode,
    /// Invalidates in-flight async work when a newer transition happened.
    generation: u64,
    /// The app the transcript will be pasted into (frontmost at Fn-down).
    target: Option<FrontTarget>,
    /// When the last clean Fn tap (Pending released inside the hold gate,
    /// no chord) ended — the first half of a toggle double-tap.
    last_tap: Option<std::time::Instant>,
    /// The resolved Context-Store term snapshot taken at session arm — the
    /// SAME set the recognizer was biased with. Reused at finish for the
    /// always-on term correction (Polish-off safety net) and as the rewrite
    /// vocabulary, so recognizer and rewrite never drift within one session.
    terms: Vec<String>,
}

#[cfg(target_os = "macos")]
static SESSION: Lazy<Mutex<Session>> = Lazy::new(|| {
    Mutex::new(Session {
        phase: Phase::Idle,
        mode: Mode::Hold,
        generation: 0,
        target: None,
        last_tap: None,
        terms: Vec::new(),
    })
});

/// Which dictation surface invoked the overlay — wired through every
/// transition so the leading slot can grow per-mode animated icons (the
/// palette stays white either way; mode never changes colors).
#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum OverlayMode {
    /// Plain dictation — the words become content. Dictate-anywhere
    /// sessions are always this (the rewrite layer isn't in that loop).
    Dictation,
    /// Voice Edit Mode — text was selected at session start, so the words
    /// are an instruction about it (see `DictationRewriter.begin`).
    Edit,
    /// A rewrite/transform pass (Writing Tools-style surfaces). No surface
    /// drives the overlay with this yet; the value is wired so one can.
    Rewrite,
}

#[cfg(target_os = "macos")]
impl OverlayMode {
    /// Frontend wire value → mode; unknown strings read as plain dictation.
    fn from_wire(value: &str) -> Self {
        match value {
            "edit" => Self::Edit,
            "rewrite" => Self::Rewrite,
            _ => Self::Dictation,
        }
    }
}

/// Overlay knobs from `preferences.dictation`, sent with the arming
/// transition like the notch geometry (the overlay keeps them for the
/// session; later transitions carry `None`).
#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
struct OverlaySettings {
    /// Raw mic-RMS floor below which the waveform stays flat.
    noise_floor: f64,
    /// Preview keeps the last N chars of the partial (head-truncated).
    preview_chars: u32,
}

/// What the overlay webview renders, pushed on every transition. The
/// stream of `stt://partial` / `stt://level` events fills in the live text
/// and waveform between transitions.
#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OverlayState {
    /// "arming" | "live" | "processing" | "done" | "error" | "hidden"
    phase: &'static str,
    app_name: Option<String>,
    app_icon: Option<String>,
    notch: Option<crate::overlay_window::NotchGeometry>,
    error: Option<String>,
    /// Hands-free session — the overlay hints "tap 🌐 to stop" since no
    /// held key is telling the user how to end it.
    toggle: bool,
    /// Which surface invoked the session (leading-slot seam).
    mode: OverlayMode,
    /// Overlay knobs, carried on arming transitions only.
    settings: Option<OverlaySettings>,
}

#[cfg(target_os = "macos")]
impl OverlayState {
    /// A transition with nothing but the phase set — the fields every
    /// construction site shares; sites with more context use struct-update
    /// syntax over this.
    fn bare(phase: &'static str) -> Self {
        Self {
            phase,
            app_name: None,
            app_icon: None,
            notch: None,
            error: None,
            toggle: false,
            mode: OverlayMode::Dictation,
            settings: None,
        }
    }
}

/// Status for the settings UI (and the `dictation_anywhere_arm` command).
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnywhereStatus {
    /// macOS only.
    pub supported: bool,
    /// Accessibility (AXIsProcessTrusted) — required for both the global
    /// key monitor and the paste injection.
    pub trusted: bool,
    /// Whether the global monitors are installed this run.
    pub monitoring: bool,
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ptr::NonNull;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use block2::RcBlock;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSEvent, NSEventMask, NSEventModifierFlags, NSEventType};
    use tauri::{AppHandle, Emitter, Manager};

    use std::sync::Mutex;

    use once_cell::sync::Lazy;

    use super::{AnywhereStatus, Mode, OverlayMode, OverlaySettings, OverlayState, Phase, SESSION};
    use crate::dictation::{InputSource, ProviderConfig, RewriteContext, RewriteMode};
    use crate::overlay_window;
    use crate::preferences::AppContextRule;
    use crate::state::AppState;
    use crate::typing::FrontTarget;

    /// Same hold gate as the in-app push-to-talk disambiguator.
    const HOLD_GATE: Duration = Duration::from_millis(300);
    /// How long the polish rewrite gets before the paste falls back to the raw
    /// transcript. Bounded so a slow/cold model never holds the paste open
    /// indefinitely; the words always land (raw) within this window even when
    /// the rewrite stalls. `begin_session` prewarms the model so the common
    /// case finishes well inside it.
    const POLISH_TIMEOUT: Duration = Duration::from_secs(25);

    /// Terminal-emulator bundle ids that default to `TerminalCommand`
    /// formatting (operator words → symbols, no prose paragraphs). Everything
    /// else defaults to `GeneralNote`. Users extend/override via
    /// `dictation.anywhere_app_contexts`.
    const TERMINAL_BUNDLES: &[&str] = &[
        "com.apple.Terminal",
        "com.googlecode.iterm2",
        "dev.warp.Warp-Stable",
        "net.kovidgoyal.kitty",
        "io.alacritty",
        "org.alacritty",
        "co.zeit.hyper",
        "com.github.wez.wezterm",
    ];

    /// Resolve the frontmost app to a rewrite context for the polished
    /// anywhere path (Gap 4, minimal). Priority: an explicit user override for
    /// this bundle id → the built-in terminal default → `GeneralNote` (which
    /// fixes invention and, on clean input, adds paragraph layout). Pure for
    /// unit testing.
    pub(super) fn resolve_context(
        bundle_id: Option<&str>,
        rules: &[AppContextRule],
    ) -> RewriteContext {
        if let Some(id) = bundle_id {
            if let Some(rule) = rules.iter().find(|r| r.bundle_id.eq_ignore_ascii_case(id)) {
                if let Some(ctx) = RewriteContext::from_wire(&rule.context) {
                    return ctx;
                }
            }
            if TERMINAL_BUNDLES.iter().any(|t| t.eq_ignore_ascii_case(id)) {
                return RewriteContext::TerminalCommand;
            }
        }
        RewriteContext::GeneralNote
    }

    /// The Gap-1 polish stage: run the shared rewrite engine over the final
    /// transcript before it's pasted, when "Polish dictation everywhere" is
    /// on. Returns the text to paste. EVERY non-success path returns the input
    /// unchanged: polish off, no model, a guard rejection, or a timeout all
    /// degrade to the words the user spoke (zero data loss). The rewrite routes
    /// through the same engine/guards/sanitizer as the in-app surfaces — no
    /// second rewrite path. (The caller compares the result to the ORIGINAL
    /// transcript to decide what the history ring keeps as recoverable.)
    async fn maybe_polish(
        app: &AppHandle,
        text: &str,
        target: Option<&FrontTarget>,
        generation: u64,
        // The armed Context-Store snapshot — fed to the rewrite as its
        // vocabulary so the polish corrects spellings from the SAME term set the
        // recognizer was biased with (no drift between the two seams).
        terms: &[String],
    ) -> String {
        let prefs = app.state::<AppState>().preferences_snapshot().dictation;
        if !prefs.anywhere_polish {
            return text.to_string();
        }
        let provider = ProviderConfig {
            kind: prefs.provider.clone(),
            endpoint: prefs.endpoint.clone(),
            model: prefs.model.clone(),
        };
        let context = resolve_context(
            target.and_then(|t| t.bundle_id.as_deref()),
            &prefs.anywhere_app_contexts,
        );
        // Anywhere capture is local-engine only (the trigger gate requires
        // it), so the transcript is already-punctuated Whisper/Parakeet output
        // → Clean, which turns on the paragraph-layout addendum.
        let request_id = format!("anywhere-{generation}");
        // Gap 2: show the dedicated "Polishing…" notch and stream the rewrite
        // into its preview as it forms. The streamed text is display-only —
        // the validated/sanitized result is what gets pasted, atomically,
        // below. Reuses the `stt://partial` channel the overlay already
        // renders (capture has stopped, so no real partials race it).
        emit_state(
            app,
            OverlayState {
                app_name: target.map(|t| t.name.clone()),
                app_icon: target.and_then(|t| t.icon_data_url.clone()),
                ..OverlayState::bare("polishing")
            },
        );
        let sink_app = app.clone();
        let sink = move |acc: &str| {
            let _ = sink_app.emit("stt://partial", serde_json::json!({ "text": acc }));
        };
        let state = app.state::<AppState>();
        let extra = (!terms.is_empty()).then(|| terms.to_vec());
        let rewrite = crate::commands::dictation::run_rewrite(
            state.inner(),
            &request_id,
            text,
            context,
            extra,
            None,
            RewriteMode::Smart,
            &provider,
            InputSource::Clean,
            Some(&sink),
        );
        let outcome = match tokio::time::timeout(POLISH_TIMEOUT, rewrite).await {
            Ok(outcome) => outcome,
            Err(_) => {
                tracing::warn!("dictation: anywhere polish timed out; pasting raw");
                return text.to_string();
            }
        };
        match (outcome.status, outcome.text) {
            ("rewritten", Some(polished)) => polished,
            (status, _) => {
                if status != "rewritten" {
                    tracing::debug!(status, "dictation: anywhere polish kept raw");
                }
                text.to_string()
            }
        }
    }
    /// How long after a clean Fn tap a second press still reads as the
    /// double-tap that starts a hands-free session (FluidVoice's tap
    /// threshold is 0.4 s; macOS's own double-press window feels the same).
    const DOUBLE_TAP_WINDOW: Duration = Duration::from_millis(400);
    /// kVK_Escape.
    const KEY_ESCAPE: u16 = 53;

    static MONITORS_INSTALLED: AtomicBool = AtomicBool::new(false);

    /// Install the global monitors at app start when trust already exists.
    /// When it doesn't, `ensure_monitors` re-attempts after the user grants
    /// (the settings toggle and the status command both call it).
    pub fn init(app: &AppHandle, mtm: MainThreadMarker) {
        if crate::typing::ax_trusted() {
            install_monitors(app, mtm);
        } else {
            tracing::info!(
                "dictation: anywhere monitors deferred — accessibility not granted yet"
            );
        }
    }

    /// Idempotent monitor installation. The `MainThreadMarker` makes the
    /// "main thread only" contract a compile-time requirement (it can only be
    /// obtained on the main thread) rather than a comment a future caller can
    /// miss. Returns the resulting status either way.
    pub fn ensure_monitors(app: &AppHandle, mtm: MainThreadMarker) -> AnywhereStatus {
        if !MONITORS_INSTALLED.load(Ordering::SeqCst) && crate::typing::ax_trusted() {
            install_monitors(app, mtm);
        }
        status()
    }

    pub fn status() -> AnywhereStatus {
        AnywhereStatus {
            supported: true,
            trusted: crate::typing::ax_trusted(),
            monitoring: MONITORS_INSTALLED.load(Ordering::SeqCst),
        }
    }

    /// The global flagsChanged + keyDown monitor. Observe-only by design:
    /// NSEvent global monitors cannot consume events, which is exactly the
    /// posture we want for a passive push-to-talk trigger.
    fn install_monitors(app: &AppHandle, _mtm: MainThreadMarker) {
        if MONITORS_INSTALLED.swap(true, Ordering::SeqCst) {
            return;
        }
        static FN_DOWN: AtomicBool = AtomicBool::new(false);

        let handle = app.clone();
        let block = RcBlock::new(move |event: NonNull<NSEvent>| {
            // SAFETY: the monitor hands us a valid NSEvent for the
            // callback's duration; we only read type/flags/keyCode.
            let event = unsafe { event.as_ref() };
            match event.r#type() {
                NSEventType::FlagsChanged => {
                    let down = event.modifierFlags()
                        .contains(NSEventModifierFlags::Function);
                    if FN_DOWN.swap(down, Ordering::SeqCst) != down {
                        on_fn(&handle, down);
                    }
                }
                NSEventType::KeyDown => {
                    on_key_down(&handle, event.keyCode());
                }
                _ => {}
            }
        });
        // The block is 'static (captures only AppHandle). The monitor
        // token is leaked — it lives for the app, like the local monitor.
        let token = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(
            NSEventMask::FlagsChanged | NSEventMask::KeyDown,
            &block,
        );
        match token {
            Some(token) => {
                std::mem::forget(token);
                std::mem::forget(block);
                tracing::info!("dictation: anywhere global monitors installed");
                // Warm the STT model now (once per run, this branch runs once)
                // so the user's first Fn-hold isn't a cold multi-GB load.
                maybe_prewarm_stt(app);
            }
            None => {
                MONITORS_INSTALLED.store(false, Ordering::SeqCst);
                tracing::warn!("dictation: anywhere global monitor failed to install");
            }
        }
    }

    /// Background-warm the local STT model when dictate-anywhere is configured,
    /// so the first Fn-hold pays the warm path (sub-second) instead of a cold
    /// load (3-4 s on first use after boot). Fire-and-forget: the load happens
    /// in a throwaway sidecar process, so the lasting benefit is the OS-level
    /// caches that persist process-to-process (file cache, CoreML
    /// specialization, ANE) — keeping the model *resident* between captures is
    /// a separate, larger change. No-op unless the feature is enabled on the
    /// local engine with a model chosen; the sidecar's own PREWARMING guard
    /// makes a concurrent in-app prewarm a no-op.
    fn maybe_prewarm_stt(app: &AppHandle) {
        let prefs = app.state::<AppState>().preferences_snapshot();
        let d = &prefs.dictation;
        if !anywhere_trigger_allowed(d.anywhere, &d.stt_engine, &d.stt_model) {
            return;
        }
        let model = d.stt_model.clone();
        let models_dir = crate::ollama::expand_tilde(&prefs.stt.models_dir);
        tauri::async_runtime::spawn(async move {
            tracing::info!(model = %model, "dictation: prewarming STT model (anywhere enabled)");
            crate::stt::prewarm(&models_dir, &model).await;
        });
    }

    /// Fn transition while another app is active (global monitors never
    /// fire for our own app). Main thread.
    fn on_fn(app: &AppHandle, down: bool) {
        if down {
            fn_pressed(app);
        } else {
            fn_released(app);
        }
    }

    /// Bridge from the in-app (local) Fn monitor: a release seen while
    /// PortBay is frontmost must still end a dictate-anywhere session the
    /// user started elsewhere and cmd-tabbed away from mid-hold. Presses
    /// stay in-app territory (push-to-talk owns those) — except when a
    /// hands-free session is live: the user carried it back into PortBay,
    /// and this press is its stop signal.
    pub fn on_local_fn(app: &AppHandle, down: bool) {
        if down {
            let finish = {
                let mut s = SESSION.lock().unwrap_or_else(|e| e.into_inner());
                if s.phase == Phase::Capturing && s.mode == Mode::Toggle {
                    s.phase = Phase::Finishing;
                    Some((s.generation, s.target.clone()))
                } else {
                    None
                }
            };
            if let Some((generation, target)) = finish {
                spawn_finish(app, generation, target);
            }
        } else {
            fn_released(app);
        }
    }

    /// What an Fn press asked for, decided under the session lock.
    enum PressAction {
        None,
        /// Wait out the hold gate, then arm (hold mode).
        BeginHold(u64),
        /// Second tap of a double-tap: arm immediately (toggle mode).
        BeginToggle(u64),
        /// Press during a hands-free capture: this is the stop signal.
        Finish(u64, Option<FrontTarget>),
    }

    /// The press-time gate for dictate-anywhere: the feature must be enabled,
    /// the local STT engine selected, and a model chosen. The global Fn
    /// trigger only drives the local sidecar — there is no macOS-dictation
    /// path here — so an unconfigured feature must no-op rather than arm.
    /// Pure so the "is the feature wired on?" routing is unit-testable apart
    /// from the AppHandle/preferences plumbing.
    pub(super) fn anywhere_trigger_allowed(
        anywhere: bool,
        stt_engine: &str,
        stt_model: &str,
    ) -> bool {
        anywhere && stt_engine == "local" && !stt_model.is_empty()
    }

    /// The phase/mode change an Fn press produces, decided purely from the
    /// current state plus whether this press is the second half of a
    /// double-tap. Split from `fn_pressed` so the whole transition table is
    /// testable without a live session, AppHandle, or the main thread.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) enum PressKind {
        None,
        BeginHold,
        BeginToggle,
        Finish,
        AbortArm,
    }

    pub(super) fn decide_press(
        phase: Phase,
        mode: Mode,
        double_tap: bool,
    ) -> (Phase, Mode, PressKind) {
        match (phase, mode) {
            // Second tap — hands-free session, no hold gate (the first tap
            // already disambiguated the gesture).
            (Phase::Idle, _) if double_tap => {
                (Phase::Arming, Mode::Toggle, PressKind::BeginToggle)
            }
            (Phase::Idle, _) => (Phase::Pending, Mode::Hold, PressKind::BeginHold),
            // Hands-free capture: a fresh Fn press is the stop signal.
            (Phase::Capturing, Mode::Toggle) => (Phase::Finishing, mode, PressKind::Finish),
            // Impatient re-tap while the model still loads: "never mind" —
            // nothing has been recorded yet worth keeping.
            (Phase::Arming, Mode::Toggle) => (Phase::AbortAfterArm, mode, PressKind::AbortArm),
            _ => (phase, mode, PressKind::None),
        }
    }

    fn fn_pressed(app: &AppHandle) {
        // Cheap pref gate before any state changes: feature on, local
        // engine, model chosen, sidecar capture conceivable.
        let prefs = app.state::<AppState>().preferences_snapshot().dictation;
        if !anywhere_trigger_allowed(prefs.anywhere, &prefs.stt_engine, &prefs.stt_model) {
            return;
        }
        let model = prefs.stt_model;

        let action = {
            let mut s = SESSION.lock().unwrap_or_else(|e| e.into_inner());
            let double_tap = prefs.anywhere_double_tap
                && s.last_tap.is_some_and(|t| t.elapsed() <= DOUBLE_TAP_WINDOW);
            let (new_phase, new_mode, kind) = decide_press(s.phase, s.mode, double_tap);
            match kind {
                PressKind::BeginHold | PressKind::BeginToggle => {
                    s.last_tap = None;
                    s.generation += 1;
                    // Capture the paste target NOW — the frontmost app at the
                    // moment the user started talking, not wherever they
                    // ended up.
                    let mtm = MainThreadMarker::new()
                        .expect("NSEvent monitor runs on the main thread");
                    s.target = crate::typing::capture_front_target(mtm);
                    s.phase = new_phase;
                    s.mode = new_mode;
                    if matches!(kind, PressKind::BeginToggle) {
                        PressAction::BeginToggle(s.generation)
                    } else {
                        PressAction::BeginHold(s.generation)
                    }
                }
                PressKind::Finish => {
                    // Clear any stale tap so the press that ends this session
                    // can't later read as the first half of a new double-tap.
                    s.last_tap = None;
                    s.phase = new_phase;
                    PressAction::Finish(s.generation, s.target.clone())
                }
                PressKind::AbortArm => {
                    s.last_tap = None;
                    s.phase = new_phase;
                    PressAction::None
                }
                PressKind::None => PressAction::None,
            }
        };

        match action {
            PressAction::BeginHold(generation) => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(HOLD_GATE).await;
                    {
                        let mut s = SESSION.lock().unwrap_or_else(|e| e.into_inner());
                        if s.generation != generation || s.phase != Phase::Pending {
                            return;
                        }
                        s.phase = Phase::Arming;
                    }
                    begin_session(app, generation, model).await;
                });
            }
            PressAction::BeginToggle(generation) => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    begin_session(app, generation, model).await;
                });
            }
            PressAction::Finish(generation, target) => {
                spawn_finish(app, generation, target);
            }
            PressAction::None => {}
        }
    }

    /// Hold confirmed: show the overlay (arming look) and start the sidecar
    /// capture. The await spans the model cold-load; the overlay's arming
    /// state is honest about it, exactly like the in-app `arming` phase.
    async fn begin_session(app: AppHandle, generation: u64, model: String) {
        let (target, toggle) = {
            let s = SESSION.lock().unwrap_or_else(|e| e.into_inner());
            (s.target.clone(), s.mode == Mode::Toggle)
        };
        // Page the rewrite model in now (best-effort, fire-and-forget) when
        // polish is on, so the rewrite at session end doesn't pay the cold
        // load inside its own timeout — same prewarm-when-anticipated pattern
        // the in-app path uses.
        {
            let d = app.state::<AppState>().preferences_snapshot().dictation;
            if d.anywhere_polish {
                crate::dictation::prewarm(&ProviderConfig {
                    kind: d.provider,
                    endpoint: d.endpoint,
                    model: d.model,
                });
            }
        }
        let notch = show_overlay(&app).await;
        emit_state(
            &app,
            OverlayState {
                app_name: target.as_ref().map(|t| t.name.clone()),
                app_icon: target.as_ref().and_then(|t| t.icon_data_url.clone()),
                notch,
                toggle,
                settings: Some(overlay_settings(&app)),
                ..OverlayState::bare("arming")
            },
        );

        let models_dir = crate::ollama::expand_tilde(
            &app.state::<AppState>().preferences_snapshot().stt.models_dir,
        );

        // Resolve the Context-Store term snapshot for this session (global set
        // — anywhere has no project surface, §10.3). The rewrite context is
        // inferred from the target app so the learned-jargon ranking matches
        // what the finish-time rewrite will use. Stored on the session so the
        // correction net + rewrite reuse the exact same snapshot; the recognizer
        // bias is the engine-gated slice of it (never a prompt Parakeet can't
        // apply).
        let context = {
            let prefs = app.state::<AppState>().preferences_snapshot().dictation;
            resolve_context(
                target.as_ref().and_then(|t| t.bundle_id.as_deref()),
                &prefs.anywhere_app_contexts,
            )
        };
        let snapshot = crate::commands::dictation::recognizer_terms(
            app.state::<AppState>().inner(),
            Some(context),
            None,
        )
        .await;
        let bias = if crate::dictation_context::recognizer_bias_enabled()
            && crate::dictation_context::engine_supports_text_bias(&model)
        {
            crate::dictation_context::instrument::record_bias(snapshot.len());
            tracing::debug!(model = %model, terms = snapshot.len(), "dictation: anywhere recognizer bias resolved");
            snapshot.clone()
        } else {
            // Default: no recognizer bias (gated/unsupported — see
            // `recognizer_bias_enabled`). The always-on correction net below
            // and the rewrite still fix known terms from `snapshot`.
            Vec::new()
        };
        // Park the full snapshot for the finish path (correction + rewrite).
        SESSION.lock().unwrap_or_else(|e| e.into_inner()).terms = snapshot;

        let started = crate::stt::start_capture(app.clone(), &models_dir, &model, &bias).await;

        let next: Phase = {
            let mut s = SESSION.lock().unwrap_or_else(|e| e.into_inner());
            if s.generation != generation {
                // Superseded (defensive — Arming blocks new sessions, so
                // this shouldn't occur). A capture that landed anyway has
                // no owner: discard it; the new owner manages the overlay.
                if started.is_ok() {
                    Phase::AbortAfterArm
                } else {
                    Phase::Idle
                }
            } else {
                match (&started, s.phase) {
                    (Ok(()), Phase::Arming) => {
                        s.phase = Phase::Capturing;
                        Phase::Capturing
                    }
                    // Released mid-load, or Esc'd: discard.
                    (Ok(()), _) => Phase::AbortAfterArm,
                    (Err(_), _) => {
                        s.phase = Phase::Idle;
                        Phase::Idle
                    }
                }
            }
        };

        match next {
            Phase::Capturing => {
                emit_state(
                    &app,
                    OverlayState {
                        app_name: target.as_ref().map(|t| t.name.clone()),
                        app_icon: target.as_ref().and_then(|t| t.icon_data_url.clone()),
                        toggle,
                        ..OverlayState::bare("live")
                    },
                );
                play_cue("Tink");
            }
            Phase::AbortAfterArm => {
                crate::stt::cancel_capture().await;
                finish_hidden(&app, generation).await;
            }
            _ => {
                if let Err(detail) = started {
                    tracing::warn!(detail = %detail, "dictation: anywhere capture failed to start");
                    emit_state(
                        &app,
                        OverlayState {
                            error: Some(detail),
                            ..OverlayState::bare("error")
                        },
                    );
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(Duration::from_millis(2200)).await;
                        finish_hidden(&app, generation).await;
                    });
                }
            }
        }
    }

    fn fn_released(app: &AppHandle) {
        let finish = {
            let mut s = SESSION.lock().unwrap_or_else(|e| e.into_inner());
            match (s.phase, s.mode) {
                // Quick tap — the emoji picker / input-source switch, not
                // us; but note the time, it may be half a toggle double-tap.
                (Phase::Pending, _) => {
                    s.phase = Phase::Idle;
                    s.last_tap = Some(std::time::Instant::now());
                    None
                }
                (Phase::Arming, Mode::Hold) => {
                    s.phase = Phase::AbortAfterArm;
                    None
                }
                (Phase::Capturing, Mode::Hold) => {
                    s.phase = Phase::Finishing;
                    Some((s.generation, s.target.clone()))
                }
                // Hands-free sessions ignore releases — the starting
                // double-tap's own release lands here; only the next Fn
                // PRESS (or Esc) stops the capture.
                _ => None,
            }
        };
        if let Some((generation, target)) = finish {
            spawn_finish(app, generation, target);
        }
    }

    /// Stop → transcribe → deliver → hide: the tail of every successful
    /// session, whether the stop came from an Fn release (hold), an Fn
    /// press (toggle), or the in-app bridge. Caller has already moved the
    /// machine to Finishing.
    fn spawn_finish(app: &AppHandle, generation: u64, target: Option<FrontTarget>) {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            emit_state(
                &app,
                OverlayState {
                    app_name: target.as_ref().map(|t| t.name.clone()),
                    app_icon: target.as_ref().and_then(|t| t.icon_data_url.clone()),
                    ..OverlayState::bare("processing")
                },
            );

            let transcript = match crate::stt::stop_capture().await {
                Ok(t) => t,
                Err(detail) => {
                    tracing::warn!(detail = %detail, "dictation: anywhere final pass failed");
                    emit_state(
                        &app,
                        OverlayState {
                            error: Some(detail),
                            ..OverlayState::bare("error")
                        },
                    );
                    tokio::time::sleep(Duration::from_millis(2200)).await;
                    finish_hidden(&app, generation).await;
                    return;
                }
            };

            let trimmed = transcript.trim();
            if !trimmed.is_empty() {
                // Gap 3: apply inline voice commands ("new line", "bullet",
                // "scratch that") as a deterministic pre-pass — a no-op unless
                // a command stood alone as its own clause. Runs BEFORE polish
                // so the structural markers compose with the rewrite's layout
                // rules (and are literal on the raw-paste path).
                let commanded = crate::dictation_commands::apply_voice_commands(trimmed);
                // The armed Context-Store snapshot (recognizer + rewrite share
                // it). Still set this session — the state machine can't start a
                // new one until finish_hidden returns to Idle.
                let terms = SESSION.lock().unwrap_or_else(|e| e.into_inner()).terms.clone();
                // Foundation: always-on term correction — a deterministic,
                // exact-squash spelling fix for known terms even when Polish is
                // OFF (the recognizer may have missed one, or the engine
                // couldn't take a bias prompt at all — Parakeet). Safe by
                // construction; see `dictation_context::correct_terms`.
                let correction = crate::dictation_context::correct_terms(&commanded, &terms);
                crate::dictation_context::instrument::record_correction(
                    correction.applied,
                    correction.already_correct,
                );
                if correction.applied > 0 {
                    tracing::debug!(
                        applied = correction.applied,
                        "dictation: anywhere term correction applied (polish-off safety net)"
                    );
                }
                let corrected = correction.text;
                // Gap 1: polish the (command-processed, term-corrected)
                // transcript through the shared rewrite engine before pasting
                // (when enabled), sourcing its vocab from the same snapshot.
                // Degrades to those words on any failure/timeout — see
                // `maybe_polish`.
                let delivered =
                    maybe_polish(&app, &corrected, target.as_ref(), generation, &terms).await;
                // History keeps the ORIGINAL transcript recoverable whenever
                // commands, correction, or polish changed what was delivered.
                let raw_original = (delivered != trimmed).then(|| trimmed.to_string());
                let mut inserted = false;
                let mut rescue: Option<String> = None;
                if let Some(target) = &target {
                    match crate::typing::insert_text(&app, delivered.clone(), target.pid).await {
                        Ok(()) => {
                            inserted = true;
                            play_cue("Pop");
                        }
                        Err(detail) => {
                            tracing::warn!(detail = %detail, "dictation: anywhere insertion failed");
                            // The words must survive the failure: leave them
                            // on the clipboard — persistently, no restore —
                            // and tell the user where they went.
                            rescue = Some(
                                if crate::typing::copy_text_persistent(&app, delivered.clone())
                                    .await
                                    .is_ok()
                                {
                                    "Couldn’t paste — copied instead. Press ⌘V.".to_string()
                                } else {
                                    "Couldn’t paste the transcript.".to_string()
                                },
                            );
                        }
                    }
                }
                // Second safety net: the history ring (tray "Paste Last
                // Dictation" + the settings panel's recent list). Stores the
                // delivered text plus the pre-polish transcript when a rewrite
                // changed it, so an over-eager polish stays recoverable.
                crate::dictation_history::record(
                    &delivered,
                    raw_original,
                    target.as_ref().map(|t| t.name.clone()),
                    inserted,
                );
                crate::tray::refresh_dictation_item(&app);

                if let Some(message) = rescue {
                    emit_state(
                        &app,
                        OverlayState {
                            app_name: target.as_ref().map(|t| t.name.clone()),
                            app_icon: target.as_ref().and_then(|t| t.icon_data_url.clone()),
                            error: Some(message),
                            ..OverlayState::bare("error")
                        },
                    );
                    tokio::time::sleep(Duration::from_millis(2200)).await;
                    finish_hidden(&app, generation).await;
                    return;
                }
            }
            emit_state(&app, OverlayState::bare("done"));
            // Let the overlay play its exit animation before the window
            // disappears under it.
            tokio::time::sleep(Duration::from_millis(450)).await;
            finish_hidden(&app, generation).await;
        });
    }

    /// Esc cancels a session in flight; any other key during the hold gate
    /// is an Fn-chord (Fn+arrow etc.) and aborts the pending trigger — the
    /// same guard the in-app disambiguator applies. A toggle session gets
    /// the equivalent chord guard while Arming (its tap-tap prefix can be
    /// the start of "tap Fn, then Fn+arrow"); once the mic is hot, keys are
    /// the user typing alongside their hands-free dictation and pass.
    fn on_key_down(app: &AppHandle, key_code: u16) {
        let cancel_generation = {
            let mut s = SESSION.lock().unwrap_or_else(|e| e.into_inner());
            match (s.phase, key_code) {
                (Phase::Pending, _) => {
                    s.phase = Phase::Idle;
                    None
                }
                (Phase::Arming, KEY_ESCAPE) => {
                    s.phase = Phase::AbortAfterArm;
                    None
                }
                (Phase::Arming, _) if s.mode == Mode::Toggle => {
                    s.phase = Phase::AbortAfterArm;
                    None
                }
                (Phase::Capturing, KEY_ESCAPE) => {
                    s.phase = Phase::Finishing;
                    Some(s.generation)
                }
                _ => None,
            }
        };
        if let Some(generation) = cancel_generation {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                crate::stt::cancel_capture().await;
                finish_hidden(&app, generation).await;
            });
        }
    }

    /// Common teardown: overlay hidden, state machine back to Idle (only
    /// when the generation still matches — a newer session owns it
    /// otherwise).
    async fn finish_hidden(app: &AppHandle, generation: u64) {
        emit_state(app, OverlayState::bare("hidden"));
        tokio::time::sleep(Duration::from_millis(300)).await;
        let hide_now = {
            let mut s = SESSION.lock().unwrap_or_else(|e| e.into_inner());
            if s.generation == generation {
                s.phase = Phase::Idle;
                s.mode = Mode::Hold;
                s.target = None;
                s.terms = Vec::new();
                true
            } else {
                false
            }
        };
        if hide_now {
            let app = app.clone();
            let _ = app.clone().run_on_main_thread(move || {
                overlay_window::hide(&app);
            });
        }
    }

    /// Show the overlay window on the pointer's screen (main thread) and
    /// hand back the geometry for the webview's shape. Placement comes from
    /// preferences per show — both session drivers route through here, so a
    /// Settings change applies to the next session.
    async fn show_overlay(app: &AppHandle) -> Option<crate::overlay_window::NotchGeometry> {
        let placement = overlay_window::OverlayPlacement::from_pref(
            &app.state::<AppState>()
                .preferences_snapshot()
                .dictation
                .overlay_position,
        );
        let (tx, rx) = tokio::sync::oneshot::channel();
        let handle = app.clone();
        let ok = app.run_on_main_thread(move || {
            let mtm = MainThreadMarker::new().expect("run_on_main_thread is the main thread");
            let _ = tx.send(overlay_window::show_on_pointer_screen(&handle, mtm, placement));
        });
        if ok.is_err() {
            return None;
        }
        rx.await.ok().flatten()
    }

    /// Overlay knobs from preferences — sent with every arming transition.
    fn overlay_settings(app: &AppHandle) -> OverlaySettings {
        let d = app.state::<AppState>().preferences_snapshot().dictation;
        OverlaySettings {
            noise_floor: d.overlay_noise_floor,
            preview_chars: d.overlay_preview_chars,
        }
    }

    fn emit_state(app: &AppHandle, state: OverlayState) {
        let _ = app.emit_to(overlay_window::OVERLAY_WINDOW_LABEL, "anywhere://state", state);
    }

    /// Finish (or abort) the active anywhere session — the overlay's stop
    /// button. Returns false when no anywhere session is running, in which
    /// case the caller forwards the stop to the in-app session instead.
    pub fn finish_active(app: &AppHandle) -> bool {
        let action = {
            let mut s = SESSION.lock().unwrap_or_else(|e| e.into_inner());
            match s.phase {
                Phase::Capturing => {
                    s.phase = Phase::Finishing;
                    Some(Some((s.generation, s.target.clone())))
                }
                // Stop during model load = "never mind"; begin_session
                // resolves the abort when the start lands.
                Phase::Arming => {
                    s.phase = Phase::AbortAfterArm;
                    Some(None)
                }
                _ => None,
            }
        };
        match action {
            Some(Some((generation, target))) => {
                spawn_finish(app, generation, target);
                true
            }
            Some(None) => true,
            None => false,
        }
    }

    // --- In-app overlay driver --------------------------------------------
    //
    // The same notch HUD serves the IN-APP local-engine session: the stt
    // commands (`commands::stt`) drive these around start/stop/cancel so
    // dictating inside PortBay and dictating anywhere look identical. The
    // anywhere SESSION machine is not involved — one machine-wide capture
    // exists (micSession serializes its own start/stop, and an anywhere
    // session can't start while PortBay is frontmost), so the two drivers
    // can't interleave; the `anywhere_idle` guard below is the defensive
    // backstop for the pathological overlap.

    /// The in-app session's overlay subject (PortBay itself — captured so
    /// the icon matches whatever frontmost actually was at mic-click) plus
    /// the mode the invoking surface declared (dictation vs voice edit).
    struct InappSession {
        target: Option<FrontTarget>,
        mode: OverlayMode,
    }

    static INAPP: Lazy<Mutex<InappSession>> = Lazy::new(|| {
        Mutex::new(InappSession {
            target: None,
            mode: OverlayMode::Dictation,
        })
    });

    fn anywhere_idle() -> bool {
        SESSION.lock().unwrap_or_else(|e| e.into_inner()).phase == Phase::Idle
    }

    /// Mic clicked in-app: show the notch in its arming look while the
    /// model loads. `mode` is the invoking surface's wire string
    /// ("dictation" | "edit" | "rewrite" — see `OverlayMode::from_wire`).
    pub async fn inapp_arming(app: &AppHandle, mode: &str) {
        if !anywhere_idle() {
            return;
        }
        let mode = OverlayMode::from_wire(mode);
        let (tx, rx) = tokio::sync::oneshot::channel();
        let ok = app.run_on_main_thread(move || {
            let mtm = MainThreadMarker::new().expect("run_on_main_thread is the main thread");
            let _ = tx.send(crate::typing::capture_front_target(mtm));
        });
        let target = match ok {
            Ok(()) => rx.await.ok().flatten(),
            Err(_) => None,
        };
        {
            let mut s = INAPP.lock().unwrap_or_else(|e| e.into_inner());
            s.target = target.clone();
            s.mode = mode;
        }
        let notch = show_overlay(app).await;
        emit_state(
            app,
            OverlayState {
                app_name: target.as_ref().map(|t| t.name.clone()),
                app_icon: target.as_ref().and_then(|t| t.icon_data_url.clone()),
                notch,
                mode,
                settings: Some(overlay_settings(app)),
                ..OverlayState::bare("arming")
            },
        );
    }

    /// Mic hot — the in-app session's live look.
    pub fn inapp_live(app: &AppHandle) {
        if !anywhere_idle() {
            return;
        }
        let (target, mode) = {
            let s = INAPP.lock().unwrap_or_else(|e| e.into_inner());
            (s.target.clone(), s.mode)
        };
        emit_state(
            app,
            OverlayState {
                app_name: target.as_ref().map(|t| t.name.clone()),
                app_icon: target.as_ref().and_then(|t| t.icon_data_url.clone()),
                mode,
                ..OverlayState::bare("live")
            },
        );
    }

    /// Stop requested — final transcription in flight.
    pub fn inapp_processing(app: &AppHandle) {
        if !anywhere_idle() {
            return;
        }
        let (target, mode) = {
            let s = INAPP.lock().unwrap_or_else(|e| e.into_inner());
            (s.target.clone(), s.mode)
        };
        emit_state(
            app,
            OverlayState {
                app_name: target.as_ref().map(|t| t.name.clone()),
                app_icon: target.as_ref().and_then(|t| t.icon_data_url.clone()),
                mode,
                ..OverlayState::bare("processing")
            },
        );
    }

    /// Transcript delivered (the frontend splices it): play the done beat,
    /// then hide. Spawned — the stop command returns the transcript without
    /// waiting out the animations.
    pub fn inapp_done(app: &AppHandle) {
        finish_inapp(app, true);
    }

    /// Failed/cancelled in-app session: just hide (micSession toasts the
    /// failure; the overlay shouldn't double-report).
    pub fn inapp_hidden(app: &AppHandle) {
        finish_inapp(app, false);
    }

    fn finish_inapp(app: &AppHandle, done_beat: bool) {
        {
            let mut s = INAPP.lock().unwrap_or_else(|e| e.into_inner());
            s.target = None;
            s.mode = OverlayMode::Dictation;
        }
        if !anywhere_idle() {
            return;
        }
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if done_beat {
                emit_state(&app, OverlayState::bare("done"));
                tokio::time::sleep(Duration::from_millis(450)).await;
            }
            emit_state(&app, OverlayState::bare("hidden"));
            tokio::time::sleep(Duration::from_millis(300)).await;
            // Hide only if an anywhere session hasn't claimed the window in
            // the meantime (it owns hide/show through its own generation).
            if anywhere_idle() {
                let handle = app.clone();
                let _ = app.run_on_main_thread(move || {
                    overlay_window::hide(&handle);
                });
            }
        });
    }

    /// Re-deliver the newest history entry into the frontmost app — the
    /// tray's "Paste Last Dictation" (freeflow's Paste Again). The target
    /// is captured synchronously (menu handlers run on the main thread and
    /// status-menu tracking doesn't activate us, so frontmost is still the
    /// user's app); delivery degrades to a persistent clipboard copy when
    /// pasting isn't possible — PortBay itself frontmost, no Accessibility,
    /// or the paste failing — so the click always produces the words
    /// somewhere reachable.
    pub fn paste_latest(app: &AppHandle) {
        let target = MainThreadMarker::new().and_then(crate::typing::capture_front_target);
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let Some(entry) = crate::dictation_history::latest() else {
                return;
            };
            let own_pid = std::process::id() as i32;
            match target {
                Some(t) if t.pid != own_pid => {
                    if let Err(detail) =
                        crate::typing::insert_text(&app, entry.text.clone(), t.pid).await
                    {
                        tracing::warn!(detail = %detail, "dictation: paste-again failed, copying instead");
                        let _ = crate::typing::copy_text_persistent(&app, entry.text).await;
                    }
                }
                _ => {
                    let _ = crate::typing::copy_text_persistent(&app, entry.text).await;
                }
            }
        });
    }

    /// Subtle session cues from the system sound set — start when the mic
    /// goes hot, end after the transcript landed. Fire-and-forget.
    fn play_cue(name: &str) {
        let path = format!("/System/Library/Sounds/{name}.aiff");
        if !std::path::Path::new(&path).exists() {
            return;
        }
        tauri::async_runtime::spawn(async move {
            let _ = tokio::process::Command::new("/usr/bin/afplay")
                .args(["-v", "0.3", &path])
                .output()
                .await;
        });
    }

    #[cfg(test)]
    mod tests {
        use super::{anywhere_trigger_allowed, decide_press, resolve_context, PressKind};
        use crate::dictation::RewriteContext;
        use crate::dictation_anywhere::{Mode, OverlayMode, Phase};
        use crate::preferences::AppContextRule;

        // --- App → rewrite-context resolution (Gap 4, minimal) -----------

        #[test]
        fn context_defaults_to_general_note() {
            // Unknown app, no rules → the safe default (fixes invention, adds
            // paragraph layout on clean input).
            assert_eq!(
                resolve_context(Some("com.tinyspeck.slackmacgap"), &[]),
                RewriteContext::GeneralNote
            );
            // No bundle id at all → still GeneralNote, never a panic.
            assert_eq!(resolve_context(None, &[]), RewriteContext::GeneralNote);
        }

        #[test]
        fn terminals_default_to_terminal_command() {
            for id in ["com.apple.Terminal", "com.googlecode.iterm2", "dev.warp.Warp-Stable"] {
                assert_eq!(
                    resolve_context(Some(id), &[]),
                    RewriteContext::TerminalCommand,
                    "{id} should map to TerminalCommand"
                );
            }
            // Bundle ids are matched case-insensitively (NSWorkspace casing
            // isn't guaranteed stable).
            assert_eq!(
                resolve_context(Some("COM.APPLE.TERMINAL"), &[]),
                RewriteContext::TerminalCommand
            );
        }

        #[test]
        fn user_rule_overrides_builtin_and_default() {
            let rules = vec![
                // Override a terminal to a different context...
                AppContextRule {
                    bundle_id: "com.apple.Terminal".into(),
                    context: "git_commit".into(),
                },
                // ...and give a non-terminal app a context.
                AppContextRule {
                    bundle_id: "com.tinyspeck.slackmacgap".into(),
                    context: "agent_prompt".into(),
                },
            ];
            assert_eq!(
                resolve_context(Some("com.apple.Terminal"), &rules),
                RewriteContext::GitCommit
            );
            assert_eq!(
                resolve_context(Some("com.tinyspeck.slackmacgap"), &rules),
                RewriteContext::AgentPrompt
            );
        }

        #[test]
        fn unknown_rule_context_falls_back() {
            // A garbage context string (e.g. a value from a newer build) must
            // not break resolution — fall through to the built-in/default.
            let rules = vec![AppContextRule {
                bundle_id: "com.apple.Terminal".into(),
                context: "not_a_real_context".into(),
            }];
            assert_eq!(
                resolve_context(Some("com.apple.Terminal"), &rules),
                RewriteContext::TerminalCommand
            );
        }

        // --- The enable/routing gate -------------------------------------
        // Mirrors the user-facing contract: the Fn trigger only fires when
        // the feature is ON, the *local* engine is selected, and a model is
        // chosen. Anything else must no-op (and fall through to nothing — the
        // global key never drives macOS dictation).

        #[test]
        fn trigger_allowed_only_when_fully_configured() {
            assert!(anywhere_trigger_allowed(true, "local", "parakeet-v3"));
        }

        #[test]
        fn trigger_blocked_when_feature_off() {
            assert!(!anywhere_trigger_allowed(false, "local", "parakeet-v3"));
        }

        #[test]
        fn trigger_blocked_when_engine_not_local() {
            // The default macOS engine has no global-Fn path.
            assert!(!anywhere_trigger_allowed(true, "macos", "parakeet-v3"));
            assert!(!anywhere_trigger_allowed(true, "", "parakeet-v3"));
        }

        #[test]
        fn trigger_blocked_when_no_model_chosen() {
            // Enabled local engine but the picker is still empty — must not arm.
            assert!(!anywhere_trigger_allowed(true, "local", ""));
        }

        // --- The press transition table ----------------------------------

        #[test]
        fn press_from_idle_single_tap_begins_hold() {
            let (phase, mode, kind) = decide_press(Phase::Idle, Mode::Hold, false);
            assert_eq!((phase, mode, kind), (Phase::Pending, Mode::Hold, PressKind::BeginHold));
            // Incoming mode is irrelevant when starting from Idle.
            let (phase, mode, kind) = decide_press(Phase::Idle, Mode::Toggle, false);
            assert_eq!((phase, mode, kind), (Phase::Pending, Mode::Hold, PressKind::BeginHold));
        }

        #[test]
        fn press_from_idle_double_tap_begins_toggle() {
            let (phase, mode, kind) = decide_press(Phase::Idle, Mode::Hold, true);
            assert_eq!(
                (phase, mode, kind),
                (Phase::Arming, Mode::Toggle, PressKind::BeginToggle)
            );
        }

        #[test]
        fn press_during_toggle_capture_finishes() {
            let (phase, mode, kind) = decide_press(Phase::Capturing, Mode::Toggle, false);
            assert_eq!((phase, mode, kind), (Phase::Finishing, Mode::Toggle, PressKind::Finish));
        }

        #[test]
        fn press_during_toggle_arming_aborts() {
            // Impatient re-tap while the model still loads.
            let (phase, mode, kind) = decide_press(Phase::Arming, Mode::Toggle, false);
            assert_eq!(
                (phase, mode, kind),
                (Phase::AbortAfterArm, Mode::Toggle, PressKind::AbortArm)
            );
        }

        #[test]
        fn press_is_noop_in_hold_capture_and_transient_phases() {
            // A hold-mode capture ends on *release*, never on a press.
            assert_eq!(decide_press(Phase::Capturing, Mode::Hold, false).2, PressKind::None);
            // A second press while a hold session is still pending/finishing
            // (or a hold-mode arming) does nothing.
            for phase in [Phase::Pending, Phase::Finishing, Phase::AbortAfterArm, Phase::Arming] {
                assert_eq!(
                    decide_press(phase, Mode::Hold, false).2,
                    PressKind::None,
                    "{phase:?} in Hold should be a no-op press"
                );
            }
            // A double-tap flag only matters from Idle; mid-session it's ignored.
            assert_eq!(decide_press(Phase::Capturing, Mode::Hold, true).2, PressKind::None);
        }

        #[test]
        fn overlay_mode_from_wire_defaults_to_dictation() {
            assert_eq!(OverlayMode::from_wire("edit"), OverlayMode::Edit);
            assert_eq!(OverlayMode::from_wire("rewrite"), OverlayMode::Rewrite);
            assert_eq!(OverlayMode::from_wire("dictation"), OverlayMode::Dictation);
            // Unknown / empty strings read as plain dictation, never panic.
            assert_eq!(OverlayMode::from_wire("garbage"), OverlayMode::Dictation);
            assert_eq!(OverlayMode::from_wire(""), OverlayMode::Dictation);
        }
    }
}

#[cfg(target_os = "macos")]
pub use macos::{
    ensure_monitors, finish_active, inapp_arming, inapp_done, inapp_hidden, inapp_live,
    inapp_processing, init, on_local_fn, paste_latest, status,
};

#[cfg(not(target_os = "macos"))]
pub fn init(_app: &tauri::AppHandle) {}

/// History only records on macOS, so there is never anything to paste —
/// but the tray item is cross-platform code and needs the symbol.
#[cfg(not(target_os = "macos"))]
pub fn paste_latest(_app: &tauri::AppHandle) {}

#[cfg(not(target_os = "macos"))]
pub fn finish_active(_app: &tauri::AppHandle) -> bool {
    false
}

#[cfg(not(target_os = "macos"))]
pub fn ensure_monitors(_app: &tauri::AppHandle) -> AnywhereStatus {
    status()
}

#[cfg(not(target_os = "macos"))]
pub fn status() -> AnywhereStatus {
    AnywhereStatus {
        supported: false,
        trusted: false,
        monitoring: false,
    }
}
