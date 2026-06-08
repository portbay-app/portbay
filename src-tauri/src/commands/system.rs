//! System-level commands — `doctor`, `tail_logs`.
//!
//! `doctor` mirrors the CLI's `cmd_doctor` JSON output shape so the GUI
//! and CLI report the same findings to the same support requests.

use tauri::{AppHandle, Emitter, State};

use crate::commands::dto::{DoctorFinding, DoctorReport, DoctorVerdict};
use crate::commands::projects::load_registry;
use crate::error::{AppError, AppResult};
use crate::hosts::HostsManager;
use crate::state::AppState;

#[tauri::command]
pub async fn doctor(state: State<'_, AppState>) -> AppResult<DoctorReport> {
    let mut findings = Vec::new();

    // Registry
    match load_registry(&state) {
        Ok(reg) => findings.push(DoctorFinding {
            check: "registry".into(),
            verdict: DoctorVerdict::Ok,
            detail: format!(
                "{} project(s), v{} schema, suffix .{}",
                reg.list_projects().len(),
                reg.version,
                reg.domain_suffix
            ),
        }),
        Err(e) => findings.push(DoctorFinding {
            check: "registry".into(),
            verdict: DoctorVerdict::Fail,
            detail: e.to_string(),
        }),
    }

    // PC daemon
    let pc_client = state
        .pc_client
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let pc_finding = match pc_client {
        None => DoctorFinding {
            check: "process-compose".into(),
            verdict: DoctorVerdict::Warn,
            detail: "not started yet".into(),
        },
        Some(c) => match c.live().await {
            Ok(true) => DoctorFinding {
                check: "process-compose".into(),
                verdict: DoctorVerdict::Ok,
                detail: "alive".into(),
            },
            Ok(false) => DoctorFinding {
                check: "process-compose".into(),
                verdict: DoctorVerdict::Warn,
                detail: "not reachable".into(),
            },
            Err(e) => DoctorFinding {
                check: "process-compose".into(),
                verdict: DoctorVerdict::Warn,
                detail: e.to_string(),
            },
        },
    };
    findings.push(pc_finding);

    // Caddy daemon
    let caddy_client = state
        .caddy_client
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let caddy_finding = match caddy_client {
        None => DoctorFinding {
            check: "caddy".into(),
            verdict: DoctorVerdict::Warn,
            detail: "not started yet".into(),
        },
        Some(c) => match c.is_alive().await {
            Ok(true) => DoctorFinding {
                check: "caddy".into(),
                verdict: DoctorVerdict::Ok,
                detail: "alive".into(),
            },
            Ok(false) => DoctorFinding {
                check: "caddy".into(),
                verdict: DoctorVerdict::Warn,
                detail: "not reachable".into(),
            },
            Err(e) => DoctorFinding {
                check: "caddy".into(),
                verdict: DoctorVerdict::Warn,
                detail: e.to_string(),
            },
        },
    };
    findings.push(caddy_finding);

    // Tools on PATH
    for tool in ["mkcert", "caddy", "process-compose"] {
        match which::which(tool) {
            Ok(p) => findings.push(DoctorFinding {
                check: format!("tool: {tool}"),
                verdict: DoctorVerdict::Ok,
                detail: p.display().to_string(),
            }),
            Err(_) => findings.push(DoctorFinding {
                check: format!("tool: {tool}"),
                verdict: DoctorVerdict::Warn,
                detail: "not found on PATH (bundled .app uses its sidecar — this only matters for CLI standalone use)".into(),
            }),
        }
    }

    // /etc/hosts reconcile state
    match (HostsManager::system().list_managed(), load_registry(&state)) {
        (Ok(entries), Ok(reg)) => {
            use std::collections::HashSet;
            let expected: HashSet<String> = reg
                .list_projects()
                .iter()
                .map(|p| p.hostname.clone())
                .collect();
            let present: HashSet<String> = entries.iter().map(|e| e.hostname.clone()).collect();
            let missing = expected.difference(&present).count();
            let orphan = present.difference(&expected).count();
            let verdict = if missing == 0 && orphan == 0 {
                DoctorVerdict::Ok
            } else {
                DoctorVerdict::Warn
            };
            let detail = if missing == 0 && orphan == 0 {
                format!("{} entries, all match registry", entries.len())
            } else {
                format!(
                    "{} entries (missing: {missing}, orphan: {orphan}). Run `sudo portbay hosts reconcile` to fix.",
                    entries.len()
                )
            };
            findings.push(DoctorFinding {
                check: "/etc/hosts".into(),
                verdict,
                detail,
            });
        }
        (Err(e), _) => findings.push(DoctorFinding {
            check: "/etc/hosts".into(),
            verdict: DoctorVerdict::Warn,
            detail: e.to_string(),
        }),
        (_, Err(_)) => {
            // Registry load already errored above; nothing useful to add here.
        }
    }

    Ok(DoctorReport { findings })
}

