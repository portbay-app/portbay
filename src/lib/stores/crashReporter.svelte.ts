import { browser } from "$app/environment";

import { safeInvoke } from "$lib/ipc";
import { crashSurface } from "$lib/stores/crashSurface.svelte";

let installed = false;

function messageFromUnknown(value: unknown): string {
  if (value instanceof Error) return value.message;
  if (typeof value === "string") return value;
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

function stackFromUnknown(value: unknown): string | undefined {
  return value instanceof Error ? value.stack : undefined;
}

/**
 * A stable de-dup key for a captured error: kind + message + the first stack
 * frame. The same fault firing repeatedly (e.g. a render loop) collapses to one
 * signature, so the crash surface prompts about it at most once.
 */
function signatureFor(kind: string, message: string, stack: string | undefined): string {
  const topFrame = stack?.split("\n").find((l) => l.trim().startsWith("at "))?.trim() ?? "";
  return `${kind}::${message}::${topFrame}`;
}

/** Record the crash locally, then let the surface decide whether to prompt. */
async function capture(kind: string, message: string, stack: string | undefined): Promise<void> {
  try {
    const id = await safeInvoke<string>("record_js_error", { kind, message, stack });
    if (id) await crashSurface.noteLiveError(id, signatureFor(kind, message, stack));
  } catch {
    /* recording failed — nothing actionable, and we never block the app */
  }
}

export function installCrashReporter() {
  if (!browser || installed) return;
  installed = true;

  window.addEventListener("error", (event) => {
    void capture(
      "error",
      event.message || messageFromUnknown(event.error),
      stackFromUnknown(event.error),
    );
  });

  window.addEventListener("unhandledrejection", (event) => {
    void capture(
      "unhandledrejection",
      messageFromUnknown(event.reason),
      stackFromUnknown(event.reason),
    );
  });
}
