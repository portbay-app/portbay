import { browser } from "$app/environment";

import { safeInvoke } from "$lib/ipc";

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

export function installCrashReporter() {
  if (!browser || installed) return;
  installed = true;

  window.addEventListener("error", (event) => {
    void safeInvoke("record_js_error", {
      kind: "error",
      message: event.message || messageFromUnknown(event.error),
      stack: stackFromUnknown(event.error),
    }).catch(() => {});
  });

  window.addEventListener("unhandledrejection", (event) => {
    void safeInvoke("record_js_error", {
      kind: "unhandledrejection",
      message: messageFromUnknown(event.reason),
      stack: stackFromUnknown(event.reason),
    }).catch(() => {});
  });
}