/// `read_dotenv(path)` — read a user-picked `.env`-style file and
/// return its `KEY=value` pairs as a vector preserving file order.
/// Comments (`#`) and blank lines are skipped; surrounding quotes
/// on the value are stripped when matched on both ends.
///
/// We do the parse on the Rust side so the wire shape is already
/// clean — the frontend just merges the result into its row state.
/// Files larger than 256 KB are rejected to avoid hostile inputs.
#[tauri::command]
pub async fn read_dotenv(path: String) -> AppResult<Vec<(String, String)>> {
    use std::fs;

    const MAX_BYTES: u64 = 256 * 1024;
    let meta =
        fs::metadata(&path).map_err(|e| AppError::BadInput(format!("can't open {path}: {e}")))?;
    if !meta.is_file() {
        return Err(AppError::BadInput(format!("not a regular file: {path}")));
    }
    if meta.len() > MAX_BYTES {
        return Err(AppError::BadInput(format!(
            ".env file is too large ({} bytes); paste it instead",
            meta.len()
        )));
    }
    let text = fs::read_to_string(&path)
        .map_err(|e| AppError::BadInput(format!("can't read {path}: {e}")))?;
    Ok(parse_dotenv(&text))
}

/// Parser for [`read_dotenv`]. Exposed for unit tests.
pub(crate) fn parse_dotenv(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Strip an optional `export ` prefix to be friendly to shell-
        // sourced env files.
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some(eq) = line.find('=') else {
            continue;
        };
        let key = line[..eq].trim();
        if key.is_empty() {
            continue;
        }
        let mut value = line[eq + 1..].trim().to_string();
        if (value.starts_with('"') && value.ends_with('"') && value.len() >= 2)
            || (value.starts_with('\'') && value.ends_with('\'') && value.len() >= 2)
        {
            value = value[1..value.len() - 1].to_string();
        }
        out.push((key.to_string(), value));
    }
    out
}

/// `quit_app` — explicit "Quit PortBay" from the user menu.
///
/// Mirrors the tray's quit path (`app.exit(0)`) so window-close-to-tray
/// stays separate from a true exit. The Rust window-close handler is
/// responsible for the menu-bar-hint toast, not this command — calling
/// `exit(0)` bypasses that hint, which is the right behaviour for an
/// explicit quit from the user menu.
#[tauri::command]
pub async fn quit_app(app: AppHandle) -> AppResult<()> {
    app.exit(0);
    Ok(())
}

/// `open_main_window` — reveal PortBay's primary window from secondary UI
/// surfaces such as the tray panel. When `path` is supplied (e.g. the tray
/// popover's nav grid), route the main window there via the same
/// `portbay://nav` channel the tray menu uses (handled in +layout.svelte).
#[tauri::command]
pub async fn open_main_window(app: AppHandle, path: Option<String>) -> AppResult<()> {
    crate::tray::show_main_window(&app);
    if let Some(route) = path {
        let _ = app.emit("portbay://nav", route);
    }
    Ok(())
}

/// Outcome of a dictation attempt, so the frontend can react precisely
/// (start the session, explain why it couldn't, or explain it's macOS-only)
/// instead of guessing from an opaque error string.
#[derive(Debug, Clone, Copy, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DictationOutcome {
    /// DictationIM confirmed it is listening (or a session was already live).
    Started,
    /// The action was sent and accepted, but no OS dictation session
    /// materialized within the confirmation window — even after the
    /// stale-mode retry. The global dictation state machine is likely
    /// wedged (see `crate::dictation_session`); the frontend should say so.
    NotEngaged,
    /// Dictation is switched off in System Settings, so macOS is showing its
    /// own "Do you want to enable Dictation?" dialog instead of starting a
    /// session. The frontend should close the recording UI quietly — the OS
    /// dialog is the message.
    OsDialog,
    /// Nothing in the responder chain handled `startDictation:`. Unexpected
    /// (NSApplication itself implements it on every supported macOS); would
    /// mean Apple removed the selector in a future release.
    Unavailable,
    /// Not macOS — no system dictation to trigger.
    Unsupported,
}

/// Send the `startDictation:` / `stopDictation:` action down the responder
/// chain — exactly what the auto-inserted Edit ▸ "Start Dictation…" menu item
/// (which morphs to "Stop Dictation…" while live) does. Must run on the main
/// thread (AppKit). Returns whether anything handled the action.
///
/// Both selectors are undocumented but implemented by `NSApplication` since
/// at least macOS 10.12 (kitty ships the same forward; agent-deck uses the
/// same `sendAction`). The `respondsToSelector` guard degrades to a clean
/// `Unavailable` instead of a crash if a future macOS drops them.
#[cfg(target_os = "macos")]
fn send_dictation_action(start: bool) -> bool {
    use objc2::runtime::NSObjectProtocol;
    use objc2::{sel, MainThreadMarker};
    use objc2_app_kit::NSApplication;

    let Some(mtm) = MainThreadMarker::new() else {
        tracing::warn!("dictation: not on main thread; refusing to touch AppKit");
        return false;
    };
    let app = NSApplication::sharedApplication(mtm);
    let action = if start {
        sel!(startDictation:)
    } else {
        sel!(stopDictation:)
    };
    if !app.respondsToSelector(action) {
        tracing::warn!("dictation: NSApplication no longer responds to start/stopDictation:");
        return false;
    }
    // SAFETY: both are standard zero-result action methods taking one
    // (nullable) sender argument; nil target routes them down the responder
    // chain, terminating at NSApp which implements them.
    unsafe { app.sendAction_to_from(action, None, None) }
}

