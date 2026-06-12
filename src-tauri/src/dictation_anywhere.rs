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
    /// PREBUFFER: the capture started at Fn-down went mic-hot and no
    /// transition has consumed it yet. Whoever moves the machine on must
    /// either promote it (→ Capturing) or cancel it — a true flag with no
    /// consumer means a hot mic leaks.
    mic_hot: bool,
    /// The notch overlay is up for this session. Decides which side
    /// completes Arming → Capturing (overlay first vs mic first) and
    /// whether an abort needs to hide anything.
    overlay_shown: bool,
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
        mic_hot: false,
        overlay_shown: false,
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

/// kVK_Escape — the default "cancel dictation" key
/// (`preferences::default_cancel_key`). Lives here, next to the key monitor
/// that consumes the configured code, so the binding semantics and the
/// default stay in one file. Cross-platform on purpose: preferences
/// (de)serialize on every OS even where the monitor doesn't run.
pub const KEY_ESCAPE: u16 = 53;

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
    use crate::dictation::{
        InputSource, ProviderConfig, RewriteContext, RewriteMode, RewriteProvider,
    };
    use crate::overlay_window;
    use crate::preferences::AppContextRule;
    use crate::state::AppState;
    use crate::typing::FrontTarget;

    /// Same hold gate as the in-app push-to-talk disambiguator. Since the
    /// PREBUFFER change this gates only the OVERLAY and the tap/chord
    /// disambiguation — the mic starts at Fn-down (see `early_capture`), so
    /// words spoken immediately are captured; a tap discards them unheard.
    const HOLD_GATE: Duration = Duration::from_millis(300);

    /// How long the window stays up after the "hidden" transition so the
    /// overlay's exit choreography can finish: the content drop-out plus the
    /// collapse spring that morphs the shape back into the physical notch
    /// outline (critically damped, ~450 ms to settle). Ordering the window
    /// out mid-collapse would freeze the shape half-expanded — rAF stops for
    /// occluded windows, so the next show would start from a stale frame.
    const OVERLAY_EXIT_GRACE: Duration = Duration::from_millis(700);

    /// Serializes sidecar capture lifecycle ops (start vs cancel). The
    /// prebuffer makes start/discard overlap real — every Fn-down starts a
    /// capture that a tap discards ~200 ms later, and without ordering the
    /// next session's start could reach the sidecar before the discard and
    /// fail with "a capture session is already active".
    static CAPTURE_FLOW: Lazy<tokio::sync::Mutex<()>> = Lazy::new(|| tokio::sync::Mutex::new(()));
    /// How long the polish rewrite gets before the paste falls back to the raw
    /// transcript. The raw words are already ON SCREEN when this clock runs —
    /// polish is a bonus, and a notch sitting in "Polishing…" much longer
    /// than this reads as a hang. The capture-start prewarm now also primes
    /// the provider's prompt cache (measured 2026-06-09: first rewrite went
    /// 24.8s → 2.6-4.4s once the static head was cached), so a healthy warm
    /// rewrite finishes in single-digit seconds and this cap only catches
    /// genuinely degraded providers.
    const POLISH_TIMEOUT: Duration = Duration::from_secs(15);

    /// How long "Polishing…" may hold the notch open. The raw words are
    /// already in the field — past this the session looks finished (notch
    /// closes, user moves on) and the polish swap lands silently in the
    /// background when it completes. What the polished-dictation products
    /// ship: text first, refinement arrives in place.
    const POLISH_NOTCH_VISIBLE: Duration = Duration::from_secs(4);

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
        // Pre-flight, tightly bounded: a provider that isn't running or has
        // no usable model must never put the notch into "Polishing…" — the
        // raw words are already on screen, so the session just finishes. A
        // healthy local provider answers in ms; an unreachable one must not
        // stall the notch either, hence the cap.
        let available = match tokio::time::timeout(
            Duration::from_millis(1500),
            provider.build().status(),
        )
        .await
        {
            Ok(st) => {
                st.reachable && (st.default_model.is_some() || !provider.model.trim().is_empty())
            }
            Err(_) => false,
        };
        if !available {
            tracing::debug!(
                provider = %provider.kind,
                "dictation: polish provider unavailable; skipping polish (raw kept)"
            );
            return text.to_string();
        }
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
    static MONITORS_INSTALLED: AtomicBool = AtomicBool::new(false);

    /// Install the global monitors at app start when trust already exists.
    /// When it doesn't, `ensure_monitors` re-attempts after the user grants
    /// (the settings toggle and the status command both call it).
    pub fn init(app: &AppHandle, mtm: MainThreadMarker) {
        if crate::typing::ax_trusted() {
            install_monitors(app, mtm);
        } else {
            tracing::info!("dictation: anywhere monitors deferred — accessibility not granted yet");
        }
    }

    /// Idempotent monitor installation. The `MainThreadMarker` makes the
    /// "main thread only" contract a compile-time requirement (it can only be
    /// obtained on the main thread) rather than a comment a future caller can
    /// miss. Returns the resulting status either way.
    pub fn ensure_monitors(app: &AppHandle, mtm: MainThreadMarker) -> AnywhereStatus {
        if !MONITORS_INSTALLED.load(Ordering::SeqCst) && crate::typing::ax_trusted() {
            install_monitors(app, mtm);
        } else if MONITORS_INSTALLED.load(Ordering::SeqCst) {
            // Monitors can predate the feature: trust at launch installs them
            // with `anywhere` still off, and install's warm correctly skips.
            // The arm call only fires on the user's toggle-on / re-check —
            // never a poll — so warming here keeps "enable mid-session" from
            // paying a cold model load on the first Fn-hold. (No-op when the
            // feature is off or the resident engine already holds the model.)
            maybe_prewarm_stt(app);
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
                    let down = event
                        .modifierFlags()
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
    /// load (3-4 s on first use after boot). Fire-and-forget into the resident
    /// `--serve` engine (`stt::EngineProc`), which keeps the model loaded in
    /// RAM between captures — so this warms once and every later capture
    /// reuses the live process. No-op unless the feature is enabled on the
    /// local engine with a model chosen; `stt::prewarm`'s own guard makes a
    /// concurrent in-app prewarm a no-op.
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
        // Physical Fn state, tracked unconditionally — the onboarding hint's
        // sustained-hold detector reads it (a tap must not hint).
        FN_HELD.store(down, Ordering::SeqCst);
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
            (Phase::Idle, _) if double_tap => (Phase::Arming, Mode::Toggle, PressKind::BeginToggle),
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
            // The user is holding Fn like a push-to-talk while the feature
            // is off/unconfigured — the one moment onboarding is wanted.
            maybe_onboarding_hint(app);
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
                    let mtm =
                        MainThreadMarker::new().expect("NSEvent monitor runs on the main thread");
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
                tracing::debug!(generation, "dictation: anywhere Fn-down (hold begins)");
                spawn_favicon_swap(app, generation);
                // PREBUFFER: the mic starts NOW, not after the hold gate —
                // that saved 300 ms (gate) + overlay-show used to come out
                // of every dictation's latency budget AND ate the first
                // word of fast talkers. The gate task only decides whether
                // an overlay appears; a tap discards the capture unheard.
                let capture_app = app.clone();
                let capture_model = model.clone();
                tauri::async_runtime::spawn(async move {
                    early_capture(capture_app, generation, capture_model).await;
                });
                let app = app.clone();
                let pressed_at = std::time::Instant::now();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(HOLD_GATE).await;
                    tracing::debug!(
                        generation,
                        gate_ms = pressed_at.elapsed().as_millis() as u64,
                        "dictation: hold gate fired (overlay next)"
                    );
                    arm_overlay_after_gate(app, generation).await;
                    tracing::debug!(
                        generation,
                        since_press_ms = pressed_at.elapsed().as_millis() as u64,
                        "dictation: overlay armed"
                    );
                });
            }
            PressAction::BeginToggle(generation) => {
                spawn_favicon_swap(app, generation);
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

    /// Browser targets get a better leading glyph: the ACTIVE TAB's site
    /// favicon (dictating into chatgpt.com should show ChatGPT, not Chrome).
    /// Fired at Fn-down right after the target capture, entirely OFF the hot
    /// path: the AppleScript round trip and any favicon fetch run
    /// concurrently with the prebuffer/hold-gate and never delay them. When
    /// the favicon resolves and this generation still owns the session, the
    /// parked target's icon is upgraded in place — so every later transition
    /// that re-reads or re-clones the target (arming after the gate,
    /// processing, polishing, the rescue toast) carries it — and the overlay
    /// is re-emitted in its CURRENT phase so an already-visible notch swaps
    /// live. Every failure leaves the browser icon exactly as it was.
    fn spawn_favicon_swap(app: &AppHandle, generation: u64) {
        let bundle_id = {
            let s = SESSION.lock().unwrap_or_else(|e| e.into_inner());
            if s.generation != generation {
                return;
            }
            s.target.as_ref().and_then(|t| t.bundle_id.clone())
        };
        let Some(bundle_id) = bundle_id else { return };
        // Cheap pre-classification on the monitor thread: non-browser
        // targets (the overwhelmingly common case) never even spawn.
        if crate::favicon::browser_family(&bundle_id).is_none() {
            return;
        }
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let Some(icon) = crate::favicon::active_tab_favicon(&bundle_id).await else {
                return;
            };
            // Swap the parked icon and decide whether to re-emit, briefly
            // under the session lock. Only an overlay that is actually up
            // re-emits; pre-gate phases just park the icon for the arming
            // emit (which re-reads the session target), and Finishing's
            // emits already cloned their target — too late, session ending.
            let emit = {
                let mut s = SESSION.lock().unwrap_or_else(|e| e.into_inner());
                if s.generation != generation {
                    None
                } else if let Some(target) = s.target.as_mut() {
                    target.icon_data_url = Some(icon.clone());
                    let name = target.name.clone();
                    let phase = match s.phase {
                        Phase::Arming if s.overlay_shown => Some("arming"),
                        Phase::Capturing => Some("live"),
                        _ => None,
                    };
                    phase.map(|p| (p, name, s.mode == Mode::Toggle))
                } else {
                    None
                }
            };
            if let Some((phase, app_name, toggle)) = emit {
                tracing::debug!(generation, "dictation: overlay icon swapped to tab favicon");
                emit_state(
                    &app,
                    OverlayState {
                        app_name: Some(app_name),
                        app_icon: Some(icon),
                        toggle,
                        ..OverlayState::bare(phase)
                    },
                );
            }
        });
    }

    /// Fire-and-forget rewrite-model prewarm when polish is on, so the
    /// rewrite at session end doesn't pay the cold load inside its own
    /// timeout — same prewarm-when-anticipated pattern the in-app path uses.
    fn maybe_prewarm_polish(app: &AppHandle) {
        let d = app.state::<AppState>().preferences_snapshot().dictation;
        if d.anywhere_polish {
            crate::dictation::prewarm(&ProviderConfig {
                kind: d.provider,
                endpoint: d.endpoint,
                model: d.model,
            });
        }
    }

    /// Resolve the Context-Store term snapshot for this session (global set
    /// — anywhere has no project surface, §10.3). The rewrite context is
    /// inferred from the target app so the learned-jargon ranking matches
    /// what the finish-time rewrite will use. The full snapshot is parked on
    /// the session (correction net + rewrite reuse the exact same set); the
    /// returned bias is the engine-gated slice of it (never a prompt the
    /// engine can't apply).
    async fn resolve_bias(
        app: &AppHandle,
        target: Option<&FrontTarget>,
        model: &str,
        generation: u64,
    ) -> Vec<String> {
        let context = {
            let prefs = app.state::<AppState>().preferences_snapshot().dictation;
            resolve_context(
                target.and_then(|t| t.bundle_id.as_deref()),
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
            && crate::dictation_context::engine_supports_text_bias(model)
        {
            crate::dictation_context::instrument::record_bias(snapshot.len());
            tracing::debug!(model = %model, terms = snapshot.len(), "dictation: anywhere recognizer bias resolved");
            snapshot.clone()
        } else {
            // Default: no recognizer bias (gated/unsupported — see
            // `recognizer_bias_enabled`). The always-on correction net and
            // the rewrite still fix known terms from the parked snapshot.
            Vec::new()
        };
        {
            let mut s = SESSION.lock().unwrap_or_else(|e| e.into_inner());
            // Only park the snapshot if this session still owns the slot — a
            // hold/toggle race across the await above must not install
            // another session's vocab.
            if s.generation == generation {
                s.terms = snapshot;
            }
        }
        bias
    }

    /// Physical Fn key state (set on press, cleared on release), tracked in
    /// `on_fn` regardless of feature gates — the onboarding hint's
    /// sustained-hold detector reads it.
    static FN_HELD: AtomicBool = AtomicBool::new(false);
    /// Once-per-run guard for the onboarding hint — a discovery nudge, not a
    /// nag. Accidental Fn taps (emoji picker, input switching) never hint;
    /// only a sustained hold does.
    static ONBOARD_HINTED: AtomicBool = AtomicBool::new(false);
    /// How long Fn must stay down before "they're holding it like
    /// push-to-talk" is credible. Well past the 300 ms session hold-gate and
    /// past a deliberate emoji-picker press.
    const ONBOARD_HOLD: Duration = Duration::from_millis(1200);

    /// Fn held while "Dictate anywhere" is off/unconfigured: the user expects
    /// dictation and gets silence — the exact moment to onboard (the app is
    /// usually in the BACKGROUND here, so an in-window toast would never be
    /// seen; the notch overlay floats over everything). Once per run, and
    /// only on a sustained hold.
    fn maybe_onboarding_hint(app: &AppHandle) {
        if ONBOARD_HINTED.load(Ordering::SeqCst) {
            return;
        }
        // Never hint over PortBay itself — an in-app Fn hold belongs to the
        // in-app push-to-talk flow (this runs on the monitor's main thread,
        // so the marker always resolves).
        if let Some(mtm) = MainThreadMarker::new() {
            if let Some(front) = crate::typing::capture_front_target(mtm) {
                if front.pid == std::process::id() as i32 {
                    return;
                }
            }
        }
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(ONBOARD_HOLD).await;
            if !FN_HELD.load(Ordering::SeqCst) {
                return; // tap or chord, not a push-to-talk hold
            }
            if ONBOARD_HINTED.swap(true, Ordering::SeqCst) {
                return;
            }
            tracing::info!(
                "dictation: sustained Fn hold with dictate-anywhere off — showing onboarding hint"
            );
            let _ = show_overlay(&app).await;
            emit_state(
                &app,
                OverlayState {
                    error: Some(
                        "Dictate anywhere is off — turn it on in PortBay under AI → Speech to Text"
                            .into(),
                    ),
                    ..OverlayState::bare("error")
                },
            );
            tokio::time::sleep(Duration::from_millis(4000)).await;
            // Hide only if no real session took the overlay over meanwhile
            // (the user may have enabled the feature and started dictating).
            let idle = {
                let s = SESSION.lock().unwrap_or_else(|e| e.into_inner());
                s.phase == Phase::Idle
            };
            if idle {
                emit_state(&app, OverlayState::bare("hidden"));
                tokio::time::sleep(OVERLAY_EXIT_GRACE).await;
                let handle = app.clone();
                let _ = app.run_on_main_thread(move || {
                    overlay_window::hide(&handle);
                });
            }
        });
    }

    /// Bring the overlay up to report a failed capture start, then tear the
    /// session down after the user has had a beat to read it.
    async fn fail_session(app: &AppHandle, generation: u64, detail: String, overlay_up: bool) {
        tracing::warn!(detail = %detail, "dictation: anywhere capture failed to start");
        if !overlay_up {
            // Pre-gate failure: no overlay yet, but the user is mid-hold
            // expecting dictation — silence here is the P0 "Fn does
            // nothing" class. Bring the notch up just to say why.
            let _ = show_overlay(app).await;
        }
        emit_state(
            app,
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

    /// PREBUFFER half of a hold session: start the sidecar capture the
    /// instant Fn goes down. Runs concurrently with the hold-gate timer
    /// (`arm_overlay_after_gate`); the two coordinate through
    /// `mic_hot`/`overlay_shown` under the session lock, and whichever
    /// completes second performs the Arming → Capturing transition. A tap
    /// or chord abandons the session and the capture is cancelled wherever
    /// its start happens to be in flight.
    async fn early_capture(app: AppHandle, generation: u64, model: String) {
        let target = {
            let s = SESSION.lock().unwrap_or_else(|e| e.into_inner());
            s.target.clone()
        };
        maybe_prewarm_polish(&app);
        let models_dir = crate::ollama::expand_tilde(
            &app.state::<AppState>()
                .preferences_snapshot()
                .stt
                .models_dir,
        );
        let bias = resolve_bias(&app, target.as_ref(), &model, generation).await;
        tracing::debug!(
            generation,
            "dictation: prebuffer bias resolved; starting capture"
        );

        let flow = CAPTURE_FLOW.lock().await;
        // The session may already be dead (tap released during the bias
        // resolve) — then nothing was started and nothing needs cancelling.
        {
            let s = SESSION.lock().unwrap_or_else(|e| e.into_inner());
            if s.generation != generation || !matches!(s.phase, Phase::Pending | Phase::Arming) {
                return;
            }
        }
        let start_at = std::time::Instant::now();
        let started = crate::stt::start_capture(app.clone(), &models_dir, &model, &bias).await;
        tracing::debug!(
            generation,
            start_ms = start_at.elapsed().as_millis() as u64,
            ok = started.is_ok(),
            "dictation: prebuffer capture start returned"
        );

        match started {
            Ok(()) => {
                enum Next {
                    /// Mic hot, but the overlay side isn't ready — parked on
                    /// the session for the gate task to promote.
                    Parked,
                    /// Overlay already up: this side promotes to Capturing.
                    Live,
                    /// Session abandoned with the overlay up: discard + hide.
                    CancelHide,
                    /// Session abandoned pre-overlay (tap): discard quietly.
                    Cancel,
                }
                let next = {
                    let mut s = SESSION.lock().unwrap_or_else(|e| e.into_inner());
                    if s.generation != generation {
                        Next::Cancel
                    } else {
                        match s.phase {
                            Phase::Pending => {
                                s.mic_hot = true;
                                Next::Parked
                            }
                            Phase::Arming if !s.overlay_shown => {
                                s.mic_hot = true;
                                Next::Parked
                            }
                            Phase::Arming => {
                                s.phase = Phase::Capturing;
                                Next::Live
                            }
                            Phase::AbortAfterArm => Next::CancelHide,
                            // Idle (tap already settled) / Finishing — discard.
                            _ => Next::Cancel,
                        }
                    }
                };
                match next {
                    Next::Parked => {}
                    Next::Live => {
                        drop(flow);
                        emit_state(
                            &app,
                            OverlayState {
                                app_name: target.as_ref().map(|t| t.name.clone()),
                                app_icon: target.as_ref().and_then(|t| t.icon_data_url.clone()),
                                ..OverlayState::bare("live")
                            },
                        );
                        play_start_cue(&app);
                    }
                    Next::CancelHide => {
                        crate::stt::cancel_capture().await;
                        drop(flow);
                        finish_hidden(&app, generation).await;
                    }
                    Next::Cancel => {
                        crate::stt::cancel_capture().await;
                    }
                }
            }
            Err(detail) => {
                drop(flow);
                let (dead, overlay_up) = {
                    let mut s = SESSION.lock().unwrap_or_else(|e| e.into_inner());
                    if s.generation != generation {
                        (true, false)
                    } else {
                        let overlay_up = s.overlay_shown;
                        s.phase = Phase::Idle;
                        s.mic_hot = false;
                        (false, overlay_up)
                    }
                };
                if !dead {
                    fail_session(&app, generation, detail, overlay_up).await;
                }
            }
        }
    }

    /// Overlay half of a hold session: after the 300 ms gate confirms this
    /// is a hold (not a tap/chord), show the arming overlay — and promote to
    /// Capturing if the prebuffer capture already went mic-hot meanwhile.
    async fn arm_overlay_after_gate(app: AppHandle, generation: u64) {
        {
            let mut s = SESSION.lock().unwrap_or_else(|e| e.into_inner());
            if s.generation != generation || s.phase != Phase::Pending {
                return;
            }
            s.phase = Phase::Arming;
        }
        let target = {
            let s = SESSION.lock().unwrap_or_else(|e| e.into_inner());
            s.target.clone()
        };
        let notch = show_overlay(&app).await;
        emit_state(
            &app,
            OverlayState {
                app_name: target.as_ref().map(|t| t.name.clone()),
                app_icon: target.as_ref().and_then(|t| t.icon_data_url.clone()),
                notch,
                settings: Some(overlay_settings(&app)),
                ..OverlayState::bare("arming")
            },
        );
        // The capture may have gone mic-hot during the gate/show — promote.
        // It may also have been abandoned mid-show (release raced the
        // overlay): then the abort path's hide may have run BEFORE our show,
        // so hide again rather than leave an orphan notch.
        enum After {
            Promote,
            Rehide,
            Wait,
        }
        let after = {
            let mut s = SESSION.lock().unwrap_or_else(|e| e.into_inner());
            if s.generation != generation {
                After::Rehide
            } else {
                s.overlay_shown = true;
                match s.phase {
                    Phase::Arming if s.mic_hot => {
                        s.mic_hot = false;
                        s.phase = Phase::Capturing;
                        After::Promote
                    }
                    Phase::Idle | Phase::AbortAfterArm => After::Rehide,
                    _ => After::Wait,
                }
            }
        };
        match after {
            After::Promote => {
                emit_state(
                    &app,
                    OverlayState {
                        app_name: target.as_ref().map(|t| t.name.clone()),
                        app_icon: target.as_ref().and_then(|t| t.icon_data_url.clone()),
                        ..OverlayState::bare("live")
                    },
                );
                play_start_cue(&app);
            }
            After::Rehide => {
                finish_hidden(&app, generation).await;
            }
            After::Wait => {}
        }
    }

    /// Cancel a prebuffer capture whose session was abandoned after it went
    /// mic-hot (the early task has already returned, so nobody else will).
    /// `hide` tears the overlay down too; a pre-gate tap never showed one.
    fn spawn_abort_capture(app: &AppHandle, generation: u64, hide: bool) {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            {
                let _flow = CAPTURE_FLOW.lock().await;
                crate::stt::cancel_capture().await;
            }
            if hide {
                finish_hidden(&app, generation).await;
            }
        });
    }

    /// Hands-free (toggle) session: show the overlay (arming look) and start
    /// the sidecar capture. The await spans the model cold-load; the
    /// overlay's arming state is honest about it, exactly like the in-app
    /// `arming` phase. Hold sessions don't come here anymore — they split
    /// across `early_capture` + `arm_overlay_after_gate` for the prebuffer.
    async fn begin_session(app: AppHandle, generation: u64, model: String) {
        let (target, toggle) = {
            let s = SESSION.lock().unwrap_or_else(|e| e.into_inner());
            (s.target.clone(), s.mode == Mode::Toggle)
        };
        maybe_prewarm_polish(&app);
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
        SESSION
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .overlay_shown = true;

        let models_dir = crate::ollama::expand_tilde(
            &app.state::<AppState>()
                .preferences_snapshot()
                .stt
                .models_dir,
        );
        let bias = resolve_bias(&app, target.as_ref(), &model, generation).await;

        // Ordered behind any pending discard of a prebuffer capture (the
        // first tap of this very double-tap started one).
        let flow = CAPTURE_FLOW.lock().await;
        let started = crate::stt::start_capture(app.clone(), &models_dir, &model, &bias).await;

        let mut dead = false;
        let next: Phase = {
            let mut s = SESSION.lock().unwrap_or_else(|e| e.into_inner());
            if s.generation != generation {
                // Superseded (defensive — Arming blocks new sessions, so
                // this shouldn't occur). A capture that landed anyway has
                // no owner: discard it; the new owner manages the overlay.
                dead = true;
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
                drop(flow);
                emit_state(
                    &app,
                    OverlayState {
                        app_name: target.as_ref().map(|t| t.name.clone()),
                        app_icon: target.as_ref().and_then(|t| t.icon_data_url.clone()),
                        toggle,
                        ..OverlayState::bare("live")
                    },
                );
                play_start_cue(&app);
            }
            Phase::AbortAfterArm => {
                crate::stt::cancel_capture().await;
                drop(flow);
                finish_hidden(&app, generation).await;
            }
            _ => {
                drop(flow);
                // A dead session's failure is not ours to report — the new
                // owner manages the overlay (mirrors early_capture's check).
                if let Err(detail) = started {
                    if !dead {
                        fail_session(&app, generation, detail, true).await;
                    }
                }
            }
        }
    }

    fn fn_released(app: &AppHandle) {
        enum Release {
            None,
            Finish(u64, Option<FrontTarget>),
            /// A prebuffer capture went hot and its session just died here —
            /// the early task has returned, so this release must discard it.
            Abort {
                generation: u64,
                hide: bool,
            },
        }
        let tap_toggle = app
            .state::<AppState>()
            .preferences_snapshot()
            .dictation
            .anywhere_tap_toggle;
        let action = {
            let mut s = SESSION.lock().unwrap_or_else(|e| e.into_inner());
            match (s.phase, s.mode) {
                // Automatic mode: a release inside the hold gate IS the
                // hands-free start — the prebuffer capture keeps rolling
                // and the gate task arms the overlay as usual; the next Fn
                // press (or the cancel key / EOU auto-stop) ends it.
                (Phase::Pending, _) if tap_toggle => {
                    s.mode = Mode::Toggle;
                    s.last_tap = None;
                    Release::None
                }
                // Quick tap — the emoji picker / input-source switch, not
                // us; but note the time, it may be half a toggle double-tap.
                (Phase::Pending, _) => {
                    s.phase = Phase::Idle;
                    s.last_tap = Some(std::time::Instant::now());
                    if std::mem::take(&mut s.mic_hot) {
                        Release::Abort {
                            generation: s.generation,
                            hide: false,
                        }
                    } else {
                        Release::None
                    }
                }
                (Phase::Arming, Mode::Hold) => {
                    s.phase = Phase::AbortAfterArm;
                    if std::mem::take(&mut s.mic_hot) {
                        Release::Abort {
                            generation: s.generation,
                            hide: true,
                        }
                    } else {
                        // Start still in flight — the early task sees
                        // AbortAfterArm when it resolves and discards there.
                        Release::None
                    }
                }
                (Phase::Capturing, Mode::Hold) => {
                    s.phase = Phase::Finishing;
                    Release::Finish(s.generation, s.target.clone())
                }
                // Hands-free sessions ignore releases — the starting
                // double-tap's own release lands here; only the next Fn
                // PRESS (or Esc) stops the capture.
                _ => Release::None,
            }
        };
        match action {
            Release::Finish(generation, target) => spawn_finish(app, generation, target),
            Release::Abort { generation, hide } => spawn_abort_capture(app, generation, hide),
            Release::None => {}
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
            // Whether the notch is still on screen by the time the session
            // wraps up — false once a slow polish closed it early (the swap
            // then finishes in the background).
            let mut notch_up = true;
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
                let terms = SESSION
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .terms
                    .clone();
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

                // PASTE-FIRST (sub-2-3 s target). Get the user's words on screen
                // *now* — typed into the field ~0.5 s after Fn-release — instead
                // of blocking the paste on the LLM polish (the cold/warm rewrite
                // is what made it "wait, then process, then paste"). Polish then
                // runs (its model is prewarmed at capture start) and, when it
                // changes the text, swaps it in place — see below.
                let mut inserted = false;
                if let Some(target) = &target {
                    match crate::typing::insert_text(&app, corrected.clone(), target.pid).await {
                        Ok(()) => inserted = true,
                        Err(detail) => {
                            tracing::warn!(detail = %detail, "dictation: anywhere insertion failed");
                        }
                    }
                }

                // Polish AFTER the raw paste. The notch shows "Polishing…" and
                // streams the rewrite preview; `maybe_polish` degrades to the raw
                // words on any failure/timeout. The notch only stays up for
                // [`POLISH_NOTCH_VISIBLE`] though: the raw words are already
                // delivered, so past that the session LOOKS done (notch closes,
                // user moves on) and the swap lands silently in the background
                // when the rewrite finishes — the guarded replace makes a late
                // swap safe by construction (any caret/text drift keeps raw).
                let polish = maybe_polish(&app, &corrected, target.as_ref(), generation, &terms);
                tokio::pin!(polish);
                let polished = match tokio::time::timeout(POLISH_NOTCH_VISIBLE, polish.as_mut())
                    .await
                {
                    Ok(polished) => polished,
                    Err(_) => {
                        tracing::debug!(
                            "dictation: polish still running — closing the notch, swap continues in background"
                        );
                        notch_up = false;
                        emit_state(&app, OverlayState::bare("done"));
                        tokio::time::sleep(Duration::from_millis(450)).await;
                        finish_hidden(&app, generation).await;
                        polish.await
                    }
                };

                // Resolve what actually ended up in the field. When polish
                // changed the text, swap it in place — but ONLY via the guarded
                // replace, which succeeds only when the words right before the
                // caret are still exactly what we pasted (native AX-readable
                // fields, user hasn't typed on). In web editors / on any drift it
                // returns false and the fast raw words simply stay — it can never
                // delete anything else the user had.
                let mut delivered = corrected.clone();
                let mut rescue: Option<String> = None;
                if polished != corrected {
                    if inserted {
                        if let Some(target) = &target {
                            if crate::typing::replace_recent_insertion(
                                &app, &corrected, &polished, target.pid,
                            )
                            .await
                            {
                                delivered = polished;
                            } else if crate::typing::replace_recent_insertion_via_keys(
                                &app, &corrected, &polished, target.pid,
                            )
                            .await
                            {
                                // Web-editor fallback: the AX write path is
                                // closed there, so the swap goes select-back →
                                // verify-readback → paste. Same safety
                                // contract — any uncertainty keeps the raw.
                                delivered = polished;
                            }
                        }
                    } else {
                        // Never landed in the field — rescue the polished text.
                        delivered = polished;
                    }
                }

                // The raw paste failed (focus moved before it landed): the words
                // aren't anywhere yet, so leave the best version on the clipboard
                // — persistently, no restore — and tell the user where they went.
                if !inserted && target.is_some() {
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

                // History keeps the ORIGINAL transcript recoverable whenever
                // commands, correction, or polish changed what was delivered
                // (tray "Paste Last Dictation" + the settings recent list).
                let raw_original = (delivered != trimmed).then(|| trimmed.to_string());
                crate::dictation_history::record(
                    &delivered,
                    raw_original,
                    target.as_ref().map(|t| t.name.clone()),
                    inserted,
                );
                crate::tray::refresh_dictation_item(&app);

                if let Some(message) = rescue {
                    // The one time the cue fires: the transcript couldn't be
                    // placed in the target field. An audible flag for the
                    // eyes-free case where the words silently went to the
                    // clipboard instead of the cursor.
                    play_failure_cue(&app);
                    if !notch_up {
                        // The notch already closed (background polish) —
                        // bring it back to deliver the rescue message.
                        let _ = show_overlay(&app).await;
                    }
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
                    if notch_up {
                        finish_hidden(&app, generation).await;
                    } else {
                        // The session was already torn down when the notch
                        // closed; just hide the re-shown window.
                        emit_state(&app, OverlayState::bare("hidden"));
                        tokio::time::sleep(OVERLAY_EXIT_GRACE).await;
                        let handle = app.clone();
                        let _ = app.run_on_main_thread(move || {
                            overlay_window::hide(&handle);
                        });
                    }
                    return;
                }
            }
            if notch_up {
                emit_state(&app, OverlayState::bare("done"));
                // Let the overlay play its exit animation before the window
                // disappears under it.
                tokio::time::sleep(Duration::from_millis(450)).await;
                finish_hidden(&app, generation).await;
            }
        });
    }

    /// Esc cancels a session in flight; any other key during the hold gate
    /// is an Fn-chord (Fn+arrow etc.) and aborts the pending trigger — the
    /// same guard the in-app disambiguator applies. A toggle session gets
    /// the equivalent chord guard while Arming (its tap-tap prefix can be
    /// the start of "tap Fn, then Fn+arrow"); once the mic is hot, keys are
    /// the user typing alongside their hands-free dictation and pass.
    fn on_key_down(app: &AppHandle, key_code: u16) {
        enum Key {
            None,
            CancelLive(u64),
            /// Abandoned with a hot prebuffer capture and no pending early
            /// task — discard it here (see `fn_released`'s twin).
            Abort {
                generation: u64,
                hide: bool,
            },
        }
        let action = {
            let mut s = SESSION.lock().unwrap_or_else(|e| e.into_inner());
            if s.phase == Phase::Idle {
                // The common path for every global keystroke: no session, no
                // preference read, out immediately.
                Key::None
            } else {
                // The cancel key is user-configurable (Esc by default); only
                // consulted while a session is in flight.
                let cancel = app
                    .state::<AppState>()
                    .preferences_snapshot()
                    .dictation
                    .anywhere_cancel_key;
                match (s.phase, key_code) {
                    (Phase::Pending, _) => {
                        s.phase = Phase::Idle;
                        if std::mem::take(&mut s.mic_hot) {
                            Key::Abort {
                                generation: s.generation,
                                hide: false,
                            }
                        } else {
                            Key::None
                        }
                    }
                    (Phase::Arming, k) if k == cancel => {
                        s.phase = Phase::AbortAfterArm;
                        if std::mem::take(&mut s.mic_hot) {
                            Key::Abort {
                                generation: s.generation,
                                hide: true,
                            }
                        } else {
                            Key::None
                        }
                    }
                    (Phase::Arming, _) if s.mode == Mode::Toggle => {
                        s.phase = Phase::AbortAfterArm;
                        Key::None
                    }
                    (Phase::Capturing, k) if k == cancel => {
                        s.phase = Phase::Finishing;
                        Key::CancelLive(s.generation)
                    }
                    _ => Key::None,
                }
            }
        };
        match action {
            Key::CancelLive(generation) => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    {
                        let _flow = CAPTURE_FLOW.lock().await;
                        crate::stt::cancel_capture().await;
                    }
                    finish_hidden(&app, generation).await;
                });
            }
            Key::Abort { generation, hide } => spawn_abort_capture(app, generation, hide),
            Key::None => {}
        }
    }

    /// Common teardown: overlay hidden, state machine back to Idle (only
    /// when the generation still matches — a newer session owns it
    /// otherwise).
    async fn finish_hidden(app: &AppHandle, generation: u64) {
        emit_state(app, OverlayState::bare("hidden"));
        tokio::time::sleep(OVERLAY_EXIT_GRACE).await;
        let hide_now = {
            let mut s = SESSION.lock().unwrap_or_else(|e| e.into_inner());
            if s.generation == generation {
                s.phase = Phase::Idle;
                s.mode = Mode::Hold;
                s.target = None;
                s.terms = Vec::new();
                s.mic_hot = false;
                s.overlay_shown = false;
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
            let _ = tx.send(overlay_window::show_on_pointer_screen(
                &handle, mtm, placement,
            ));
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
        let _ = app.emit_to(
            overlay_window::OVERLAY_WINDOW_LABEL,
            "anywhere://state",
            state,
        );
    }

    /// Streaming-engine End-of-Utterance (relayed by `stt::route_event`).
    /// Only a hands-free (toggle) session with the auto-stop preference on
    /// treats it as the stop signal: hold sessions end on Fn release (the
    /// held key IS the stop), and in-app sessions belong to micSession.
    /// Engines without native EOU detection simply never send this.
    pub fn on_eou(app: &AppHandle) {
        if !app
            .state::<AppState>()
            .preferences_snapshot()
            .dictation
            .anywhere_auto_stop
        {
            return;
        }
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
    }

    /// A live capture's engine failed BEHIND the mic (relayed by
    /// `stt::read_loop`): with the mic-first sidecar, `listening` resolves the
    /// start while the model still loads, so a load failure can land after the
    /// session went live. Fail the visible session now instead of leaving the
    /// notch "recording" until the user releases into a dead engine. In-app
    /// sessions (anywhere idle) are untouched — micSession surfaces the error
    /// at its own stop, which the dropped routing makes fail fast.
    pub fn on_capture_lost(app: &AppHandle, detail: String) {
        let failing = {
            let s = SESSION.lock().unwrap_or_else(|e| e.into_inner());
            matches!(s.phase, Phase::Pending | Phase::Arming | Phase::Capturing)
                .then_some((s.generation, s.overlay_shown))
        };
        let Some((generation, overlay_up)) = failing else {
            return;
        };
        tracing::warn!(detail = %detail, "dictation: engine failed behind a live capture");
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            fail_session(&app, generation, detail, overlay_up).await;
        });
    }

    /// Finish (or abort) the active anywhere session — the overlay's stop
    /// button. Returns false when no anywhere session is running, in which
    /// case the caller forwards the stop to the in-app session instead.
    pub fn finish_active(app: &AppHandle) -> bool {
        enum Stop {
            Finish(u64, Option<FrontTarget>),
            /// Mic-hot prebuffer capture with no pending early task —
            /// discard here (the in-flight case resolves in `early_capture`
            /// / `begin_session` via AbortAfterArm).
            Abort(u64),
            Acknowledged,
            NotOurs,
        }
        let action = {
            let mut s = SESSION.lock().unwrap_or_else(|e| e.into_inner());
            match s.phase {
                Phase::Capturing => {
                    s.phase = Phase::Finishing;
                    Stop::Finish(s.generation, s.target.clone())
                }
                // Stop during model load = "never mind".
                Phase::Arming => {
                    s.phase = Phase::AbortAfterArm;
                    if std::mem::take(&mut s.mic_hot) {
                        Stop::Abort(s.generation)
                    } else {
                        Stop::Acknowledged
                    }
                }
                _ => Stop::NotOurs,
            }
        };
        match action {
            Stop::Finish(generation, target) => {
                spawn_finish(app, generation, target);
                true
            }
            Stop::Abort(generation) => {
                spawn_abort_capture(app, generation, true);
                true
            }
            Stop::Acknowledged => true,
            Stop::NotOurs => false,
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
            tokio::time::sleep(OVERLAY_EXIT_GRACE).await;
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
    /// goes hot, and a failure flag when the transcript couldn't be placed
    /// in the target field. A clean, placed transcript stays silent: the
    /// words in the field are confirmation enough. Fire-and-forget. The
    /// start sound and the volume are preferences; the failure cue always
    /// uses Pop (it's a safety signal, only the volume applies).
    fn play_start_cue(app: &AppHandle) {
        let d = app.state::<AppState>().preferences_snapshot().dictation;
        if d.anywhere_cue_sound.is_empty() {
            return; // "None" — silent start
        }
        play_cue_file(&d.anywhere_cue_sound, d.anywhere_cue_volume);
    }

    fn play_failure_cue(app: &AppHandle) {
        let d = app.state::<AppState>().preferences_snapshot().dictation;
        play_cue_file("Pop", d.anywhere_cue_volume);
    }

    /// One-shot cue playback for the Settings picker — selecting a sound (or
    /// moving the volume slider) echoes the choice, like macOS's alert-sound
    /// list. Routed through `play_cue_file` so the preview is byte-identical
    /// to what a real session start will play (same sanitizing, same clamp).
    pub fn preview_cue(name: &str, volume: f32) {
        play_cue_file(name, volume);
    }

    fn play_cue_file(name: &str, volume: f32) {
        // The name lands in a filesystem path — accept bare system-sound
        // names only.
        if !name.chars().all(|c| c.is_ascii_alphanumeric()) {
            return;
        }
        let volume = volume.clamp(0.0, 1.0);
        if volume <= 0.0 {
            return;
        }
        let path = format!("/System/Library/Sounds/{name}.aiff");
        if !std::path::Path::new(&path).exists() {
            return;
        }
        tauri::async_runtime::spawn(async move {
            let _ = tokio::process::Command::new("/usr/bin/afplay")
                .args(["-v", &format!("{volume:.2}"), &path])
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
            for id in [
                "com.apple.Terminal",
                "com.googlecode.iterm2",
                "dev.warp.Warp-Stable",
            ] {
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
            assert_eq!(
                (phase, mode, kind),
                (Phase::Pending, Mode::Hold, PressKind::BeginHold)
            );
            // Incoming mode is irrelevant when starting from Idle.
            let (phase, mode, kind) = decide_press(Phase::Idle, Mode::Toggle, false);
            assert_eq!(
                (phase, mode, kind),
                (Phase::Pending, Mode::Hold, PressKind::BeginHold)
            );
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
            assert_eq!(
                (phase, mode, kind),
                (Phase::Finishing, Mode::Toggle, PressKind::Finish)
            );
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
            assert_eq!(
                decide_press(Phase::Capturing, Mode::Hold, false).2,
                PressKind::None
            );
            // A second press while a hold session is still pending/finishing
            // (or a hold-mode arming) does nothing.
            for phase in [
                Phase::Pending,
                Phase::Finishing,
                Phase::AbortAfterArm,
                Phase::Arming,
            ] {
                assert_eq!(
                    decide_press(phase, Mode::Hold, false).2,
                    PressKind::None,
                    "{phase:?} in Hold should be a no-op press"
                );
            }
            // A double-tap flag only matters from Idle; mid-session it's ignored.
            assert_eq!(
                decide_press(Phase::Capturing, Mode::Hold, true).2,
                PressKind::None
            );
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
    inapp_processing, init, on_capture_lost, on_eou, on_local_fn, paste_latest, preview_cue,
    status,
};

#[cfg(not(target_os = "macos"))]
pub fn init(_app: &tauri::AppHandle) {}

/// Cues come from /System/Library/Sounds, so there is nothing to preview
/// off macOS — but the Settings command is cross-platform code.
#[cfg(not(target_os = "macos"))]
pub fn preview_cue(_name: &str, _volume: f32) {}

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
