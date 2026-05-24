/**
 * Wire shape of every Tauri command's error reject. Mirrors the Rust
 * `AppError` manual `Serialize` impl in `src-tauri/src/error.rs`.
 *
 * The full envelope-rendering component lands in card #4
 * (`P2 — Error envelope renderer`). The type lives here so other modules
 * can be wired against it from day one.
 */
export type Severity = "success" | "info" | "warning" | "error";

export interface CommandError {
  code: string;
  whatHappened: string;
  whyItMatters: string;
  whoCausedIt: "user" | "system";
  actions: ErrorAction[];
  /** Optional inner detail (stack trace, error chain) for the "Show details"
      expander. Rust side may omit this. */
  details?: string;
  /** Optional severity override. When unset, falls back to mapping by
      `whoCausedIt` (user → warning, system → error) so existing call
      sites keep their previous look. Set explicitly for informational /
      success notifications so the toast doesn't read as a failure. */
  severity?: Severity;
}

export interface ErrorAction {
  label: string;
  /** Frontend command id the button invokes. */
  command?: string;
  /** External URL the button opens via the shell. Takes precedence over
      `command` when both are set. */
  url?: string;
  /** Optional argument object passed when invoking `command`. */
  args?: Record<string, unknown>;
}

export function isCommandError(value: unknown): value is CommandError {
  if (!value || typeof value !== "object") return false;
  const v = value as Record<string, unknown>;
  return (
    typeof v.code === "string" &&
    typeof v.whatHappened === "string" &&
    typeof v.whyItMatters === "string" &&
    typeof v.whoCausedIt === "string" &&
    Array.isArray(v.actions)
  );
}