/// Send `startDictation:` / `stopDictation:` on the main thread and report
/// whether the responder chain accepted it.
#[cfg(target_os = "macos")]
async fn dispatch_dictation_action(app: &AppHandle, start: bool) -> AppResult<bool> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let _ = tx.send(send_dictation_action(start));
    })
    .map_err(|e| AppError::Internal(format!("dictation main-thread hop failed: {e}")))?;
    rx.await
        .map_err(|_| AppError::Internal("dictation result never arrived".into()))
}

/// How long to wait for DictationIM to confirm a transition. Generous: a cold
/// start spawns DictationIM and preheats `corespeechd` (~2 s observed); only a
/// genuinely failed start pays the full window.
#[cfg(target_os = "macos")]
const DICTATION_CONFIRM: std::time::Duration = std::time::Duration::from_secs(4);

/// Confirmation window for the FIRST start of an app run: DictationIM +
/// `corespeechd` cold spawn (and possibly an offline-model page-in) can
/// overrun the warm window, and the old behaviour — give up, flip the UI
/// off — read as "the mic only works on the second click". The frontend
/// shows an arming state during the wait, so the longer window is honest,
/// not a frozen recording UI.
#[cfg(target_os = "macos")]
const DICTATION_CONFIRM_COLD: std::time::Duration = std::time::Duration::from_secs(8);

/// The confirmation window for this attempt — cold until a session has been
/// confirmed once this run.
#[cfg(target_os = "macos")]
fn dictation_confirm_window() -> std::time::Duration {
    if crate::dictation_session::ever_listened() {
        DICTATION_CONFIRM
    } else {
        DICTATION_CONFIRM_COLD
    }
}

/// How long a `DidExitDictationMode` may precede the same toggle's
/// `StartedListening`. A single `startDictation:` routinely posts a
/// stale-mode exit and THEN the fresh session's listening a beat later
/// (observed 214 ms apart on macOS 26.2, 2026-06-06) — judging the attempt on
/// the first event alone misreads that as a failure, and a hasty second
/// toggle would kill the session that's about to go live.
#[cfg(target_os = "macos")]
const EXIT_TO_LISTENING_GRACE: std::time::Duration = std::time::Duration::from_millis(1500);

/// Minimum spacing between DictationIM rescues. A freshly respawned
/// DictationIM needs a beat to initialize its speech recognizer ("Scheduling
/// idle termination because there is no speech recognizer" in its log) — a
/// second `killall` landing inside that window murders the recovering
/// instance and re-wedges the state machine. Observed live 2026-06-06: a
/// burst of queued mic clicks each ran its own wedge→killall cycle and the
/// storm fed itself for minutes. Within this window a failed start returns
/// `NotEngaged` instead of rescuing again.
#[cfg(target_os = "macos")]
const RESCUE_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(10);

/// When the last `killall DictationIM` rescue ran, for `RESCUE_COOLDOWN`.
#[cfg(target_os = "macos")]
static LAST_RESCUE: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);

/// Wait until `deadline` for DictationIM to confirm listening, tolerating
/// `Exited` churn in between. Returns whether listening was confirmed.
#[cfg(target_os = "macos")]
async fn wait_for_listening(
    events: &mut tokio::sync::watch::Receiver<(u64, Option<crate::dictation_session::OsEvent>)>,
    deadline: tokio::time::Instant,
) -> bool {
    loop {
        match tokio::time::timeout_at(deadline, events.changed()).await {
            Ok(Ok(())) => {
                if matches!(
                    *events.borrow_and_update(),
                    (_, Some(crate::dictation_session::OsEvent::Listening))
                ) {
                    return true;
                }
                // Exited churn — keep waiting for the follow-up.
            }
            _ => return false,
        }
    }
}

