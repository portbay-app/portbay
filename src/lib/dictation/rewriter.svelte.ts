/**
 * Smart Dictation — the per-surface rewrite controller.
 *
 * One instance per dictation target (SSH agent composer, card spec, card
 * comment). The owning component calls `begin()` when the OS dictation
 * session starts and `finish()` when it ends (both already have a single
 * exit path — the recording `$effect`'s teardown); everything else is
 * handled here:
 *
 *   snapshot → diff (what did dictation insert?) → gate (worth rewriting?)
 *   → backend rewrite (cancellable, non-blocking) → verify the field didn't
 *   move underneath us → splice → offer undo.
 *
 * Failure philosophy: the raw transcript is ALREADY in the field when we
 * start — macOS typed it there — so every failure mode (provider down,
 * timeout, cancel, validation reject, user edited mid-flight) simply leaves
 * the user's text alone. Nothing is ever lost; a rewrite can only improve or
 * no-op.
 */
import { safeInvoke } from "$lib/ipc";
import { preferences } from "$lib/stores/preferences.svelte";

import {
  extractInsertion,
  needsSpace,
  relocateInsertion,
  spliceRewrite,
  worthRewriting,
  type Insertion,
} from "./splice";
import type { InputSource, RewriteContext, RewriteOutcome, RewritePhase } from "./types";

/** Diagnosis breadcrumbs, kept deliberately (2026-06-06): the WKWebView
 * console is invisible in `tauri dev` output, so the rewrite layer's
 * decision points route through the backend log instead. Debug-level only —
 * silent at the production filter. Enable with
 * `PORTBAY_LOG="info,portbay_lib::commands::dictation=debug"`. */
function trace(msg: string): void {
  void safeInvoke("dictation_trace", { msg }).catch(() => {});
}

/** How long the "Polished — Undo" affordance stays up. Dismissing the CHIP
 * does not disarm ⌘Z — the undo stays armed until the field diverges or the
 * next dictation session begins (see `#dismissChip` vs `#clearChip`). */
const DONE_DISMISS_MS = 8_000;
/** How long the quiet "kept as spoken" note stays up. */
const KEPT_RAW_DISMISS_MS = 4_000;
/** macOS may still replace its last hypothesis right after the session ends;
 * give the final insert a beat to land before reading the field. */
const SETTLE_MS = 350;

/** Provider latched off for this session after a structural `no_model`
 * failure. Smart mode is on by default with the Apple provider, so machines
 * without Apple Intelligence would otherwise pay a doomed sidecar spawn and
 * show a "kept as spoken" chip after every dictation. Shared across all
 * rewriter instances (the provider is global); keyed by provider id so
 * switching providers in Settings re-enables immediately. Ollama is never
 * latched — its `no_model` ("server up, nothing pulled") is user-fixable and
 * worth surfacing. */
let latchedProvider: string | null = null;

/** Re-arm a latched provider — Settings calls this when its probe finds the
 * provider available again (e.g. Apple Intelligence finished downloading). */
export function clearProviderLatch(): void {
  latchedProvider = null;
}

/** Throttle for the model prewarm hint fired at dictation start (the OS can
 * evict the on-device model after idle, so once-per-run isn't enough; every
 * session would be noise). Module-level: the model is global, not
 * per-surface. */
let lastPrewarmAt = 0;
const PREWARM_INTERVAL_MS = 5 * 60_000;

/** Ask the backend to page the rewrite model in — fire-and-forget, so the
 * rewrite at session end doesn't pay first-use model load. Backend no-ops
 * for providers that don't need it (Ollama keeps itself warm). */
function prewarmProvider(): void {
  const now = Date.now();
  if (now - lastPrewarmAt < PREWARM_INTERVAL_MS) return;
  lastPrewarmAt = now;
  const prefs = preferences.value.dictation;
  void safeInvoke("dictation_prewarm", {
    provider: { kind: prefs.provider, endpoint: prefs.endpoint, model: prefs.model },
  }).catch(() => {});
}

