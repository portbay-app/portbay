/**
 * Completion post-processing — the cleanup pass that turns a raw model emission
 * into something safe to show as ghost text. The transforms (and their order)
 * are ported from Continue's `core/autocomplete/postprocessing`, which exist to
 * defang exactly the junk a small, host-run code model produces: echoing the
 * current line, runaway repetition, wrapping output in code fences, leading
 * whitespace dupes. Each returns `null` to mean "drop this completion".
 */

export interface PostprocessInput {
  completion: string;
  prefix: string;
  suffix: string;
  /** Single-line surfaces keep only the first line of the completion. */
  multiline: boolean;
}

/** Strip a leading ```lang fence and a trailing ``` the model sometimes adds. */
function removeBackticks(text: string): string {
  let t = text;
  const fenceStart = t.match(/^\s*```[a-zA-Z0-9_-]*\n/);
  if (fenceStart) t = t.slice(fenceStart[0].length);
  t = t.replace(/\n?```\s*$/, "");
  return t;
}

/** The completion just re-emits the line the cursor is already on. */
function rewritesLineAbove(text: string, prefix: string): boolean {
  const prefixLines = prefix.split("\n");
  const currentLine = prefixLines[prefixLines.length - 1];
  if (!currentLine.trim()) return false;
  const firstCompletionLine = text.split("\n")[0];
  return firstCompletionLine.trim() === currentLine.trim() && firstCompletionLine.trim().length > 0;
}

/** A short fragment repeated across many lines / most of the output — the
    classic small-model degeneration (Continue's `isExtremeRepetition`). */
function isExtremeRepetition(text: string): boolean {
  const lines = text.split("\n").filter((l) => l.trim().length > 0);
  if (lines.length < 6) return false;
  const counts = new Map<string, number>();
  for (const l of lines) counts.set(l.trim(), (counts.get(l.trim()) ?? 0) + 1);
  const maxRepeat = Math.max(...counts.values());
  return maxRepeat / lines.length > 0.8;
}

/** When the completion runs into what's already after the cursor, cut it there
    so we don't duplicate the suffix. Two cases: the completion runs *into* the
    suffix mid-stream, and the common one where it re-emits closing
    brackets/punctuation the suffix already supplies. */
function stopAtSuffix(text: string, suffix: string): string {
  let out = text;
  const sfx = suffix.replace(/^\s+/, "");

  const probe = sfx.slice(0, 16);
  if (probe.length >= 4) {
    const idx = out.indexOf(probe);
    if (idx > 0) out = out.slice(0, idx);
  }

  // Trailing duplicate of the suffix's opening — only strip when it's purely
  // closing brackets / punctuation / whitespace, so we never eat an identifier.
  for (let k = Math.min(out.length, sfx.length); k > 0; k--) {
    const tail = out.slice(out.length - k);
    if (sfx.startsWith(tail) && /^[)\]}>;,\s]+$/.test(tail)) {
      out = out.slice(0, out.length - k);
      break;
    }
  }
  return out;
}

/** Returns the ghost text to show, or `null` to suppress the suggestion. */
export function postprocess({ completion, prefix, suffix, multiline }: PostprocessInput): string | null {
  if (!completion) return null;
  let text = removeBackticks(completion);
  if (text.trim().length === 0) return null;

  if (!multiline) {
    const nl = text.indexOf("\n");
    if (nl !== -1) text = text.slice(0, nl);
  } else if (isExtremeRepetition(text)) {
    return null;
  }

  // Drop a leading space the prefix already supplies.
  if (/[^\S\n]$/.test(prefix) && text.startsWith(" ")) {
    text = text.replace(/^ +/, "");
  }

  if (rewritesLineAbove(text, prefix)) return null;

  text = stopAtSuffix(text, suffix);

  // Trailing whitespace adds nothing as a ghost and reads as a dangling caret.
  text = text.replace(/[ \t]+$/g, "");
  if (text.length === 0 || text.trim().length === 0) return null;
  return text;
}