/// Trigger the OS's built-in dictation so speech is typed straight into the
/// focused field — we never run our own recognizer. Shared by the task board
/// and the SSH agent composer (ungated, no cross-feature dependency). The
/// frontend focuses the target field first, so the WKWebView is first
/// responder and dictation inserts into it via NSTextInputClient (the same
/// machinery Safari uses).
///
/// Mechanism: send the `startDictation:` action to the responder chain — the
/// exact code path of the Edit ▸ "Start Dictation…" menu item. Unlike the
/// keystroke-synthesis approaches this replaces, it
///   • is independent of the user's dictation shortcut (Fn×2, Ctrl×2, custom,
///     or none — synthetic Fn presses were ignored anyway because the
///     double-tap detector watches the keyboard driver, not the event stream),
///   • needs no Accessibility / Automation grants and no System Events, and
///   • when Dictation is disabled, makes macOS itself show its "Do you want
///     to enable Dictation?" onboarding dialog instead of failing silently.
///
/// CRITICALLY, the action is a **toggle on a global state machine**, not a
/// start verb — see `crate::dictation_session` for the full failure story.
/// So this command confirms the start against DictationIM's notifications:
///   • `StartedListening` within the window → `Started`.
///   • `DidExitDictationMode` → our toggle landed on a stale session left
///     engaged by an earlier run (macOS ends audio without exiting dictation
///     mode); send the action once more — the second toggle starts fresh.
///   • Nothing at all → `NotEngaged`, so the UI can say something true
///     instead of showing a recording timer over a dead mic.
///
/// Sounds: the session-OPEN moment plays only macOS's own pop (unsuppressible,
/// and adding ours there just doubled it — tried and reverted). Our own cue
/// plays at the LISTENING confirmation instead (`play_dictation_start_cue`) —
/// seconds later, when the mic is actually hot — so eyes-free users know when
/// to start talking. The off-side cue lives in `stop_dictation`, which avoids
/// the OS pop instead of stacking on it.
///
/// Permissions: none required at the app level — system dictation captures
/// audio in `corespeechd`, so no mic TCC prompt is attributed to PortBay.
/// The Info.plist usage strings + audio-input entitlement stay for any future
/// in-app capture path.
#[tauri::command]
pub async fn start_dictation(app: AppHandle) -> AppResult<DictationOutcome> {
    #[cfg(target_os = "macos")]
    {
        // Where does start latency go? The spans logged below split a slow
        // start into its parts: lock wait (a stop's mute→restore window),
        // teardown cool-down (logged where it sleeps), and dispatch→listening
        // (pure OS time — the only part Notes/Fn-Fn pays too). The frontend
        // logs the end-to-end click→live figure via `dictation_trace`.
        let t0 = std::time::Instant::now();

        // Serialized with `stop_dictation`: its mute→stop→restore window must
        // fully close before a new session starts, or the new session's OS
        // pop would land inside the muted span.
        let _transition = DICTATION_TRANSITION.lock().await;
        let lock_ms = t0.elapsed().as_millis() as u64;
        if lock_ms > 50 {
            tracing::info!(lock_ms, "dictation start: waited on a stop in flight");
        }

        if crate::dictation_session::os_session_active() {
            // A session is already live (e.g. a quick stop→start race the
            // user won); toggling again would kill it. Still cue — this path
            // emits no `listening` event, so the response is what flips the
            // UI live, and the audible "speak now" must come with it.
            tracing::info!("dictation start: OS session already live; treating as started");
            play_dictation_start_cue();
            return Ok(DictationOutcome::Started);
        }

        // A start toggled while DictationIM is still tearing down the
        // previous session is refused ("bottom line input") — and that
        // refusal wedges the global state machine until the IM dies (see
        // `dictation_session::EXIT_COOLDOWN`). The user clicking the mic
        // right after a session ended is exactly this race: wait the
        // teardown out instead of toggling into it.
        if let Some(wait) = crate::dictation_session::exit_cooldown_remaining() {
            tracing::info!(
                wait_ms = wait.as_millis() as u64,
                "dictation start: waiting out the previous session's teardown"
            );
            tokio::time::sleep(wait).await;
        }

        // Snapshot the event feed BEFORE sending, so the confirmation loop
        // only sees transitions caused by this attempt.
        let mut events = crate::dictation_session::subscribe();
        events.borrow_and_update();

        let dispatched_at = std::time::Instant::now();
        if !dispatch_dictation_action(&app, true).await? {
            tracing::warn!("dictation start: responder chain dropped startDictation:");
            return Ok(DictationOutcome::Unavailable);
        }

        // Dictation switched off in System Settings: the action we just sent
        // makes macOS show its own enable dialog; no session will start.
        if crate::dictation_session::dictation_pref_enabled() == Some(false) {
            tracing::info!(
                "dictation start: Dictation disabled in System Settings; macOS shows its enable dialog"
            );
            return Ok(DictationOutcome::OsDialog);
        }

        // The loop below returns on a confirmed start; breaking out of it
        // means the start failed in a way only a DictationIM reset fixes:
        //   • refusal wedge — every toggle lands as an exit (DictationIM
        //     rejects the start outright with "bottom line input" and posts
        //     `DidExitDictationMode` instead of listening), or
        //   • silent wedge — the toggle produces NO event at all within the
        //     window. Observed live 2026-06-06: a burst of `Exited`
        //     notifications (the IM looping), then total silence on the next
        //     start — the wedge documented in `dictation_session` ("every
        //     later start is refused the same way until DictationIM is
        //     killed"), in its quiet form. A session legitimately warming up
        //     (cold spawn, asset download) posts `StartedListening` well
        //     inside the 8 s cold window, so silence past it means wedged,
        //     not slow.
        let mut retoggled = false;
        let confirm_window = dictation_confirm_window();
        let deadline = tokio::time::Instant::now() + confirm_window;
        loop {
            match tokio::time::timeout_at(deadline, events.changed()).await {
                Ok(Ok(())) => {
                    if matches!(
                        *events.borrow_and_update(),
                        (_, Some(crate::dictation_session::OsEvent::Listening))
                    ) {
                        tracing::info!(
                            total_ms = t0.elapsed().as_millis() as u64,
                            os_ms = dispatched_at.elapsed().as_millis() as u64,
                            "dictation start: confirmed listening"
                        );
                        play_dictation_start_cue();
                        return Ok(DictationOutcome::Started);
                    }
                    // Exited: our toggle closed a stale dictation mode. The
                    // SAME toggle often follows with the fresh session's
                    // listening a beat later — grace-wait for it before
                    // concluding a second toggle is needed (a hasty re-toggle
                    // would kill the session that's about to go live).
                    let grace = std::cmp::min(
                        deadline,
                        tokio::time::Instant::now() + EXIT_TO_LISTENING_GRACE,
                    );
                    if wait_for_listening(&mut events, grace).await {
                        tracing::info!(
                            total_ms = t0.elapsed().as_millis() as u64,
                            os_ms = dispatched_at.elapsed().as_millis() as u64,
                            "dictation start: confirmed listening after stale-mode exit"
                        );
                        play_dictation_start_cue();
                        return Ok(DictationOutcome::Started);
                    }
                    if !retoggled {
                        retoggled = true;
                        tracing::warn!("dictation start: toggled a stale session off; retrying");
                        if !dispatch_dictation_action(&app, true).await? {
                            return Ok(DictationOutcome::Unavailable);
                        }
                    } else {
                        tracing::warn!("dictation start: retry also failed to engage a session");
                        break;
                    }
                }
                _ => {
                    tracing::warn!(
                        timeout_secs = confirm_window.as_secs(),
                        "dictation start: no OS dictation event within the window; treating as wedged"
                    );
                    break;
                }
            }
        }

        // Rescue: DictationIM is refusing every start (loudly or silently).
        // Killing it resets the global dictation state machine (launchd
        // respawns it on demand) — this automates the `killall DictationIM`
        // the not-engaged toast used to prescribe — then one more toggle
        // starts clean. Rate-limited: rescuing again while the previous
        // respawn is still initializing only re-wedges it (see
        // `RESCUE_COOLDOWN`).
        {
            {
                let mut last = LAST_RESCUE.lock().expect("LAST_RESCUE poisoned");
                if let Some(at) = *last {
                    if at.elapsed() < RESCUE_COOLDOWN {
                        tracing::warn!(
                            since_ms = at.elapsed().as_millis() as u64,
                            "dictation start: skipping DictationIM reset (one just ran); not engaged"
                        );
                        return Ok(DictationOutcome::NotEngaged);
                    }
                }
                *last = Some(std::time::Instant::now());
            }
            tracing::warn!("dictation start: wedge suspected; resetting DictationIM and retrying");
            // SIGKILL, not SIGTERM: a wedged DictationIM has been observed
            // surviving a plain killall (still the same pid minutes later) —
            // it ignores TERM while stuck. KILL can't be ignored, and the IM
            // holds no state worth a graceful exit (launchd respawns it on
            // demand).
            let status = tokio::process::Command::new("/usr/bin/killall")
                .args(["-9", "DictationIM"])
                .status()
                .await;
            tracing::info!(?status, "dictation start: killall -9 DictationIM");
            // Let the respawned instance finish initializing before the
            // re-toggle. 300 ms was too short: the fresh DictationIM spends
            // ~1 s bringing up its speech recognizer, and a toggle landing
            // during that window is refused and re-wedges it (observed live
            // 2026-06-06). Also snapshots the event feed after, not before,
            // so the dying instance's final notification isn't misread.
            tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
            events.borrow_and_update();
            if !dispatch_dictation_action(&app, true).await? {
                return Ok(DictationOutcome::Unavailable);
            }
            // Churn-tolerant wait: the fresh instance posts a stale-mode
            // `Exited` BEFORE its `Listening` (observed 214 ms apart) —
            // judging on the first event alone declared this recovery a
            // failure while the session went live moments later, leaving a
            // hot mic the UI had already given up on.
            let deadline = tokio::time::Instant::now() + confirm_window;
            if wait_for_listening(&mut events, deadline).await {
                tracing::info!(
                    total_ms = t0.elapsed().as_millis() as u64,
                    "dictation start: recovered after DictationIM reset"
                );
                play_dictation_start_cue();
                return Ok(DictationOutcome::Started);
            }
            tracing::warn!("dictation start: DictationIM reset did not recover a session");
        }
        Ok(DictationOutcome::NotEngaged)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Ok(DictationOutcome::Unsupported)
    }
}

