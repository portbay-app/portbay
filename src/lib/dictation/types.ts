/**
 * Smart Dictation — shared wire/UI types.
 *
 * Mirrors `src-tauri/src/dictation.rs` (contexts/modes serialize snake_case)
 * and `src-tauri/src/commands/dictation.rs` (the `RewriteOutcome` envelope).
 */

/** Where the dictated text is going — drives the smart-mode rewrite rules.
 * Derived from the surface that owns the focused field (plus cheap content
 * heuristics like a card's "bug" label), never from the transcript itself. */
export type RewriteContext =
  | "general_note"
  | "todo_task"
  | "agent_prompt"
  | "terminal_command"
  | "git_commit"
  | "deploy_note"
  | "bug_report";

/** What transcribed the speech — pinned at session start (mirrors
 * `micSession`'s engine pin) and passed to the rewrite so it can match the
 * input's quality:
 *   • "raw"   — macOS system dictation: raw ASR, needs cleanup + structure
 *               (the shipped, fully-probed behavior; the default everywhere).
 *   • "clean" — on-device Whisper/Parakeet: already punctuated, so the rewrite
 *               focuses on layout/arrangement instead of cleanup.
 * Mirrors `InputSource` in `src-tauri/src/dictation.rs` (serialized snake_case). */
export type InputSource = "raw" | "clean";

/** Backend rewrite result. Anything but "rewritten" = keep the raw transcript. */
export interface RewriteOutcome {
  status: "rewritten" | "failed" | "cancelled" | "no_model";
  text: string | null;
  detail: string | null;
}

/** Provider liveness + installed models (Settings → AI → Smart Dictation). */
export interface DictationProviderStatus {
  reachable: boolean;
  models: string[];
  defaultModel: string | null;
  /** Machine-readable unavailability reason (Apple provider): requires_macos_26 |
   * device_not_eligible | apple_intelligence_not_enabled | model_not_ready |
   * sidecar_missing | sidecar_failed | unavailable. Ollama leaves it null. */
  reason: string | null;
}

/** "Dictate anywhere" feature status (`dictation_anywhere_status` /
 * `dictation_anywhere_arm`): platform support, the Accessibility grant the
 * global hotkey + paste injection need, and whether the global monitors are
 * live this app run. */
export interface DictationAnywhereStatus {
  supported: boolean;
  trusted: boolean;
  monitoring: boolean;
}

/** UI phase of the rewrite layer for one dictation surface.
 * "restored" is Edit Mode's failure exit: the spoken instruction couldn't be
 * applied, so the pre-edit text (including the selection it overwrote) was
 * put back. "unavailable" is an EXPLICIT action (Writing Tools) that couldn't
 * run at all — provider unreachable, no model, or empty output — distinct from
 * a genuine no-change so the user knows to check their rewrite model rather
 * than assume nothing needed doing. */
export type RewritePhase =
  | "idle"
  | "rewriting"
  | "done"
  | "kept-raw"
  | "restored"
  | "unavailable";
