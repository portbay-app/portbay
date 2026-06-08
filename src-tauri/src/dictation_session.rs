//! macOS system-dictation session tracking + diagnostics.
//!
//! Why this exists: `startDictation:` / `stopDictation:` don't start/stop a
//! session — they advance the **global** dictation state machine
//! (NSDictationManager → TSM "ironwood" → DictationIM), which is effectively a
//! toggle shared across every app. macOS routinely ends the *audio* session on
//! its own (silence timeout, HUD "Done") while leaving dictation *mode*
//! engaged — and an app firing blind toggles drifts out of phase: its next
//! "start" actually exits the phantom mode, which reads as "dictation never
//! starts". Left alone the state machine eventually wedges until DictationIM
//! is killed. (Diagnosed live on macOS 26.2; a plain `startDictation:` works
//! first try in a fresh state, so entitlements/Info.plist were never the
//! problem — audio capture for system dictation happens in `corespeechd`, not
//! the app, which is also why no mic TCC entry ever appears.)
//!
//! The fix: DictationIM posts distributed notifications at exactly the two
//! transitions we care about —
//!   • `DictationIMNotificationStartedListening`  (session live, mic hot)
//!   • `DictationIMNotificationDidExitDictationMode` (mode fully ended)
//! — so we subscribe once at startup and mirror the OS truth. `start_dictation`
//! then *confirms* a start (and un-wedges a stale mode by retrying when its
//! toggle lands as an exit), `stop_dictation` only sends a stop when a session
//! is actually live, and the frontend recording UI follows the
//! `dictation://listening` / `dictation://ended` events instead of guessing
//! from field blur.
//!
//! Everything here is local system state; no audio or text leaves the machine.

#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
#[cfg(target_os = "macos")]
use std::time::Instant;

use once_cell::sync::Lazy;
use tokio::sync::watch;

/// An observed OS dictation transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OsEvent {
    /// DictationIM started listening — the session is live, mic is hot.
    Listening,
    /// DictationIM fully exited dictation mode.
    Exited,
}

/// Monotonic event feed: `(seq, last_event)`. Subscribers snapshot with
/// `borrow_and_update()` before acting, then `changed().await` — the seq makes
/// every transition observable even when two arrive back-to-back.
static EVENTS: Lazy<watch::Sender<(u64, Option<OsEvent>)>> =
    Lazy::new(|| watch::channel((0, None)).0);

#[cfg(target_os = "macos")]
static SEQ: AtomicU64 = AtomicU64::new(0);

/// `StartedListening` seen with no `DidExitDictationMode` since. Seeded
/// `false` at launch; a stale mode left by a previous run is healed by the
/// retry in `start_dictation` (its toggle lands as `Exited`, we re-send).
#[cfg(target_os = "macos")]
static ACTIVE: AtomicBool = AtomicBool::new(false);

/// Any `StartedListening` seen this app run. The first start of a run pays
/// DictationIM + `corespeechd` cold spawn (and possibly a model page-in), so
/// `start_dictation` grants it a longer confirmation window.
#[cfg(target_os = "macos")]
static EVER_LISTENED: AtomicBool = AtomicBool::new(false);