export interface RewriterTarget {
  /** Current field value (the bound $state the textarea writes). */
  read(): string;
  /** Replace the field value (splice result / undo). */
  write(next: string): void;
  /** Rewrite context for this surface, resolved at finish time so content
   * heuristics (bug label, @mention) see the final state. */
  context(): RewriteContext;
  /** Surface-local vocabulary: technical terms visible on this surface
   * (pending command, conversation, card title…) whose exact spelling the
   * rewrite should restore when the speech clearly refers to them. Resolved
   * at finish time; merged ahead of the registry vocabulary backend-side.
   * See $lib/dictation/vocabulary. */
  vocabulary?(): string[];
  /** Registry project id when this surface belongs to one (task cards) —
   * scopes the learned vocabulary: terms learned here rank higher inside
   * the same project. Null/absent for project-less surfaces (SSH). */
  project?(): string | null;
  /** Veto rewriting for this session (e.g. composer holds a slash command).
   * Receives the inserted segment and the full field value. */
  skip?(inserted: string, fieldValue: string): boolean;
  /** Called after a rewrite (or undo) changed the field — surfaces that
   * persist on blur need an explicit save, since a splice isn't a blur. */
  onApplied?(): void;
  /** Current selection range in the field's element, for voice Edit Mode.
   * Return null when the element is gone. */
  selection?(): { start: number; end: number } | null;
}

/** The selection captured at session start when Edit Mode engages. */
interface EditSelection {
  start: number;
  end: number;
  /** The selected text itself (from the start-of-session snapshot). */
  text: string;
}

export class DictationRewriter {
  /** Drives the status chip. */
  phase = $state<RewritePhase>("idle");