/// Serializes dictation session transitions — see `start_dictation` /
/// `stop_dictation`. Stop now spans an async mute→stop→restore window, and
/// the frontend fires stop-then-start when switching fields mid-session; the
/// lock keeps that FIFO and keeps a restart's pop out of the muted span.
#[cfg(target_os = "macos")]
static DICTATION_TRANSITION: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Play the first existing sound of `candidates` via `afplay`. Missing files
/// → silent no-op, never an error. Fire-and-forget; a detached thread reaps
/// the child so no zombies accumulate.
#[cfg(target_os = "macos")]
fn play_dictation_cue(candidates: &[&str]) {
    let Some(path) = candidates.iter().find(|p| std::path::Path::new(p).exists()) else {
        tracing::debug!("dictation cue: no system sound found; skipping");
        return;
    };
    match std::process::Command::new("/usr/bin/afplay")
        .arg(path)
        .spawn()
    {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => tracing::debug!("dictation cue failed to play: {e}"),
    }
}

/// The "speak now" cue, played when DictationIM CONFIRMS listening — the
/// audible counterpart of the honest mic UI (stop button + clock at
/// `dictation://listening`). The OS pop fires at session OPEN, seconds
/// before the mic is hot; eyes-free users had no signal for when speech
/// actually starts landing. The pop itself stays untouched (start side is
/// unmuted by design — see `stop_dictation`'s mute choreography); this rides
/// the regular output channel like the off cue.
#[cfg(target_os = "macos")]
fn play_dictation_start_cue() {
    const START: &[&str] = &[
        "/System/Library/PrivateFrameworks/AssistantServices.framework/Versions/A/Resources/dt-begin.caf",
        "/System/Library/Components/CoreAudio.component/Contents/SharedSupport/SystemSounds/siri/jbl_begin_short.caf",
    ];
    play_dictation_cue(START);
}