/// Whether a dictation session has been confirmed at least once this run.
pub fn ever_listened() -> bool {
    #[cfg(target_os = "macos")]
    {
        EVER_LISTENED.load(Ordering::SeqCst)
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// Last transition + when, for diagnostics.
#[cfg(target_os = "macos")]
static LAST_EVENT: Lazy<std::sync::Mutex<Option<(OsEvent, Instant)>>> =
    Lazy::new(|| std::sync::Mutex::new(None));

/// When the last REAL session teardown happened — an `Exited` that ended a
/// session we saw listening. Distinct from `LAST_EVENT` because DictationIM
/// posts `DidExitDictationMode` liberally (pairs at app launch, on focus
/// changes — observed live 2026-06-06, seq 1–12 with no session anywhere);
/// only an exit that follows a live session has a teardown window worth
/// waiting out.
#[cfg(target_os = "macos")]
static LAST_TEARDOWN: Lazy<std::sync::Mutex<Option<Instant>>> =
    Lazy::new(|| std::sync::Mutex::new(None));

/// Whether the OS dictation session is live right now (as far as the
/// notification feed has told us).
pub fn os_session_active() -> bool {
    #[cfg(target_os = "macos")]
    {
        ACTIVE.load(Ordering::SeqCst)
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// Subscribe to the OS event feed. Snapshot with `borrow_and_update()` first;
/// then every `changed().await` is a real transition.
pub fn subscribe() -> watch::Receiver<(u64, Option<OsEvent>)> {
    EVENTS.subscribe()
}

/// How long DictationIM needs after `DidExitDictationMode` before it will
/// accept a new start. A `startDictation:` toggled while the IM is still
/// tearing down the previous session is refused with "_startListening:
/// Dictation did not start because there is bottom line input" — and that
/// refusal *wedges* the global state machine: every later start is refused
/// the same way until DictationIM is killed or idle-exits. (Observed live on
/// macOS 26.2, 2026-06-05: a start 230 ms after the exit wedged it for the
/// rest of the app session.)
#[cfg(target_os = "macos")]
pub const EXIT_COOLDOWN: std::time::Duration = std::time::Duration::from_millis(1500);

/// Time still to wait before a new start is safe — `None` when no cool-down
/// applies (no real teardown seen yet, or the last one is old enough).
/// `start_dictation` sleeps this out instead of toggling into the teardown
/// window. Keyed off `LAST_TEARDOWN`, not `LAST_EVENT`: the liberal
/// no-session `Exited` notifications must not add phantom waits to every
/// start.
pub fn exit_cooldown_remaining() -> Option<std::time::Duration> {
    #[cfg(target_os = "macos")]
    {
        let at = (*LAST_TEARDOWN.lock().ok()?)?;
        EXIT_COOLDOWN.checked_sub(at.elapsed())
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// Last observed transition and how many seconds ago, for diagnostics.
pub fn last_event() -> Option<(&'static str, u64)> {
    #[cfg(target_os = "macos")]
    {
        LAST_EVENT.lock().ok()?.map(|(ev, at)| {
            let name = match ev {
                OsEvent::Listening => "listening",
                OsEvent::Exited => "exited",
            };
            (name, at.elapsed().as_secs())
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

#[cfg(target_os = "macos")]
fn record(event: OsEvent, app: &tauri::AppHandle) {
    use tauri::Emitter;

    let was_active = ACTIVE.swap(matches!(event, OsEvent::Listening), Ordering::SeqCst);
    if matches!(event, OsEvent::Listening) {
        EVER_LISTENED.store(true, Ordering::SeqCst);
    }
    // Only an exit that ends a session we saw listening is a real teardown
    // (and thus opens the start-refusal window the cool-down guards).
    if matches!(event, OsEvent::Exited) && was_active {
        if let Ok(mut teardown) = LAST_TEARDOWN.lock() {
            *teardown = Some(Instant::now());
        }
    }
    if let Ok(mut last) = LAST_EVENT.lock() {
        *last = Some((event, Instant::now()));
    }
    let seq = SEQ.fetch_add(1, Ordering::SeqCst) + 1;
    EVENTS.send_replace((seq, Some(event)));
    tracing::info!(?event, seq, "dictation: OS session transition");
    let _ = app.emit(
        match event {
            OsEvent::Listening => "dictation://listening",
            OsEvent::Exited => "dictation://ended",
        },
        (),
    );
}

/// Register the DictationIM observers. Called once from setup (main thread).
/// The observer tokens and blocks are intentionally leaked — they live for
/// the whole process.
#[cfg(target_os = "macos")]
pub fn init(app: &tauri::AppHandle) {
    use block2::RcBlock;
    use objc2_foundation::{NSDistributedNotificationCenter, NSNotification, NSString};
    use std::ptr::NonNull;

    let center = NSDistributedNotificationCenter::defaultCenter();
    let observe = |name: &str, event: OsEvent| {
        let app = app.clone();
        let block = RcBlock::new(move |_n: NonNull<NSNotification>| {
            record(event, &app);
        });
        let ns_name = NSString::from_str(name);
        // SAFETY: the block is sendable (captures only AppHandle + Copy data)
        // and the center copies it; `forget` keeps our reference alive too so
        // the observation can never dangle.
        let token = unsafe {
            center.addObserverForName_object_queue_usingBlock(Some(&ns_name), None, None, &block)
        };
        std::mem::forget(token);
        std::mem::forget(block);
    };
    observe(
        "DictationIMNotificationStartedListening",
        OsEvent::Listening,
    );
    observe(
        "DictationIMNotificationDidExitDictationMode",
        OsEvent::Exited,
    );
    tracing::info!("dictation: DictationIM session observers registered");

    init_fn_monitor(app);
}

/// Watch the Fn (🌐) key for push-to-talk. The Fn key never reaches the
/// WKWebView as a DOM event on macOS — it only surfaces as the `function`
/// modifier on AppKit `flagsChanged` events — so a *local* event monitor
/// (active-app only, exactly push-to-talk's scope) forwards transitions to
/// the frontend as `dictation://fn` with a bool payload. Events pass through
/// untouched; this only observes. Must run on the main thread.
///
/// Quirk note: arrow/F-row keys carry the `function` modifier on their own
/// keyDown events, but those aren't `flagsChanged` — only the physical Fn
/// key toggles this monitor, which is exactly what we want.
#[cfg(target_os = "macos")]
fn init_fn_monitor(app: &tauri::AppHandle) {
    use block2::RcBlock;
    use objc2_app_kit::{NSEvent, NSEventMask, NSEventModifierFlags};
    use std::ptr::NonNull;
    use tauri::Emitter;

    static FN_DOWN: AtomicBool = AtomicBool::new(false);

    let app = app.clone();
    let block = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
        // SAFETY: the monitor hands us a valid NSEvent for the callback's
        // duration; we only read its modifier flags.
        let down =
            unsafe { event.as_ref().modifierFlags() }.contains(NSEventModifierFlags::Function);
        if FN_DOWN.swap(down, Ordering::SeqCst) != down {
            tracing::debug!(down, "dictation: Fn key transition");
            let _ = app.emit("dictation://fn", down);
            // A dictate-anywhere session releases here when the user
            // cmd-tabbed INTO PortBay mid-hold — the global monitor only
            // fires while other apps are active, so without this bridge
            // the session would hang until the next Fn press outside.
            crate::dictation_anywhere::on_local_fn(&app, down);
        }
        event.as_ptr()
    });
    // SAFETY: block is 'static (captures only AppHandle) and returns the
    // event unmodified. The monitor token is leaked — it lives for the app.
    let token = unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::FlagsChanged, &block)
    };
    match token {
        Some(token) => {
            std::mem::forget(token);
            std::mem::forget(block);
            tracing::info!("dictation: Fn-key push-to-talk monitor installed");
        }
        None => {
            tracing::warn!("dictation: Fn-key monitor failed to install; push-to-talk inactive")
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn init(_app: &tauri::AppHandle) {}

/// The system "Dictation Enabled" pref (System Settings → Keyboard →
/// Dictation), read via the `com.apple.assistant.support` defaults domain.
/// `None` = key absent / unreadable (fresh machine, future macOS move) —
/// callers must not treat that as "disabled".
#[cfg(target_os = "macos")]
pub fn dictation_pref_enabled() -> Option<bool> {
    use objc2::AnyThread;
    use objc2_foundation::{NSNumber, NSString, NSUserDefaults};

    let suite = NSString::from_str("com.apple.assistant.support");
    // Reading another (non-container) domain is fine for a non-sandboxed app.
    let defaults = NSUserDefaults::initWithSuiteName(NSUserDefaults::alloc(), Some(&suite))?;
    let key = NSString::from_str("Dictation Enabled");
    let value = defaults.objectForKey(&key)?;
    let number = value.downcast::<NSNumber>().ok()?;
    Some(number.boolValue())
}

#[cfg(not(target_os = "macos"))]
pub fn dictation_pref_enabled() -> Option<bool> {
    None
}

// ---------------------------------------------------------------------------
// Diagnostics helpers (cold path — spawning a process or dlopen is fine here)
// ---------------------------------------------------------------------------

/// TCC authorization labels shared by AVCaptureDevice and SFSpeechRecognizer.
/// Note: **system dictation needs neither** — audio capture happens in
/// `corespeechd`, so these never prompt and stay "not_determined" on a healthy
/// setup. They're reported because a Denied state would still be worth seeing.
#[cfg(target_os = "macos")]
fn authorization_label(status: isize) -> &'static str {
    match status {
        0 => "not_determined",
        1 => "restricted",
        2 => "denied",
        3 => "authorized",
        _ => "unknown",
    }
}

/// dlopen a system framework so its ObjC classes become visible to the
/// runtime. Diagnostics-only: avoids hard-linking AVFoundation/Speech for two
/// status reads.
#[cfg(target_os = "macos")]
fn load_framework(path: &std::ffi::CStr) -> bool {
    unsafe extern "C" {
        fn dlopen(path: *const std::ffi::c_char, flag: i32) -> *mut std::ffi::c_void;
    }
    const RTLD_LAZY: i32 = 0x1;
    unsafe { !dlopen(path.as_ptr(), RTLD_LAZY).is_null() }
}

/// Microphone TCC status via `AVCaptureDevice` (informational — see
/// [`authorization_label`]).
#[cfg(target_os = "macos")]
pub fn mic_permission() -> String {
    use objc2::runtime::AnyClass;
    use objc2_foundation::NSString;

    if !load_framework(c"/System/Library/Frameworks/AVFoundation.framework/AVFoundation") {
        return "unknown".into();
    }
    let Some(cls) = AnyClass::get(c"AVCaptureDevice") else {
        return "unknown".into();
    };
    let media = NSString::from_str("soun"); // AVMediaTypeAudio
    let status: isize = unsafe { objc2::msg_send![cls, authorizationStatusForMediaType: &*media] };
    authorization_label(status).into()
}

/// Speech-recognition TCC status via `SFSpeechRecognizer` (informational —
/// system dictation does not use SFSpeechRecognizer).
#[cfg(target_os = "macos")]
pub fn speech_permission() -> String {
    use objc2::runtime::AnyClass;

    if !load_framework(c"/System/Library/Frameworks/Speech.framework/Speech") {
        return "unknown".into();
    }
    let Some(cls) = AnyClass::get(c"SFSpeechRecognizer") else {
        return "unknown".into();
    };
    let status: isize = unsafe { objc2::msg_send![cls, authorizationStatus] };
    authorization_label(status).into()
}

/// Whether a default audio-input device exists.
#[cfg(target_os = "macos")]
pub fn audio_input_available() -> Option<bool> {
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2_foundation::NSString;

    if !load_framework(c"/System/Library/Frameworks/AVFoundation.framework/AVFoundation") {
        return None;
    }
    let cls = AnyClass::get(c"AVCaptureDevice")?;
    let media = NSString::from_str("soun");
    let device: Option<Retained<AnyObject>> =
        unsafe { objc2::msg_send![cls, defaultDeviceWithMediaType: &*media] };
    Some(device.is_some())
}

/// The dictation language macOS would use (best-effort; the pref moved over
/// the years). Falls back to `en-US` when nothing is readable, which matches
/// the testing default.
#[cfg(target_os = "macos")]
pub async fn dictation_locale() -> String {
    let out = tokio::process::Command::new("/usr/bin/defaults")
        .args(["read", "com.apple.assistant.backedup", "Session Language"])
        .output()
        .await;
    if let Ok(out) = out {
        if out.status.success() {
            let locale = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !locale.is_empty() {
                return locale;
            }
        }
    }
    "en-US".into()
}

/// Whether the offline dictation model for `locale` is installed
/// (`Offline Dictation Status` in com.apple.assistant.support). `None` =
/// unreadable / key shape changed.
#[cfg(target_os = "macos")]
pub async fn offline_model_installed(locale: &str) -> Option<bool> {
    let home = std::env::var("HOME").ok()?;
    let path = format!("{home}/Library/Preferences/com.apple.assistant.support.plist");
    let out = tokio::process::Command::new("/usr/libexec/PlistBuddy")
        .args([
            "-c",
            &format!("Print :Offline Dictation Status:{locale}:Installed"),
            &path,
        ])
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    match String::from_utf8_lossy(&out.stdout).trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// Whether the DictationIM input-method process is currently running. Not
/// required pre-start (launchd spawns it on demand) — useful to spot a
/// wedged instance worth `killall DictationIM`.
#[cfg(target_os = "macos")]
pub async fn dictation_im_running() -> bool {
    tokio::process::Command::new("/usr/bin/pgrep")
        .args(["-x", "DictationIM"])
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}
