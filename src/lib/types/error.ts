/**
 * Wire shape of every Tauri command's error reject. Mirrors the Rust
 * `AppError` manual `Serialize` impl in `src-tauri/src/error.rs`.
 *
 * The full envelope-rendering component lands in card #4
 * (`P2 — Error envelope renderer`). The type lives here so other modules
 * can be wired against it from day one.
 */
export interface CommandError {
  code: string;
  whatHappened: string;
  whyItMatters: string;
  whoCausedIt: "user" | "system";
  actions: ErrorAction[];
}

export interface ErrorAction {
  label: string;
  /** Frontend command id the button invokes. Undefined means passive hint. */
  command?: string;
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
