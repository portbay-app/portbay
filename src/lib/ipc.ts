/**
 * Canonical Tauri invoke wrapper.
 *
 * Every frontend → Rust IPC call goes through `safeInvoke`. Direct
 * `invoke()` is disallowed (enforce by grep / convention; see card #4
 * outcome).
 *
 * Behaviour:
 *   - Calls Tauri's `invoke<T>` under the hood.
 *   - On rejection, parses the structured `CommandError` envelope from the
 *     rejection payload. Falls back to a synthetic envelope when the
 *     rejection isn't shaped right (e.g. the command doesn't exist or
 *     Tauri itself errored before reaching Rust).
 *   - Pushes the envelope into `errorBus` as a toast.
 *   - Re-throws the (parsed or synthetic) `CommandError` so callers can
 *     also handle it inline if they want — typical pattern is to ignore
 *     the throw because the toast already informed the user.
 */
import { invoke, type InvokeArgs } from "@tauri-apps/api/core";

import { errorBus } from "$lib/stores/errors.svelte";
import { isCommandError, type CommandError } from "$lib/types/error";

export async function safeInvoke<T>(
  command: string,
  args?: InvokeArgs,
): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (raw) {
    const err = normalise(raw);
    errorBus.push(err);
    throw err;
  }
}

/**
 * Try to coerce an arbitrary rejection value into a `CommandError`. Used
 * by `safeInvoke` and also exported for tests / callers that want to
 * surface non-IPC errors (e.g. a network call) through the same UI.
 */
export function normalise(raw: unknown): CommandError {
  if (isCommandError(raw)) return raw;
  // Tauri can reject with a string (e.g. unknown-command errors before
  // reaching Rust). Capture it as a generic envelope.
  return {
    code: "UNKNOWN",
    whatHappened: typeof raw === "string" ? raw : String(raw),
    whyItMatters: "An unexpected error occurred.",
    whoCausedIt: "system",
    actions: [],
  };
}
