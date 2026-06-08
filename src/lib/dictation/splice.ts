/**
 * Smart Dictation — pure string utilities for the snapshot/diff/splice
 * mechanism.
 *
 * macOS dictation types straight into the focused field, so the "raw
 * transcript" is recovered by diffing the field value around the session:
 * snapshot at start, diff at stop, rewrite only the inserted segment, splice
 * the result back. Pure functions, unit-tested in
 * `src/lib/__tests__/dictationSplice.test.ts`.
 */

/** The dictated segment located inside the post-session field value. */
export interface Insertion {
  /** Chars of `after` before the inserted segment (common prefix). */
  prefixLen: number;
  /** Chars of `after` after the inserted segment (common suffix). */
  suffixLen: number;
  /** The inserted text itself: `after.slice(prefixLen, after.length - suffixLen)`. */
  inserted: string;
}

/**
 * Locate what a dictation session inserted by diffing the field value before
 * and after (longest common prefix + suffix). Returns null when nothing was
 * inserted — unchanged field, pure deletion, or empty insertion — i.e. when
 * there is nothing to rewrite.
 *
 * The user may also have typed/corrected manually during the session; that
 * lands inside the inserted window too, which is fine — it's still "what this
 * session produced".
 */
export function extractInsertion(before: string, after: string): Insertion | null {
  if (before === after) return null;

  let prefixLen = 0;
  const maxPrefix = Math.min(before.length, after.length);
  while (prefixLen < maxPrefix && before[prefixLen] === after[prefixLen]) prefixLen++;

  let suffixLen = 0;
  // Suffix must not overlap the prefix on either string (repeated text).
  const maxSuffix = Math.min(before.length, after.length) - prefixLen;
  while (
    suffixLen < maxSuffix &&
    before[before.length - 1 - suffixLen] === after[after.length - 1 - suffixLen]
  ) {
    suffixLen++;
  }

  const inserted = after.slice(prefixLen, after.length - suffixLen);
  if (inserted.length === 0) return null;
  return { prefixLen, suffixLen, inserted };
}

/**
 * Is the inserted segment worth a model round-trip? Short utterances ("yes",
 * "ok", "done") gain nothing and risk mangling, so they stay raw. The bar is
 * deliberately low — three words — because even "fix login bug" benefits from
 * smart-mode shaping on a task card.
 */
export function worthRewriting(inserted: string): boolean {
  const trimmed = inserted.trim();
  return trimmed.length >= 12 && trimmed.split(/\s+/).length >= 3;
}

/** Whether a space is needed between two non-empty joints when splicing.
 * Exported for the local-engine transcript insert (rewriter `insert()`),
 * which splices at the captured caret with the same spacing repair. */
export function needsSpace(left: string, right: string): boolean {
  if (!left || !right) return false;
  const a = left[left.length - 1];
  const b = right[0];
  if (/\s/.test(a) || /\s/.test(b)) return false;
  // No space before closing punctuation, after an opener, or around path-ish
  // joints the model may have legitimately tightened ("/", "-", "_").
  if (/[.,;:!?)\]}/_-]/.test(b)) return false;
  if (/[([{/_-]/.test(a)) return false;
  return true;
}

/**
 * Replace the inserted segment of `current` with `rewritten`, repairing
 * spacing at both joints (the model trims its output, so a mid-text splice
 * could otherwise weld words together).
 */
export function spliceRewrite(current: string, ins: Insertion, rewritten: string): string {
  const prefix = current.slice(0, ins.prefixLen);
  const suffix = current.slice(current.length - ins.suffixLen);
  const left = needsSpace(prefix, rewritten) ? " " : "";
  const right = needsSpace(rewritten, suffix) ? " " : "";
  return prefix + left + rewritten + right + suffix;
}

/**
 * Re-locate the inserted segment after the user edited the field while the
 * rewrite was in flight. Only succeeds when the raw segment still exists
 * exactly once — anything else is ambiguous and the caller must keep the
 * user's text untouched.
 */
export function relocateInsertion(current: string, inserted: string): Insertion | null {
  const first = current.indexOf(inserted);
  if (first === -1) return null;
  if (current.indexOf(inserted, first + 1) !== -1) return null; // ambiguous
  return {
    prefixLen: first,
    suffixLen: current.length - first - inserted.length,
    inserted,
  };
}
