/**
 * micSession — the ONE owner of the macOS dictation session on the frontend.
 *
 * Why this exists: every dictation surface (SSH agent composer, command gate,
 * card spec/comment) used to hand-roll the same session state machine —
 * optimistic `dictating` flags, per-effect event listeners, blur guards, and
 * teardown stops. The hand-rolled copies shared one fatal flaw: the UI
 * claimed "recording" the moment the mic was clicked, while the backend can
 * legitimately spend 1.5–9.5 s before the mic is hot (teardown cool-down +
 * DictationIM confirmation + stale-mode retry — see
 * `src-tauri/src/commands/system.rs`). Words spoken in that window were lost,
 * and a second click during it toggled the session off — the "mic stops
 * immediately / works on the second try" class of bugs.
 *
 * The controller makes the state machine honest and singular:
 *
 *   idle ──toggle──▶ arming ──dictation://listening──▶ live ──toggle/blur/
 *     ▲                │  ▲                              │     OS-end──▶ idle
 *     └──cancel/fail───┘  └─(already-live short-circuit)─┘
 *
 *   • `arming` — start requested, mic NOT hot yet. The button shows the red
 *     pulsing mic but no clock and no stop glyph (2026-06-06, user request:
 *     the old click-time clock read as "already recording" seconds before
 *     the OS mic was hot, and words spoken in that window were lost). A
 *     click here cancels cleanly (and if the OS start lands anyway, it is
 *     stopped — never orphaned) instead of double-toggling.
 *   • `live`   — DictationIM confirmed listening (or the backend's
 *     already-live short-circuit said so). Speech lands. The button becomes
 *     the stop control and the mm:ss clock starts counting HERE.
 *
 * One OS session exists machine-wide, so one controller instance exists
 * app-wide. Surfaces register per-toggle hooks (focus + rewrite snapshot on
 * begin, rewrite finish on end) and read `phase`/`owner` for their UI.
 * Backend invokes are serialized through an internal promise chain so a
 * stop→start handoff can never reorder into start→stop (the backend's
 * transition lock serializes too, but only in arrival order).
 */
import { listen } from "@tauri-apps/api/event";

import { safeInvoke } from "$lib/ipc";
import { errorBus } from "$lib/stores/errors.svelte";
import { preferences } from "$lib/stores/preferences.svelte";

import {
  resolveMicEngine,
  resolveToggleAction,
  type MicEngine,
  type MicPhase,
} from "./engine";

export type { MicPhase };

// Phase + engine types and the pure routing/toggle decisions live in the plain
// (rune-free) `./engine` module so they're unit-testable without the Svelte
// compiler. The resolved engine is pinned for a session's lifetime (a Settings
// change mid-session must not split a start/stop pair across engines). Both
// engines emit the same `dictation://listening`/`dictation://ended` events, so
// the phase machine below is engine-agnostic.

export interface MicSurfaceHooks {
  /** Session granted to this owner: focus the field and snapshot it for the
   * rewrite layer (runs at arming entry, before anything can be inserted). */
  begin(): void;
  /** What this session means, queried AFTER begin() (the rewriter decides
   * edit-vs-dictation from the selection it snapshots there). Labels the
   * notch overlay's leading slot; absent = "dictation". Wire to
   * `DictationRewriter.sessionMode`. */
  mode?(): "dictation" | "edit" | "rewrite";
  /** Local STT engine only: splice the final transcript at the begin()-time
   * caret/selection, BEFORE `end()` runs — the write macOS dictation does
   * itself when it is the engine (wire to `DictationRewriter.insert`). */
  insertTranscript?(transcript: string): void;
  /** Session over (stop click, cancel, blur, OS end, handoff, failed start).
   * Fires exactly once per granted session — the single entry point for
   * `DictationRewriter.finish()`. */
  end(): void;
}