  #target: RewriterTarget;
  #snapshot: string | null = null;
  /** Caret/selection captured at session start, for the local engine's
   * transcript insert (`insert()`) — macOS dictation types at the live
   * caret itself; the local engine must reproduce that against the
   * snapshot. Null = no element selection (append at end). */
  #caret: { start: number; end: number } | null = null;
  /** Non-null = this session is a voice EDIT of the captured selection: the
   * dictated words are an instruction about it, not content. */
  #editSel: EditSelection | null = null;
  /** What's transcribing this session, pinned at `begin()` (a Settings change
   * mid-session must not split the source across the start/stop pair — same
   * reason `micSession` pins its engine). Drives the rewrite's layout focus;
   * defaults to "raw" (the shipped, probed behavior). See `InputSource`. */
  #inputSource: InputSource = "raw";
  /** Monotonic token: anything async checks it still owns the controller. */
  #generation = 0;
  #requestId: string | null = null;
  /** `learned` carries what the accepted rewrite fed the vocabulary store
   * (`dictation_rewrite` learns server-side; edits don't), so `undo()` can
   * send the symmetric demotion — a rejected polish must not keep its terms
   * reinforced. */
  #undo: {
    previous: string;
    applied: string;
    learned?: { text: string; context: RewriteContext; project: string | null };
  } | null = null;
  #dismissTimer: ReturnType<typeof setTimeout> | null = null;

  constructor(target: RewriterTarget) {
    this.#target = target;
  }

  /** Whether the rewrite layer is on at all (drives nothing UI-visible until
   * a session actually qualifies — the mic flow is unchanged when off).
   * On by default; the model scales its own intervention (light cleanup →
   * full restructure) and ⌘Z restores the raw transcript. Two ways off:
   * the user explicitly picks the "off" provider (no rewrite at all), or a
   * provider latched off for the session after a structural `no_model`
   * failure. Stale persisted `mode: "off"` prefs are deliberately ignored —
   * the provider is the switch now. */
  get enabled(): boolean {
    const provider = preferences.value.dictation.provider;
    return provider !== "off" && provider !== latchedProvider;
  }

  /** What the current session means, valid after `begin()`: "edit" when a
   * non-empty selection flipped it into Voice Edit Mode, else "dictation".
   * Surfaces feed this to `MicSurfaceHooks.mode` so the notch overlay can
   * label the session. */
  get sessionMode(): "dictation" | "edit" {
    return this.#editSel ? "edit" : "dictation";
  }

  /** OS dictation session started: snapshot the field. Supersedes any
   * rewrite still in flight from a previous session.
   *
   * Voice Edit Mode: a non-empty selection at session start (with the
   * rewrite layer on — editing needs the model) flips the session's meaning:
   * the dictated words become an INSTRUCTION about the selected text. macOS
   * will type the instruction over the selection; `finish()` recovers it by
   * diffing, transforms the original selection, and splices the result back. */
  begin(): void {
    this.#abandonInFlight();
    this.#clearChip();
    const snapshot = this.#target.read();
    this.#snapshot = snapshot;
    this.#editSel = null;
    // Pin the transcription source the same way micSession pins its engine: a
    // chosen local model means Whisper/Parakeet (clean input → layout focus),
    // otherwise macOS dictation (raw ASR → cleanup). Resolved here, used at
    // finish() — the field is the same, but the source must be the one that
    // was live when the session started.
    const d = preferences.value.dictation;
    this.#inputSource = d.sttEngine === "local" && d.sttModel ? "clean" : "raw";
    // Captured for the local engine's insert() regardless of the rewrite
    // layer's state — where the transcript lands is transcription behavior,
    // not rewrite behavior.
    this.#caret = this.#target.selection?.() ?? null;
    if (this.enabled) {
      prewarmProvider();
      const sel = this.#caret;
      if (sel && sel.end > sel.start) {
        this.#editSel = { ...sel, text: snapshot.slice(sel.start, sel.end) };
      }
    }
  }

  /** Local STT engine only: place the final transcript into the field at
   * the session-start caret/selection — the exact write macOS dictation
   * performs itself when it is the engine. Runs BEFORE `finish()`, whose
   * snapshot diff then sees this as the session's insertion and the whole
   * rewrite pipeline (including voice Edit Mode, where the transcript
   * overwrites the captured selection as the instruction) behaves
   * identically for both engines. */
  insert(transcript: string): void {
    const text = transcript.trim();
    if (this.#snapshot === null || !text) return;
    const current = this.#target.read();
    // The caret anchors into the SNAPSHOT; if the user edited the field
    // mid-capture the offsets no longer point where they typed — append at
    // the end instead of splicing into the wrong place. (macOS dictation
    // has the same wrinkle: its live caret follows the user's edits; an
    // appended transcript is the honest degrade for ours.)
    const anchored = current === this.#snapshot && this.#caret !== null;
    const start = anchored ? this.#caret!.start : current.length;
    const end = anchored ? this.#caret!.end : current.length;
    const left = current.slice(0, start);
    const right = current.slice(end);
    const lpad = needsSpace(left, text) ? " " : "";
    const rpad = needsSpace(text, right) ? " " : "";
    this.#target.write(left + lpad + text + rpad + right);
    this.#target.onApplied?.();
  }

  /** OS dictation session ended (stop click, blur, unmount): diff, gate,
   * rewrite, splice. Fire-and-forget — never blocks the recording UI. */
  finish(): void {
    if (this.#snapshot === null) {
      trace("finish: no snapshot (begin never ran?)");
      return;
    }
    const before = this.#snapshot;
    const edit = this.#editSel;
    this.#snapshot = null;
    this.#editSel = null;
    if (!this.enabled) {
      trace(
        `finish: disabled (mode=${preferences.value.dictation.mode} provider=${preferences.value.dictation.provider} latched=${latchedProvider})`,
      );
      return;
    }
    trace(`finish: scheduled (edit=${!!edit} beforeLen=${before.length})`);
    const generation = ++this.#generation;
    // Let macOS finalize its last hypothesis before reading the field.
    setTimeout(() => {
      if (generation !== this.#generation) return;
      if (edit) void this.#runEdit(generation, before, edit);
      else void this.#run(generation, before);
    }, SETTLE_MS);
  }

  /** User cancelled (chip ×) or the surface needs the raw text NOW (send
   * pressed mid-rewrite). Keeps whatever is in the field — and keeps any
   * armed undo: dismissing the chip must not take ⌘Z with it. */
  cancel(): void {
    this.#abandonInFlight();
    this.#dismissChip();
  }

  /** Hint the provider's model into memory ahead of an imminent transform —
   * the Writing Tools popover opening is the same "request anticipated"
   * signal a dictation session start is. */
  prewarm(): void {
    if (this.enabled) prewarmProvider();
  }

  /** Writing Tools: transform a selection — either by a typed/preset
   * instruction through the Voice-Edit path (`dictation_edit`, minus the
   * voice), or through the smart-rewrite path with a context override
   * (`dictation_rewrite` — used where BASE_RULES' no-invention machinery
   * beats a bare instruction; see $lib/dictation/writingTools). Unlike a
   * voice edit, nothing has overwritten the selection, so every failure
   * simply leaves the field untouched ("Kept original" chip). Success
   * splices the transformed text over the selection and arms the same ⌘Z
   * undo as a rewrite — but shows NO "Polished — Undo" chip (user feedback
   * 2026-06-06: the chip next to the Writing Tools button is noise there;
   * the popover interaction itself is the confirmation, and ⌘Z stays live).
   *
   * `sel` is the selection captured when the popover OPENED — the popover's
   * own focus changes make the live `target.selection()` unreliable by the
   * time a preset is clicked. */
  async transformSelection(
    action: string | { kind: "edit"; instruction: string } | { kind: "rewrite"; context: RewriteContext },
    sel: { start: number; end: number } | null,
  ): Promise<void> {
    if (!this.enabled) return;
    const resolved =
      typeof action === "string" ? ({ kind: "edit", instruction: action } as const) : action;
    const range = sel ?? this.#target.selection?.() ?? null;
    if (!range || range.end <= range.start) return;
    if (resolved.kind === "edit" && resolved.instruction.trim().length < 3) return;

    const before = this.#target.read();
    const selection = before.slice(range.start, range.end);
    if (!selection.trim()) return;

    // Supersede any rewrite/transform still in flight (also bumps the
    // generation this completion is checked against).
    this.#abandonInFlight();
    this.#dismissChip();
    const generation = this.#generation;
    trace(
      `transform: invoking (${
        resolved.kind === "edit" ? `"${resolved.instruction.slice(0, 40)}"` : resolved.context
      } selLen=${selection.length})`,
    );

    const prefs = preferences.value.dictation;
    const provider = { kind: prefs.provider, endpoint: prefs.endpoint, model: prefs.model };
    const requestId = crypto.randomUUID();
    this.#requestId = requestId;
    this.#setPhase("rewriting");

    let outcome: RewriteOutcome;
    try {
      // `?? failed` covers the simulator, whose mock IPC resolves unknown
      // commands to null — a preset click there must degrade like any other
      // failure, not throw.
      outcome =
        (resolved.kind === "edit"
          ? await safeInvoke<RewriteOutcome>("dictation_edit", {
              requestId,
              selection,
              instruction: resolved.instruction.trim(),
              extraVocabulary: this.#target.vocabulary?.() ?? [],
              project: this.#target.project?.() ?? null,
              provider,
            })
          : await safeInvoke<RewriteOutcome>("dictation_rewrite", {
              requestId,
              text: selection,
              context: resolved.context,
              extraVocabulary: this.#target.vocabulary?.() ?? [],
              project: this.#target.project?.() ?? null,
              mode: "smart",
              provider,
            })) ?? { status: "failed", text: null, detail: null };
    } catch {
      // IPC-level failure (safeInvoke already toasted) — keep the text.
      outcome = { status: "failed", text: null, detail: null };
    }

    if (generation !== this.#generation) {
      trace("transform: superseded mid-flight");
      return;
    }
    this.#requestId = null;
    trace(`transform: outcome=${outcome.status}`);

    if (outcome.status !== "rewritten" || !outcome.text) {
      if (outcome.status === "cancelled") this.#dismissChip();
      else {
        // Same latch rule as rewrites: structurally-unavailable Apple
        // provider goes quiet for the session (and the affordance with it).
        if (outcome.status === "no_model" && prefs.provider === "apple") {
          latchedProvider = prefs.provider;
        }
        // "Kept original" ONLY for a genuine no-op (already clean / too short
        // to bother). A real failure — provider unreachable, no model, or
        // empty/invalid output — says "Rewrite unavailable" instead, so the
        // user checks their rewrite model rather than assuming nothing needed
        // doing. (This is exactly the symptom that read as "stuck on AFM":
        // an Ollama that was down, or a reasoning model returning empty, both
        // surfaced as the indistinguishable "Kept original".)
        const benign =
          !!outcome.detail &&
          (outcome.detail.includes("no change") || outcome.detail.includes("too short"));
        this.#setPhase(benign ? "restored" : "unavailable", KEPT_RAW_DISMISS_MS);
      }
      return;
    }

    // The edit path reports "instruction made no change" as failed
    // backend-side; the rewrite path doesn't — mirror it here so an
    // already-clean selection reads "Kept original" instead of offering a
    // meaningless Undo.
    if (outcome.text === selection || outcome.text === selection.trim()) {
      this.#setPhase("restored", KEPT_RAW_DISMISS_MS);
      return;
    }

    // Splice only if the field still holds exactly what it held when the
    // action fired — the user may have kept typing while the model worked.
    if (this.#target.read() !== before) {
      this.#dismissChip();
      return;
    }
    const applied = before.slice(0, range.start) + outcome.text + before.slice(range.end);
    this.#undo = {
      previous: before,
      applied,
      // Only the rewrite path feeds the learned store backend-side; an
      // edit-path undo has nothing to demote.
      learned:
        resolved.kind === "rewrite"
          ? {
              text: outcome.text,
              context: resolved.context,
              project: this.#target.project?.() ?? null,
            }
          : undefined,
    };
    this.#target.write(applied);
    this.#target.onApplied?.();
    // No "done" chip for Writing Tools (see the doc comment above) — the
    // undo stays armed; only the visual affordance is dropped.
    this.#dismissChip();
  }

  /** Whether ⌘Z should revert the last rewrite instead of the (empty)
   * native undo stack: an undo is armed and the field still holds exactly
   * what we wrote. Programmatic splices bypass the textarea's own history,
   * so the surfaces route ⌘Z here while this is true. */
  get canUndo(): boolean {
    return this.#undo !== null && this.#target.read() === this.#undo.applied;
  }

  /** Revert the last applied rewrite to the raw transcript — only while the
   * field still holds exactly what we wrote. A real revert also demotes the
   * rejected polish's terms in the learned store (fire-and-forget — the
   * splice never waits on disk; see dictation_unlearn). */
  undo(): void {
    if (!this.#undo) return;
    if (this.#target.read() === this.#undo.applied) {
      this.#target.write(this.#undo.previous);
      this.#target.onApplied?.();
      const learned = this.#undo.learned;
      if (learned) {
        trace(`undo: unlearning rejected rewrite (${learned.context})`);
        void safeInvoke("dictation_unlearn", {
          text: learned.text,
          context: learned.context,
          project: learned.project,
        }).catch(() => {});
      }
    }
    this.#clearChip();
  }

  async #run(generation: number, before: string): Promise<void> {
    const after = this.#target.read();
    const insertion = extractInsertion(before, after);
    if (!insertion || !worthRewriting(insertion.inserted)) {
      trace(
        `run: gate skip (insertion=${insertion ? `"${insertion.inserted.slice(0, 40)}"` : "null"} afterLen=${after.length} beforeLen=${before.length})`,
      );
      return;
    }
    if (this.#target.skip?.(insertion.inserted, after)) {
      trace("run: surface skip()");
      return;
    }
    trace(`run: invoking rewrite ("${insertion.inserted.slice(0, 40)}")`);

    const prefs = preferences.value.dictation;
    const requestId = crypto.randomUUID();
    // Resolved once, used for the request AND remembered for undo's
    // symmetric unlearn — content heuristics could resolve differently later.
    const context = this.#target.context();
    const project = this.#target.project?.() ?? null;
    this.#requestId = requestId;
    this.#setPhase("rewriting");

    let outcome: RewriteOutcome;
    try {
      outcome = await safeInvoke<RewriteOutcome>("dictation_rewrite", {
        requestId,
        text: insertion.inserted,
        context,
        extraVocabulary: this.#target.vocabulary?.() ?? [],
        project,
        // Always smart — the prompt itself scales from light cleanup to full
        // restructure based on the input (see build_prompt in dictation.rs).
        mode: "smart",
        provider: {
          kind: prefs.provider,
          endpoint: prefs.endpoint,
          model: prefs.model,
        },
        // Pinned at begin(): clean STT input shifts the rewrite toward layout.
        source: this.#inputSource,
      });
    } catch {
      // IPC-level failure (safeInvoke already toasted) — keep the raw text.
      outcome = { status: "failed", text: null, detail: null };
    }

    // A newer session/cancel superseded this rewrite while it was in flight.
    if (generation !== this.#generation) {
      trace("run: superseded mid-flight");
      return;
    }
    this.#requestId = null;
    trace(`run: outcome=${outcome.status}`);

    if (outcome.status !== "rewritten" || !outcome.text) {
      if (outcome.status === "cancelled") this.#dismissChip();
      else if (outcome.status === "no_model" && prefs.provider === "apple") {
        // Structurally unavailable (no Apple Intelligence on this machine):
        // latch the provider off for the session, silently — the raw
        // transcript is in the field and nagging can't fix the hardware.
        latchedProvider = prefs.provider;
        this.#dismissChip();
      } else this.#setPhase("kept-raw", KEPT_RAW_DISMISS_MS);
      return;
    }

    // The field may have moved while the model worked: re-anchor on the raw
    // segment, and if it's gone or ambiguous, leave the user's text alone.
    const current = this.#target.read();
    const anchor: Insertion | null =
      current === after ? insertion : relocateInsertion(current, insertion.inserted);
    if (!anchor) {
      this.#dismissChip();
      return;
    }
    if (outcome.text === anchor.inserted) {
      // Model says it's already clean — nothing to apply, nothing to undo.
      this.#dismissChip();
      return;
    }

    const applied = spliceRewrite(current, anchor, outcome.text);
    this.#undo = {
      previous: current,
      applied,
      learned: { text: outcome.text, context, project },
    };
    this.#target.write(applied);
    this.#target.onApplied?.();
    this.#setPhase("done", DONE_DISMISS_MS);
  }

  /** Voice Edit Mode session end: what dictation typed (over the selection)
   * is the INSTRUCTION; the captured selection is the text to transform.
   *
   * Outcomes:
   *   • success → `before` with the selection replaced by the transformed
   *     text; Undo restores the pre-edit original.
   *   • any failure → restore `before` (the instruction sitting in the field
   *     is worthless as content, and the selection it overwrote is the
   *     user's data) — shown as "kept original".
   *   • user edited the field mid-flight → leave their text alone.
   */
  async #runEdit(generation: number, before: string, edit: EditSelection): Promise<void> {
    const after = this.#target.read();
    const insertion = extractInsertion(before, after);
    // Nothing dictated (insta-stop) — the selection is untouched; done.
    if (!insertion) return;
    const instruction = insertion.inserted.trim();

    const restoreOriginal = () => {
      if (this.#target.read() === after) {
        this.#target.write(before);
        this.#target.onApplied?.();
        this.#setPhase("restored", KEPT_RAW_DISMISS_MS);
      } else {
        this.#dismissChip();
      }
    };

    if (instruction.length < 3) {
      restoreOriginal();
      return;
    }

    const prefs = preferences.value.dictation;
    const requestId = crypto.randomUUID();
    this.#requestId = requestId;
    this.#setPhase("rewriting");

    let outcome: RewriteOutcome;
    try {
      outcome = await safeInvoke<RewriteOutcome>("dictation_edit", {
        requestId,
        selection: edit.text,
        instruction,
        extraVocabulary: this.#target.vocabulary?.() ?? [],
        project: this.#target.project?.() ?? null,
        provider: {
          kind: prefs.provider,
          endpoint: prefs.endpoint,
          model: prefs.model,
        },
      });
    } catch {
      outcome = { status: "failed", text: null, detail: null };
    }

    if (generation !== this.#generation) return;
    this.#requestId = null;

    if (outcome.status !== "rewritten" || !outcome.text) {
      // Latch a structurally-unavailable Apple provider here too — but still
      // restore: the instruction overwrote the selection, which is the
      // user's data.
      if (outcome.status === "no_model" && prefs.provider === "apple") {
        latchedProvider = prefs.provider;
      }
      restoreOriginal();
      return;
    }

    // Splice the transformed selection into the PRE-edit text — but only if
    // the field still shows exactly what the session left behind.
    if (this.#target.read() !== after) {
      this.#dismissChip();
      return;
    }
    const applied = before.slice(0, edit.start) + outcome.text + before.slice(edit.end);
    this.#undo = { previous: before, applied };
    this.#target.write(applied);
    this.#target.onApplied?.();
    this.#setPhase("done", DONE_DISMISS_MS);
  }

  /** Cancel the backend request, if one is running, and invalidate stale
   * async completions. Best-effort: an unknown id is a backend no-op. */
  #abandonInFlight(): void {
    this.#generation++;
    if (this.#requestId) {
      void safeInvoke("dictation_rewrite_cancel", { requestId: this.#requestId }).catch(
        () => {},
      );
      this.#requestId = null;
    }
  }

  #setPhase(phase: RewritePhase, dismissAfterMs?: number): void {
    if (this.#dismissTimer) clearTimeout(this.#dismissTimer);
    this.#dismissTimer = null;
    this.phase = phase;
    if (dismissAfterMs) {
      // Auto-dismiss hides the CHIP only — ⌘Z must keep working after the
      // "Polished — Undo" affordance times out (it used to disarm here,
      // resurrecting pre-dictation text through native undo instead).
      this.#dismissTimer = setTimeout(() => this.#dismissChip(), dismissAfterMs);
    }
  }

  /** Hide the status chip, keep any armed undo. The undo's real lifetime is
   * `canUndo`'s field check (diverged = gone) and `begin()` (next session
   * supersedes it). */
  #dismissChip(): void {
    if (this.#dismissTimer) clearTimeout(this.#dismissTimer);
    this.#dismissTimer = null;
    this.phase = "idle";
  }

  /** Full reset at session boundaries: chip AND undo. */
  #clearChip(): void {
    this.#dismissChip();
    this.#undo = null;
  }
}