/// Play the app's dictation *off* cue (AssistantServices `dt-confirm`, older
/// CoreAudio Siri set as fallback).
#[cfg(target_os = "macos")]
fn play_dictation_end_cue() {
    const END: &[&str] = &[
        "/System/Library/PrivateFrameworks/AssistantServices.framework/Versions/A/Resources/dt-confirm.caf",
        "/System/Library/Components/CoreAudio.component/Contents/SharedSupport/SystemSounds/siri/jbl_confirm.caf",
    ];
    play_dictation_cue(END);
}

/// Read the system alert (UI sound-effects) volume and set it to 0, returning
/// the previous value for restore — or None if it couldn't be read (no mute
/// happens then). Plain `set volume` is a StandardAdditions command: no
/// Automation/Accessibility prompt.
#[cfg(target_os = "macos")]
async fn mute_alert_volume() -> Option<u8> {
    let out = tokio::process::Command::new("/usr/bin/osascript")
        .args([
            "-e",
            "set s to alert volume of (get volume settings)",
            "-e",
            "set volume alert volume 0",
            "-e",
            "s",
        ])
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<u8>()
        .ok()
}

#[cfg(target_os = "macos")]
async fn restore_alert_volume(volume: u8) {
    let _ = tokio::process::Command::new("/usr/bin/osascript")
        .args(["-e", &format!("set volume alert volume {volume}")])
        .status()
        .await;
}

/// End a live dictation session. The frontend calls this whenever its
/// recording UI exits (stop click, focus moved off the field, editor closed);
/// with no session live it returns without touching the OS.
///
/// That guard is load-bearing, not an optimization: `stopDictation:` is a
/// toggle on the global dictation state machine (see
/// `crate::dictation_session`). The old "harmless best-effort stop on every
/// exit" fired against already-ended sessions and drove the machine out of
/// phase — the main reason the mic button appeared to never work.
///
/// Sound choreography: macOS replays its session pop on *every* end — the
/// explicit `stopDictation:` and even a focus-loss end both pop (verified by
/// ear; a resign-first-responder approach was tried and reverted). The pop is
/// a system UI sound on the *alert* channel, though, while `afplay` uses the
/// regular output — so we mute the alert channel for the moment of the stop,
/// play the proper off chime (`dt-confirm`) ourselves, and restore the alert
/// volume after the (silenced) pop has passed. Start keeps its OS pop: the
/// mute window only spans the stop, and the transition lock keeps a quick
/// restart from landing inside it.
#[tauri::command]
pub async fn stop_dictation(app: AppHandle) -> AppResult<()> {
    #[cfg(target_os = "macos")]
    {
        let _transition = DICTATION_TRANSITION.lock().await;

        if !crate::dictation_session::os_session_active() {
            // Nothing live (macOS already ended it, or the start never
            // engaged) — a toggle here would START a session or wedge the
            // state machine further.
            tracing::debug!("dictation stop: no live OS session; skipping toggle");
            return Ok(());
        }

        let previous = mute_alert_volume().await;

        // Watch for the exit confirmation so the log tells the truth about
        // whether macOS actually closed the session.
        let mut events = crate::dictation_session::subscribe();
        events.borrow_and_update();

        app.run_on_main_thread(|| {
            let _ = send_dictation_action(false);
        })
        .map_err(|e| AppError::Internal(format!("dictation main-thread hop failed: {e}")))?;

        // The app-chosen off sound — regular output channel, so it plays
        // through the mute.
        play_dictation_end_cue();

        match tokio::time::timeout(std::time::Duration::from_millis(1500), events.changed()).await
        {
            Ok(Ok(())) => {
                let (_, event) = *events.borrow_and_update();
                tracing::info!(?event, "dictation stop: OS confirmed transition");
            }
            _ => tracing::warn!(
                "dictation stop: no exit confirmation from DictationIM; state may resync on next start"
            ),
        }

        if let Some(volume) = previous {
            // Long enough for the muted pop to play out, short enough that a
            // deliberate restart (which waits on the lock) isn't held up.
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
            restore_alert_volume(volume).await;
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
    }
    Ok(())
}

/// Everything the dictation pipeline can self-report, for debugging "the mic
/// does nothing" without a Console.app session. Invoke from devtools:
/// `await window.__TAURI__.core.invoke("dictation_diagnostics")`.
///
/// Permission fields are informational: system dictation captures audio in
/// `corespeechd`, so `not_determined` is the healthy steady state for both —
/// only an explicit `denied`/`restricted` is worth flagging.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictationDiagnostics {
    /// macOS — the only platform with system dictation.
    pub platform_supported: bool,
    /// NSApplication still implements `startDictation:`/`stopDictation:`.
    pub trigger_available: bool,
    /// System Settings → Keyboard → Dictation. `None` = pref unreadable.
    pub dictation_enabled: Option<bool>,
    /// The dictation language macOS reports (defaults to en-US).
    pub locale: String,
    /// Offline model installed for `locale`. `None` = unreadable.
    pub offline_model_installed: Option<bool>,
    /// Mic TCC status via AVCaptureDevice (informational — see above).
    pub mic_permission: String,
    /// Speech-recognition TCC status via SFSpeechRecognizer (informational).
    pub speech_recognition_permission: String,
    /// A default audio-input device exists. `None` = couldn't determine.
    pub audio_input_available: Option<bool>,
    /// DictationIM input-method process is running (spawned on demand; only
    /// interesting when a session is supposedly live but nothing types).
    pub dictation_im_running: bool,
    /// Our mirror of the OS session (StartedListening seen, no exit since).
    pub os_session_active: bool,
    /// Last observed OS transition: `("listening"|"exited", seconds ago)`.
    pub last_session_event: Option<(String, u64)>,
}

