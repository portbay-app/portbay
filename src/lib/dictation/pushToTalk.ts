/**
 * Push-to-talk for dictation surfaces: hold the Fn (🌐) key to dictate into
 * the focused field, release to stop — the freeflow/Wispr-Flow interaction,
 * scoped to PortBay's own dictation-enabled fields.
 *
 * The Fn key never reaches the WKWebView as a DOM event on macOS, so the
 * backend watches it with a local NSEvent monitor (active-app only) and
 * forwards transitions as the `dictation://fn` event (bool payload). Two
 * disambiguation rules keep Fn's other jobs working:
 *   • engagement waits HOLD_MS — a quick tap (emoji picker / input-source
 *     switch / the system's own double-tap dictation shortcut) never
 *     triggers, and
 *   • any real key while holding cancels the pending hold (Fn+arrow = page
 *     navigation, Fn+F-row = function keys) or ends a live session (the
 *     user switched to the keyboard).
 *
 * Each surface owns one instance, wired to its existing mic toggle, so
 * push-to-talk reuses the exact start/stop/rewrite path the mic button takes
 * (snapshot → OS dictation → diff → polish, including voice Edit Mode when a
 * selection exists). Dispose in the surface's teardown.
 */
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** Hold this long before engaging — long enough to skip taps and Fn-chords,
 * short enough to feel immediate. */
const HOLD_MS = 300;

export interface PushToTalkHooks<W extends string> {
  /** Pref + surface gate, checked at arm AND engage time. */
  enabled(): boolean;
  /** Which dictation field is focused right now, or null. */
  target(): W | null;
  /** Start dictation for the field (the surface's mic-toggle path). */
  start(which: W): void;
  /** End the surface's dictation session (whatever field it's on). */
  stop(): void;
}

/**
 * Install the Fn listener + cancel keylisteners. Returns a dispose function.
 * The keydown listener is capture-phase so a field's own handlers (e.g.
 * Enter to send) can't shadow the cancel.
 */
export function createPushToTalk<W extends string>(hooks: PushToTalkHooks<W>): () => void {
  let holdTimer: ReturnType<typeof setTimeout> | null = null;
  let engaged: W | null = null;

  const disarm = () => {
    if (holdTimer) clearTimeout(holdTimer);
    holdTimer = null;
  };
  const release = () => {
    disarm();
    if (engaged !== null) {
      engaged = null;
      hooks.stop();
    }
  };

  const onFnDown = () => {
    if (engaged !== null || holdTimer !== null) return;
    if (!hooks.enabled()) return;
    const which = hooks.target();
    if (which === null) return;
    holdTimer = setTimeout(() => {
      holdTimer = null;
      // Re-check: focus or prefs may have moved during the hold.
      if (hooks.enabled() && hooks.target() === which) {
        engaged = which;
        hooks.start(which);
      }
    }, HOLD_MS);
  };

  const unlistenFn = listen<boolean>("dictation://fn", (e) => {
    if (e.payload) onFnDown();
    else release();
  });

  // A real key while holding: an Fn-chord (cancel the pending hold) or
  // typing over a live push-to-talk session (end it).
  const onKeyDown = () => {
    if (holdTimer !== null) disarm();
    else if (engaged !== null) release();
  };

  // Focus loss / app switch mid-hold: never leave a session running. (The
  // backend monitor is local-only, so the Fn release would go unseen.)
  const onWindowBlur = () => release();

  window.addEventListener("keydown", onKeyDown, true);
  window.addEventListener("blur", onWindowBlur);
  return () => {
    release();
    window.removeEventListener("keydown", onKeyDown, true);
    window.removeEventListener("blur", onWindowBlur);
    void unlistenFn.then((un: UnlistenFn) => un());
  };
}
