import { browser } from "$app/environment";

import { safeInvoke } from "$lib/ipc";
import { crashSurface } from "$lib/stores/crashSurface.svelte";

let installed = false;

/**
 * When true, transient `error`/`unhandledrejection` events are ignored rather
 * than recorded as crashes. Set while the page is unloading (a reload/navigation
 * aborts in-flight async work — IPC promises, listeners — which throws/rejects
 * on the way out; that's not a crash) and, in dev, around each Vite HMR update.
 */
let suppressing = false;

/** Suppress capture for a short window, e.g. across an HMR module swap. */
function suppressBriefly(ms = 1500): void {
  suppressing = true;
  window.setTimeout(() => {
    suppressing = false;
  }, ms);
}

/**
 * Browser noise that fires a `window` "error" event but is **not** a crash, so
 * it must not be recorded or surfaced as one. The ResizeObserver loop
 * notifications are the canonical case: the spec defines them as a benign
 * "some callbacks were deferred to the next frame" signal (every major browser
 * dispatches them), routinely emitted by content-measuring widgets like
 * CodeMirror and xterm. They carry no `error`/stack and need no action.
 */
function isBenignErrorEvent(message: string): boolean {
  return /ResizeObserver loop (completed with undelivered notifications|limit exceeded)/.test(
    message,
  );
}

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
  // A reload/navigation or a dev HMR swap is in progress — the error is churn
  // from tearing the context down, not a crash. Drop it silently.
  if (suppressing) return;
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

  // A full reload/navigation tears down in-flight work; stop capturing the
  // moment we start unloading so the teardown errors aren't logged as a crash.
  window.addEventListener("beforeunload", () => {
    suppressing = true;
  });

  // Dev only: Vite swaps modules under the running app on save. The transient
  // errors/rejections that fire mid-swap (or before a full HMR reload) are
  // hot-reload churn, never crashes — suppress capture around each update.
  if (import.meta.env.DEV && import.meta.hot) {
    const hot = import.meta.hot;
    hot.on("vite:beforeUpdate", () => suppressBriefly());
    hot.on("vite:beforeFullReload", () => suppressBriefly());
    hot.on("vite:invalidate", () => suppressBriefly());
    // A clean partial update finished without reloading — resume immediately.
    hot.on("vite:afterUpdate", () => {
      suppressing = false;
    });
  }

  window.addEventListener("error", (event) => {
    const message = event.message || messageFromUnknown(event.error);
    // Benign browser notifications (e.g. ResizeObserver loop) fire here but
    // aren't crashes — drop them so they never reach the crash surface.
    if (isBenignErrorEvent(message)) return;
    void capture("error", message, stackFromUnknown(event.error));
  });

  window.addEventListener("unhandledrejection", (event) => {
    void capture(
      "unhandledrejection",
      messageFromUnknown(event.reason),
      stackFromUnknown(event.reason),
    );
  });
}