/// Collect [`DictationDiagnostics`]. Read-only: nothing here prompts, starts,
/// or stops anything.
#[tauri::command]
pub async fn dictation_diagnostics(app: AppHandle) -> AppResult<DictationDiagnostics> {
    #[cfg(target_os = "macos")]
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        app.run_on_main_thread(move || {
            use objc2::runtime::NSObjectProtocol;
            use objc2::{sel, MainThreadMarker};
            use objc2_app_kit::NSApplication;
            let available = MainThreadMarker::new().is_some_and(|mtm| {
                let app = NSApplication::sharedApplication(mtm);
                app.respondsToSelector(sel!(startDictation:))
                    && app.respondsToSelector(sel!(stopDictation:))
            });
            let _ = tx.send(available);
        })
        .map_err(|e| AppError::Internal(format!("dictation main-thread hop failed: {e}")))?;
        let trigger_available = rx.await.unwrap_or(false);

        let locale = crate::dictation_session::dictation_locale().await;
        let diagnostics = DictationDiagnostics {
            platform_supported: true,
            trigger_available,
            dictation_enabled: crate::dictation_session::dictation_pref_enabled(),
            offline_model_installed: crate::dictation_session::offline_model_installed(&locale)
                .await,
            locale,
            mic_permission: crate::dictation_session::mic_permission(),
            speech_recognition_permission: crate::dictation_session::speech_permission(),
            audio_input_available: crate::dictation_session::audio_input_available(),
            dictation_im_running: crate::dictation_session::dictation_im_running().await,
            os_session_active: crate::dictation_session::os_session_active(),
            last_session_event: crate::dictation_session::last_event()
                .map(|(name, secs)| (name.to_string(), secs)),
        };
        tracing::info!(?diagnostics, "dictation diagnostics collected");
        Ok(diagnostics)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Ok(DictationDiagnostics {
            platform_supported: false,
            trigger_available: false,
            dictation_enabled: None,
            locale: "en-US".into(),
            offline_model_installed: None,
            mic_permission: "unknown".into(),
            speech_recognition_permission: "unknown".into(),
            audio_input_available: None,
            dictation_im_running: false,
            os_session_active: false,
            last_session_event: None,
        })
    }
}

/// `tail_logs(id, limit, offset)` — snapshot of a project's recent log output.
///
/// Reads the tail of the per-project log file PC writes at
/// `<logs_dir>/<id>.log` — the same canonical file `subscribe_logs` streams.
/// We deliberately do *not* hit PC's REST `/process/logs` endpoint: it returns
/// HTTP 400 (`process <id> doesn't exist`) for any process not currently loaded
/// in the daemon — i.e. every stopped project — and even for running ones only
/// returns whatever is still in PC's in-memory ring, which is frequently empty.
/// The on-disk file is the durable record, so reading it shows history for
/// stopped projects and never errors.
///
/// For live streaming, see `subscribe_logs` (Channel<T> follow mode).
///
/// Deliberately a **synchronous** command. Tauri runs sync commands on the
/// blocking thread pool, whereas `async` commands share the async-worker pool —
/// and that worker pool gets congested by the reconciler's synchronous work
/// (mkcert, `ps` sweeps, the PC stop grace-sleep) running on it. As an `async`
/// command this snapshot would queue behind that congestion and the log viewer
/// would sit blank for many seconds before "old logs" appeared, while the HTTP
/// inspector — whose `recent_requests` backfill is sync — stayed instant. Sync
/// puts the file read on its own pool so the snapshot returns immediately. The
/// read itself is bounded and cheap, so it never starves that pool.
#[tauri::command]
pub fn tail_logs(
    state: State<'_, AppState>,
    id: String,
    #[allow(non_snake_case)] limit: Option<u32>,
    // Accepted for API compatibility. File-tail always returns the most recent
    // lines, so an offset into PC's in-memory buffer no longer applies.
    #[allow(non_snake_case)] offset: Option<u64>,
) -> AppResult<Vec<String>> {
    let _ = offset;
    let limit = limit.unwrap_or(1000).max(1) as usize;
    let path = state.logs_dir.join(format!("{id}.log"));
    Ok(tail_file_lines(&path, limit))
}