class MicSessionController {
  /** Honest session phase — `live` only after the OS confirmed listening. */
  phase = $state<MicPhase>("idle");
  /** Which surface holds the session (`null` when idle). */
  owner = $state<string | null>(null);
  /** Seconds since the mic went LIVE — the mm:ss clock starts when the OS
   * confirms listening, not at the click (counting through the arming
   * window claimed seconds of recording that never captured anything). */
  seconds = $state(0);
  /** Live partial transcript (local engine only; empty otherwise) — overlay
   * display, never written into the field. */
  partial = $state<string>("");

  /** Invalidates in-flight async work when the session it belonged to ends. */
  #generation = 0;
  #hooks: MicSurfaceHooks | null = null;
  /** Engine pinned for the current session (see MicEngine). */
  #engine: MicEngine = "macos";
  /** The local model the current session captures with. */
  #localModel = "";
  /** FIFO chain for backend invokes — a handoff's stop always lands before
   * the next start. */
  #chain: Promise<unknown> = Promise.resolve();
  #timer: ReturnType<typeof setInterval> | null = null;
  #listeners: Promise<void> | null = null;
  /** Click timestamp of the current start attempt, for the latency
   * breadcrumb logged when the session goes live. */
  #armedAt = 0;
  /** Prevents repeating the "no local model" notice every click — one toast
   * per app session is enough; the bell keeps the record for later reference. */
  #noModelNoticeShown = false;

  constructor() {
    // Register the OS-truth listeners NOW, not on first click — `listen()`
    // round-trips before it's armed, and paying that inside the first
    // mic click both delays the start and risks missing a fast
    // `dictation://listening`.
    void this.#ensureListeners();
  }

  /** Whether `owner` holds the session in any phase (recording UI is up). */
  heldBy(owner: string): boolean {
    return this.phase !== "idle" && this.owner === owner;
  }

  /** Whether `owner` holds a session that is still arming (mic not hot). */
  armingFor(owner: string): boolean {
    return this.phase === "arming" && this.owner === owner;
  }

  /** Whether `owner` holds a live session (mic hot, clock running). */
  liveFor(owner: string): boolean {
    return this.phase === "live" && this.owner === owner;
  }

  /** The mic click. Idle → start for `owner`; held by `owner` → stop/cancel;
   * held by another surface → handoff (their session ends first). */
  toggle(owner: string, hooks: MicSurfaceHooks): void {
    switch (resolveToggleAction(this.phase, this.owner, owner)) {
      case "stop":
        this.#end();
        return;
      case "handoff":
        this.#end(); // close the other surface's session first
        break;
      case "start":
        break;
    }
    void this.#start(owner, hooks);
  }

  /** End the session if `owner` holds it — blur, unmount, push-to-talk
   * release. No-op otherwise, so surfaces can call it unconditionally. */
  release(owner: string): void {
    if (this.heldBy(owner)) this.#end();
  }

  async #start(owner: string, hooks: MicSurfaceHooks): Promise<void> {
    this.owner = owner;
    this.phase = "arming";
    this.#hooks = hooks;
    this.#armedAt = performance.now();
    this.partial = "";
    // Engine pinned per session. "local" needs a chosen model — without one
    // the session quietly runs on macOS dictation (Settings points the user
    // at the model picker; a dead mic button would be worse).
    const prefs = preferences.value.dictation;
    this.#engine = resolveMicEngine(prefs.sttEngine, prefs.sttModel);
    this.#localModel = prefs.sttModel;
    // Asked for the local engine but no model is chosen — `resolveMicEngine`
    // falls back to macOS dictation so the mic still works, but say so instead
    // of silently switching engines. Points at the model picker (Settings →
    // Speech-to-Text on the AI page).
    //
    // Category "project-error" has toast:true in the default notification prefs
    // so this lands as a visible bottom-right toast, not just the bell. Severity
    // "warning" is appropriate: not broken, but action is needed. The
    // `#noModelNoticeShown` flag keeps it to one toast per app session — the bell
    // already deduplicates within a 2 s window, but repeated clicks would stack
    // toasts over minutes without this guard.
    if (prefs.sttEngine === "local" && this.#engine !== "local" && !this.#noModelNoticeShown) {
      this.#noModelNoticeShown = true;
      errorBus.push({
        code: "DICTATION_NO_LOCAL_MODEL",
        category: "project-error",
        whatHappened: "No local model selected — using Apple Speech",
        whyItMatters:
          "You picked the on-device speech engine but haven't chosen a model yet, " +
          "so this session is running on Apple Speech. Download and select a local model in " +
          "Settings → Speech-to-Text to transcribe on-device.",
        whoCausedIt: "user",
        severity: "warning",
        actions: [],
      });
    }
    const engine = this.#engine;
    const generation = ++this.#generation;
    // Focus + rewrite-layer snapshot BEFORE the session can insert anything.
    hooks.begin();
    // Queried after begin() — the rewriter only knows whether this is a
    // voice edit once it has snapshotted the selection.
    const mode = hooks.mode?.() ?? "dictation";

