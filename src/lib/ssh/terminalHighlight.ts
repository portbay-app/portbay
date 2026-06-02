/**
 * terminalHighlight — pure matching logic for user-defined terminal highlight
 * rules. A rule is a pattern (regex or literal) + a colour + a render style;
 * matching text in shell/log output is tinted, underlined, or outlined. This
 * module is free of xterm/DOM imports so it's unit-testable and shared by both
 * the live preview (settings) and the decoration engine that paints the real
 * terminal ({@link ./terminalHighlightEngine}).
 *
 * Overlap policy: each character belongs to at most one rule. Rules earlier in
 * the list win (drag-to-reorder = drag-to-prioritise); ties break toward the
 * longer match, then the earlier start.
 */

/** How a matched span is drawn over the terminal cells. */
export type HighlightRenderMode = "background" | "underline" | "outline";

export interface HighlightRule {
  id: string;
  /** Optional display name, shown in the rule list. */
  label: string;
  /** Regex source, or a literal substring when {@link isRegex} is false. */
  pattern: string;
  /** Treat {@link pattern} as a regular expression (false = literal text). */
  isRegex: boolean;
  /** Match case-sensitively (false = case-insensitive). */
  caseSensitive: boolean;
  /** Highlight colour as `#RRGGBB`. */
  color: string;
  renderMode: HighlightRenderMode;
  enabled: boolean;
}

/** A rule compiled once for repeated matching; `regex` is null on a bad pattern. */
export interface CompiledRule {
  id: string;
  color: string;
  renderMode: HighlightRenderMode;
  regex: RegExp | null;
}

/** A resolved, non-overlapping highlight span over a single line of text. */
export interface HighlightMatch {
  ruleId: string;
  color: string;
  renderMode: HighlightRenderMode;
  /** Inclusive start index into the line string. */
  start: number;
  /** Exclusive end index into the line string. */
  end: number;
}

/** Hard ceiling on rules — keeps the per-line matching pass cheap. */
export const MAX_HIGHLIGHT_RULES = 20;

/** A ready-made rule the user can drop in (so the feature is usable without
 *  hand-writing a regex). The `id` is assigned when added to the store. */
export type HighlightPreset = Omit<HighlightRule, "id" | "enabled">;

/** Curated starter patterns covering the common log/training-output cases. */
export const HIGHLIGHT_PRESETS: HighlightPreset[] = [
  { label: "Errors", pattern: "\\b(error|fatal|fail(ed|ure)?|panic)\\b", isRegex: true, caseSensitive: false, color: "#ef4444", renderMode: "background" },
  { label: "Warnings", pattern: "\\b(warn(ing)?|deprecated)\\b", isRegex: true, caseSensitive: false, color: "#f59e0b", renderMode: "background" },
  { label: "Success", pattern: "\\b(success|done|passed|ready|ok)\\b", isRegex: true, caseSensitive: false, color: "#22c55e", renderMode: "background" },
  { label: "IPv4 address", pattern: "\\b(?:25[0-5]|2[0-4]\\d|1?\\d?\\d)(?:\\.(?:25[0-5]|2[0-4]\\d|1?\\d?\\d)){3}\\b", isRegex: true, caseSensitive: false, color: "#3b82f6", renderMode: "background" },
  { label: "Host:port", pattern: "\\b[\\w.-]+:\\d{2,5}\\b", isRegex: true, caseSensitive: false, color: "#ec4899", renderMode: "background" },
  { label: "URL", pattern: "https?://[^\\s)\\]]+", isRegex: true, caseSensitive: false, color: "#a855f7", renderMode: "underline" },
  { label: "File path", pattern: "(?:/[\\w.-]+)+/?", isRegex: true, caseSensitive: false, color: "#14b8a6", renderMode: "underline" },
  { label: "Timestamp", pattern: "\\b\\d{4}-\\d{2}-\\d{2}[ T]\\d{2}:\\d{2}:\\d{2}\\b", isRegex: true, caseSensitive: false, color: "#64748b", renderMode: "background" },
  { label: "UUID", pattern: "\\b[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}\\b", isRegex: true, caseSensitive: false, color: "#f97316", renderMode: "outline" },
];

/** Per-line match ceiling, so a pathological pattern can't blow up a long line. */
const MAX_MATCHES_PER_LINE = 200;

/** Escape regex metacharacters so a literal rule matches its text verbatim. */
function escapeLiteral(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function buildRegex(rule: HighlightRule): RegExp | null {
  const source = rule.isRegex ? rule.pattern : escapeLiteral(rule.pattern);
  // Global for find-all; `i` unless the rule opts into case sensitivity.
  const flags = rule.caseSensitive ? "g" : "gi";
  try {
    return new RegExp(source, flags);
  } catch {
    return null;
  }
}

/**
 * Compile the enabled, non-empty rules in list order. Invalid regexes compile
 * to a null `regex` (kept in place so the UI can flag them) and are skipped at
 * match time.
 */
export function compileRules(rules: HighlightRule[]): CompiledRule[] {
  return rules
    .filter((r) => r.enabled && r.pattern.trim().length > 0)
    .map((r) => ({
      id: r.id,
      color: r.color,
      renderMode: r.renderMode ?? "background",
      regex: buildRegex(r),
    }));
}

/**
 * Validate a pattern for the settings UI. A literal pattern is always valid; a
 * regex returns the engine's error message when malformed, else null.
 */
export function patternError(pattern: string, isRegex = true): string | null {
  if (!isRegex || !pattern.trim()) return null;
  try {
    new RegExp(pattern);
    return null;
  } catch (e) {
    return e instanceof Error ? e.message : "Invalid regular expression";
  }
}

/**
 * Find the highlight spans for one line of text. Returns non-overlapping matches
 * sorted by start position, applying the earlier-rule-wins overlap policy.
 */
export function matchLine(text: string, rules: CompiledRule[]): HighlightMatch[] {
  if (!text || rules.length === 0) return [];

  // Gather every candidate match, tagged with its rule's list priority.
  const candidates: (HighlightMatch & { priority: number })[] = [];
  rules.forEach((rule, priority) => {
    if (!rule.regex) return;
    rule.regex.lastIndex = 0;
    let m: RegExpExecArray | null;
    let count = 0;
    while ((m = rule.regex.exec(text)) !== null) {
      if (m[0].length === 0) {
        // Zero-width match (e.g. `^`, `\b`) — advance to avoid an infinite loop.
        rule.regex.lastIndex++;
        continue;
      }
      candidates.push({
        ruleId: rule.id,
        color: rule.color,
        renderMode: rule.renderMode,
        start: m.index,
        end: m.index + m[0].length,
        priority,
      });
      if (++count >= MAX_MATCHES_PER_LINE) break;
    }
  });

  if (candidates.length === 0) return [];

  // Higher priority (earlier rule) first; then longer; then earlier start.
  candidates.sort((a, b) => {
    if (a.priority !== b.priority) return a.priority - b.priority;
    const lenA = a.end - a.start;
    const lenB = b.end - b.start;
    if (lenA !== lenB) return lenB - lenA;
    return a.start - b.start;
  });

  // Greedily accept matches that don't overlap an already-accepted span.
  const accepted: HighlightMatch[] = [];
  for (const c of candidates) {
    const clashes = accepted.some((a) => c.start < a.end && c.end > a.start);
    if (clashes) continue;
    accepted.push({
      ruleId: c.ruleId,
      color: c.color,
      renderMode: c.renderMode,
      start: c.start,
      end: c.end,
    });
  }

  accepted.sort((a, b) => a.start - b.start);
  return accepted;
}