/// Read at most this many bytes from the end of a log file for the snapshot.
/// Per-project logs don't rotate, so without this a long-lived project's log
/// would grow unbounded and a full-file scan would re-introduce the open lag.
/// Mirrors the request inspector's `TAIL_READ_BYTES` (which is why it's instant).
const TAIL_READ_BYTES: u64 = 512 * 1024;

/// Return the last `limit` lines of a log file, oldest-first.
///
/// Robust to the realities of live log files:
/// - missing file (project never started) → empty vec, not an error;
/// - invalid UTF-8 (stray bytes mid-stream) → decoded lossily so a single
///   bad byte never blanks the viewer;
/// - bounded read: for a large file we `seek` to the last [`TAIL_READ_BYTES`]
///   and drop the first (partial) line, so the snapshot stays O(tail) — instant
///   even at hundreds of MB — instead of scanning the whole file;
/// - bounded memory: a ring buffer keeps at most `limit` lines.
fn tail_file_lines(path: &std::path::Path, limit: usize) -> Vec<String> {
    use std::collections::VecDeque;
    use std::io::{BufRead, BufReader, Seek, SeekFrom};

    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    // For a large file, jump to the tail window; the first line we then read is
    // almost certainly a fragment, so skip it once we're past the start.
    let mut skip_partial = false;
    if let Ok(meta) = file.metadata() {
        if meta.len() > TAIL_READ_BYTES {
            let start = meta.len() - TAIL_READ_BYTES;
            if file.seek(SeekFrom::Start(start)).is_ok() {
                skip_partial = true;
            }
        }
    }
    let mut reader = BufReader::new(file);
    if skip_partial {
        let mut discard: Vec<u8> = Vec::new();
        let _ = reader.read_until(b'\n', &mut discard);
    }
    let mut ring: VecDeque<String> = VecDeque::with_capacity(limit.min(4096) + 1);
    let mut buf: Vec<u8> = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) => break,
            Ok(_) => {
                while matches!(buf.last(), Some(b'\n' | b'\r')) {
                    buf.pop();
                }
                if ring.len() == limit {
                    ring.pop_front();
                }
                ring.push_back(String::from_utf8_lossy(&buf).into_owned());
            }
            Err(_) => break,
        }
    }
    ring.into()
}

#[cfg(test)]
mod tests {
    use super::{parse_dotenv, tail_file_lines, TAIL_READ_BYTES};

    #[test]
    fn tail_returns_last_lines_small_file() {
        let dir = std::env::temp_dir().join(format!("portbay_tail_small_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("s.log");
        std::fs::write(&path, "a\nb\nc\nd\ne\n").unwrap();
        assert_eq!(tail_file_lines(&path, 3), vec!["c", "d", "e"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tail_seeks_past_huge_file_and_keeps_correct_tail() {
        // Write well over TAIL_READ_BYTES of numbered lines, then confirm the
        // bounded (seek-to-end) read still returns the true last lines intact —
        // i.e. the partial-first-line skip didn't corrupt or drop the real tail.
        let dir = std::env::temp_dir().join(format!("portbay_tail_big_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("big.log");

        let mut content = String::new();
        let mut n = 0u32;
        while (content.len() as u64) < TAIL_READ_BYTES + 256 * 1024 {
            content.push_str(&format!("line-{n}\n"));
            n += 1;
        }
        std::fs::write(&path, &content).unwrap();

        let last = tail_file_lines(&path, 4);
        assert_eq!(last.len(), 4);
        assert_eq!(last[3], format!("line-{}", n - 1));
        assert_eq!(last[0], format!("line-{}", n - 4));
        // Every returned line is a clean, whole record (no fragment leaked in).
        assert!(last.iter().all(|l| l.starts_with("line-")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_keys_strips_comments_and_blanks() {
        let body = "\
# top comment
DATABASE_URL=postgres://localhost/foo

API_KEY=abc123
";
        let kv = parse_dotenv(body);
        assert_eq!(kv.len(), 2);
        assert_eq!(
            kv[0],
            ("DATABASE_URL".into(), "postgres://localhost/foo".into())
        );
        assert_eq!(kv[1], ("API_KEY".into(), "abc123".into()));
    }

    #[test]
    fn unwraps_matched_quotes_only() {
        let kv = parse_dotenv("A=\"with spaces\"\nB='single'\nC=\"mismatch'");
        assert_eq!(kv[0].1, "with spaces");
        assert_eq!(kv[1].1, "single");
        assert_eq!(kv[2].1, "\"mismatch'");
    }

    #[test]
    fn strips_export_prefix() {
        let kv = parse_dotenv("export FOO=bar\n");
        assert_eq!(kv[0], ("FOO".into(), "bar".into()));
    }

    #[test]
    fn ignores_lines_without_equals_or_empty_keys() {
        let kv = parse_dotenv("notakv\n=missingkey\nGOOD=ok\n");
        assert_eq!(kv.len(), 1);
        assert_eq!(kv[0].0, "GOOD");
    }
}