    // The enqueue keeps this start behind any in-flight stop (handoff /
    // quick restart) — the backend additionally sleeps out its teardown
    // cool-down, all of which the `arming` phase shows honestly.
    const outcome = await this.#enqueue(async () => {
      await this.#ensureListeners();
      try {
        if (engine === "local") {
          // Resolves once the sidecar's mic is hot (or fails) — model load
          // happens inside this call, which the arming phase shows honestly.
          await safeInvoke<void>("stt_start_capture", { model: this.#localModel, mode });
          return "started";
        }
        return await safeInvoke<string>("start_dictation");
      } catch {
        // safeInvoke already toasted the IPC failure (and for the local
        // engine, the start error's detail).
        return "invoke_failed";
      }
    });

    if (generation !== this.#generation) {
      // Cancelled / superseded while confirming. If the start landed
      // anyway, nobody is watching that session — close it.
      if (outcome === "started") {
        void this.#enqueue(() =>
          (engine === "local"
            ? safeInvoke("stt_cancel_capture")
            : safeInvoke("stop_dictation")
          ).catch(() => {}),
        );
      }
      return;
    }

    if (outcome === "started") {
      // The listening event may have beaten the command's response; only
      // flip if it hasn't. The backend's already-live short-circuit emits no
      // event at all, so this is also what arms that path.
      if (this.phase === "arming") this.#goLive();
    } else {
      this.#explain(outcome);
      this.#end();
    }
  }

  /** Close the current session from any phase. Single exit path: flips the
   * UI, stops a live session, invalidates in-flight work, fires `end()`.
   *
   * Engine divergence lives here: macOS dictation already typed the words
   * into the field, so `end()` fires immediately; the local engine's words
   * only exist as the sidecar's final transcript, so the stop awaits it,
   * delivers it via `insertTranscript`, and THEN fires `end()` — the
   * rewrite layer's diff must see the transcript in the field. */
  #end(): void {
    const wasLive = this.phase === "live";
    const engine = this.#engine;
    const hooks = this.#hooks;
    this.#generation++;
    this.#hooks = null;
    this.phase = "idle";
    this.owner = null;
    this.partial = "";
    this.#stopClock();

    if (engine === "local" && wasLive) {
      void this.#enqueue(async () => {
        try {
          const transcript = await safeInvoke<string>("stt_stop_capture");
          if (transcript?.trim()) hooks?.insertTranscript?.(transcript);
        } catch {
          // safeInvoke toasted; a failed final pass keeps the field as-is —
          // same zero-data-loss degrade as every rewrite failure.
        }
        hooks?.end();
      });
      return;
    }

    if (wasLive) {
      // The backend only sends a real `stopDictation:` when a session is
      // live — an OS-initiated end already cleared that, so this no-ops.
      void this.#enqueue(() => safeInvoke("stop_dictation").catch(() => {}));
    }
    // Fire the rewrite layer now (it settles 350 ms before reading the
    // field), in parallel with the backend stop — same timing as before.
    hooks?.end();
  }

  #goLive(): void {
    this.phase = "live";
    // The clock is the "speak now" signal — it must not run before the mic
    // is hot, so it starts here, not at the click.
    this.#startClock();
    // Latency breadcrumb (debug-level backend log): click → mic hot. The
    // backend logs its own entry→listening spans; the difference exposes
    // IPC/scheduling delay (see the async-worker starvation history).
    const ms = Math.round(performance.now() - this.#armedAt);
    void safeInvoke("dictation_trace", { msg: `mic live ${ms}ms after click (${this.owner})` }).catch(
      () => {},
    );
  }

  #startClock(): void {
    this.#stopClock();
    let secs = 0;
    this.#timer = setInterval(() => {
      secs += 1;
      this.seconds = secs;
    }, 1000);
  }

  #stopClock(): void {
    if (this.#timer) clearInterval(this.#timer);
    this.#timer = null;
    this.seconds = 0;
  }

  /** Append a backend op to the FIFO chain and return its result. */
  #enqueue<T>(op: () => Promise<T>): Promise<T> {
    const next = this.#chain.then(op, op);
    this.#chain = next.catch(() => {});
    return next;
  }

  /** Register the OS-truth listeners once, BEFORE the first start is sent —
   * `listen()` resolves asynchronously, and a per-session registration could
   * miss a fast `dictation://listening` (the old per-effect design's race).
   * Kicked from the constructor so the first click doesn't pay the
   * registration round-trip; the start path still awaits it for safety.
   * Resilient: outside a Tauri webview (tests) the stored promise resolves
   * instead of poisoning every later start. */
  #ensureListeners(): Promise<void> {
    this.#listeners ??= Promise.all([
      listen("dictation://listening", () => {
        // Confirmation for OUR arming session. A listening event while idle
        // is someone else's dictation (system shortcut) — not ours to track.
        if (this.phase === "arming") this.#goLive();
      }),
      listen("dictation://ended", () => {
        // macOS closed the session itself (HUD "Done", silence timeout).
        // While arming this is the start path's stale-mode un-wedge churn —
        // it must not close the brand-new recording UI. For the local
        // engine this event follows OUR stop (phase already idle) — no-op.
        if (this.phase === "live") this.#end();
      }),
      listen<{ text: string }>("stt://partial", (event) => {
        // Local engine's live hypothesis — overlay only, never the field.
        if (this.phase === "live" && this.#engine === "local") {
          this.partial = event.payload?.text ?? "";
        }
      }),
      listen("dictation://stop-request", () => {
        // The notch overlay's stop button — ends whichever surface holds
        // the session (same exit path as the surface's own stop click).
        if (this.phase !== "idle") this.#end();
      }),
    ]).then(
      () => {},
      () => {},
    );
    return this.#listeners;
  }

  /** Outcome → user-facing explanation. Centralized so every surface gets
   * the same (single) toast. `os_dialog` stays quiet — macOS is showing its
   * own "Enable Dictation?" dialog, which is the message. */
  #explain(outcome: string): void {
    if (outcome === "unavailable") {
      errorBus.push({
        code: "DICTATION_UNAVAILABLE",
        category: "agent-board",
        whatHappened: "macOS didn't accept the dictation request",
        whyItMatters:
          "This macOS version doesn't expose the system dictation trigger. " +
          "You can still dictate with your keyboard shortcut " +
          "(System Settings → Keyboard → Dictation).",
        whoCausedIt: "system",
        severity: "warning",
        actions: [],
      });
    } else if (outcome === "unsupported") {
      errorBus.push({
        code: "DICTATION_UNSUPPORTED",
        category: "agent-board",
        whatHappened: "Dictation is macOS-only",
        whyItMatters: "Voice-to-text uses the system dictation shortcut, available on macOS.",
        whoCausedIt: "user",
        severity: "info",
        actions: [],
      });
    } else if (outcome === "not_engaged") {
      errorBus.push({
        code: "DICTATION_NOT_ENGAGED",
        category: "agent-board",
        whatHappened: "Dictation didn't start listening",
        whyItMatters:
          "macOS accepted the request but never opened a dictation session, " +
          "even after PortBay reset the dictation service. Try the mic again; " +
          "if it keeps happening, toggle Dictation off and on in " +
          "System Settings → Keyboard.",
        whoCausedIt: "system",
        severity: "warning",
        actions: [],
      });
    }
  }
}

/** The app-wide dictation session — one OS session exists, so one of these. */
export const micSession = new MicSessionController();
