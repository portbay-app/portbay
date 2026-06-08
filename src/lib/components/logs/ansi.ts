import Convert from "ansi-to-html";

/** Theme-mapped ANSI palette shared by every converter in the app. */
const CONVERT_OPTIONS = {
  escapeXML: true,
  fg: "var(--color-fg-muted)",
  bg: "transparent",
  colors: {
    0: "var(--color-fg-subtle)",
    1: "var(--color-status-crashed)",
    2: "var(--color-status-running)",
    3: "var(--color-status-unhealthy)",
    4: "var(--color-accent)",
    5: "#c084fc",
    6: "var(--color-status-starting)",
    7: "var(--color-fg-muted)",
    8: "var(--color-fg-subtle)",
    9: "var(--color-status-crashed)",
    10: "var(--color-status-running)",
    11: "var(--color-status-unhealthy)",
    12: "var(--color-accent-hover)",
    13: "#d946ef",
    14: "var(--color-status-starting)",
    15: "var(--color-fg)",
  },
};

const converter = new Convert(CONVERT_OPTIONS);

/**
 * Incremental ANSI → HTML renderer for a growing output string (a streaming
 * deploy step). Each `push(full)` converts only the appended suffix with a
 * stateful converter (so a colour code that opened in an earlier chunk still
 * applies), and returns the accumulated HTML. A shrinking input (output
 * trimmed or replaced wholesale) resets and reconverts.
 */
export class AnsiAppender {
  private conv = new Convert({ ...CONVERT_OPTIONS, stream: true });
  private seen = "";
  private html = "";

  push(full: string): string {
    if (!full.startsWith(this.seen)) {
      this.conv = new Convert({ ...CONVERT_OPTIONS, stream: true });
      this.seen = "";
      this.html = "";
    }
    if (full.length > this.seen.length) {
      this.html += this.conv.toHtml(full.slice(this.seen.length));
      this.seen = full;
    }
    return this.html;
  }
}

/**
 * Severity used for per-line color coding. `info` is the uncoloured default.
 * `system` is reserved for PortBay-authored lifecycle lines (Starting, the
 * command echo, etc.) so they read as distinct from the process's own output.
 */
export type LogLevel = "error" | "warn" | "debug" | "info" | "system";

export interface LogLine {
  /** Severity for color coding. */
  level: LogLevel;
  /**
   * Plain message text — JSON envelope unwrapped, ANSI stripped. Used for
   * search and copy so neither matches the machine wrapper or escape codes.
   */
  text: string;
  /** ANSI-rendered HTML of the message, for display via `{@html}`. */
  html: string;
}

// Matches a CSI ANSI escape sequence (colour codes, cursor moves). Used to
// strip codes out of the plain-text projection.
// eslint-disable-next-line no-control-regex
const ANSI_RE = /\[[0-9;?]*[A-Za-z]/g;

// Error / warning markers tools actually print. Kept deliberately specific —
// matching the bare word "error" would paint lines like "0 errors" red. We key
// on the uppercase level tokens pnpm/turbo/npm emit and the common `level:` /
// `[level]` prefixes instead.
const ERROR_RE =
  /(^|\s)(ERROR|FATAL|PANIC)(\s|$)|\berror[:!]|\[error\]|\belifecycle\b|npm err!|\bcommand failed\b/i;
const WARN_RE =
  /(^|\s)(WARN|WARNING)(\s|$)|\bwarn(ing)?[:!]|\[warn(ing)?\]|\bdeprecated\b|unsupported engine/i;

/**
 * Process Compose wraps each captured output line in a JSON envelope:
 *   {"level":"info","process":"web","replica":0,"message":"> next dev"}
 * Unwrap to the human `message`. Lines without a `message` (PC's blank
 * separators) become empty lines. Anything that isn't PC's envelope — a plain
 * log line, or an unwrapped REST tail — passes through verbatim.
 */
function unwrap(raw: string): { message: string; jsonLevel: string } {
  const trimmed = raw.trimStart();
  if (trimmed.startsWith("{") && trimmed.includes('"process"')) {
    try {
      const obj = JSON.parse(trimmed);
      if (obj && typeof obj === "object" && "process" in obj) {
        return {
          message: typeof obj.message === "string" ? obj.message : "",
          jsonLevel: typeof obj.level === "string" ? obj.level : "",
        };
      }
    } catch {
      /* not valid JSON after all — fall through to plain passthrough */
    }
  }
  return { message: raw, jsonLevel: "" };
}

/**
 * Decide a line's severity. Process Compose stamps most captured output as
 * `info` regardless of what the tool meant, so the message text is the more
 * reliable signal — we trust an explicit error level, then sniff the message.
 */
function detectLevel(message: string, jsonLevel: string): LogLevel {
  const lvl = jsonLevel.toLowerCase();
  if (lvl === "error" || lvl === "fatal" || lvl === "panic") return "error";
  if (ERROR_RE.test(message)) return "error";
  if (WARN_RE.test(message)) return "warn";
  if (lvl === "warn" || lvl === "warning") return "warn";
  if (lvl === "debug" || lvl === "trace") return "debug";
  return "info";
}

/** Parse one raw log line into a displayable, searchable, colour-tagged line. */
export function parseLogLine(raw: string): LogLine {
  const { message, jsonLevel } = unwrap(raw);
  return {
    level: detectLevel(message, jsonLevel),
    text: message.replace(ANSI_RE, ""),
    html: converter.toHtml(message || " "),
  };
}

/**
 * Build a displayable line from a PortBay-authored message (not Process
 * Compose output), bypassing the JSON-envelope unwrap. `level` defaults to
 * `system`; lifecycle errors/warnings pass their own level so they colour and
 * filter like real log severities. ANSI is stripped from the plain projection
 * and rendered to HTML so an emoji/marker prefix shows correctly.
 */
export function eventLogLine(message: string, level: LogLevel = "system"): LogLine {
  return {
    level,
    text: message.replace(ANSI_RE, ""),
    html: converter.toHtml(message || " "),
  };
}

/** Tailwind text-colour class for a level. `info` returns "" (inherits). */
export function levelClass(level: LogLevel): string {
  switch (level) {
    case "error":
      return "text-status-crashed";
    case "warn":
      return "text-status-unhealthy";
    case "debug":
      return "text-fg-subtle";
    case "system":
      return "text-accent";
    default:
      return "";
  }
}
