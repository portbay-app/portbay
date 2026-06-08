/**
 * Pure dictation-engine routing.
 *
 * Kept in a plain `.ts` module (no Svelte runes) so it can be unit-tested
 * without the Svelte compiler, and shared by `micSession` and any other
 * surface that needs the same decision.
 */

/** Which recognizer a session runs on:
 *   • "macos" — system dictation; the OS types into the focused field.
 *   • "local" — the portbay-stt sidecar (Whisper/Parakeet on-device);
 *     PortBay inserts the final transcript itself. */
export type MicEngine = "macos" | "local";

/** Honest session phase — `live` only after the OS/sidecar confirmed it is
 * listening, never at the click (the "arming" window can be 1.5–9.5 s). */
export type MicPhase = "idle" | "arming" | "live";

/** What a mic click resolves to. */
export type ToggleAction = "stop" | "handoff" | "start";

/** Resolve what a mic click does, from the current phase and who holds the
 * session:
 *   • "stop"    — the requester already holds it (any non-idle phase): end it
 *                 (cancel while arming, stop while live);
 *   • "handoff" — another surface holds a session: end theirs, then start ours;
 *   • "start"   — nothing active: start fresh.
 * Pure so the toggle decision is unit-testable apart from the session machine
 * (the IPC + Svelte-runes class). */
export function resolveToggleAction(
  phase: MicPhase,
  currentOwner: string | null,
  requestingOwner: string,
): ToggleAction {
  if (phase !== "idle" && currentOwner === requestingOwner) return "stop";
  if (phase !== "idle") return "handoff";
  return "start";
}

/** Resolve the engine from preferences. "local" requires BOTH the local
 * engine selected and a model chosen; with the engine set to local but no
 * model picked yet, the session falls back to macOS dictation rather than
 * presenting a dead mic button (Settings points the user at the model
 * picker). */
export function resolveMicEngine(sttEngine: string, sttModel: string): MicEngine {
  return sttEngine === "local" && sttModel ? "local" : "macos";
}
