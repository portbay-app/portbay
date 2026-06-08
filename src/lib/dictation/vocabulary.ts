/**
 * Surface-context vocabulary for Smart Dictation.
 *
 * The idea (lifted from terminal AI assistants like smart-suggestion, which
 * feed the model the terminal buffer): the text already ON the surface —
 * a pending command, the agent conversation, a card's title — contains the
 * exact jargon the user is about to speak, and speech-to-text reliably
 * mangles exactly those terms ("russ sftp" for `russh-sftp`, "port bay
 * landing" for `portbay-landing`).
 *
 * This extractor pulls technical-shaped tokens out of that ambient text so
 * the rewrite prompt's vocabulary section (a SPELLING reference — "never
 * insert a term that was not spoken", see `push_vocabulary` in
 * src-tauri/src/dictation.rs) can restore their exact spelling. It deals in
 * tokens, not content: nothing from the surface is sent as prose, and the
 * prompt's no-invented-facts rule is unchanged.
 *
 * Order matters: earlier sources rank higher, and the backend keeps surface
 * terms ahead of the global registry vocabulary under the shared cap.
 */

/** Default per-surface term budget — leaves room under the backend's 40-term
 * cap for the registry vocabulary (project names + hostnames). */
const DEFAULT_CAP = 16;

/** Words that pass the shape test but are noise in a spelling reference. */
const STOPLIST = new Set([
  "e.g",
  "i.e",
  "etc",
  "a.m",
  "p.m",
]);

/** Whether one token looks like a technical identifier worth spelling out:
 * paths, flags, dotted/hyphenated/underscored names, camelCase, versions,
 * letter+digit mixes. Plain words are excluded — the model spells those fine. */
function isTechnical(token: string): boolean {
  if (token.length < 3 || token.length > 64) return false;
  if (STOPLIST.has(token.toLowerCase())) return false;
  // Pure numbers (incl. dotted/colon-separated like 8080, 0.1.4 alone) are
  // dictation's job to get right, not a spelling reference's.
  if (/^[\d.,:]+$/.test(token)) return false;
  // Flags: -f, --force, --dry-run.
  if (/^--?[a-z\d][\w-]*$/i.test(token)) return true;
  // Internal structural punctuation: path/host/identifier shapes.
  if (/[a-z\d][-_./:@][a-z\d]/i.test(token)) return true;
  // Letters and digits mixed: qwen2, sha256, b3cf96c-ish.
  if (/[a-z]/i.test(token) && /\d/.test(token)) return true;
  // camelCase / PascalCase with an internal hump.
  if (/[a-z][A-Z]/.test(token)) return true;
  return false;
}

/**
 * Extract technical terms from surface text, earliest source first.
 * Case-insensitive dedupe keeps the first spelling seen. `cap` bounds the
 * result so one chatty surface can't crowd the prompt.
 */
export function extractTechnicalTerms(sources: string[], cap = DEFAULT_CAP): string[] {
  const seen = new Set<string>();
  const terms: string[] = [];
  for (const source of sources) {
    if (terms.length >= cap) break;
    for (const raw of source.split(/\s+/)) {
      if (terms.length >= cap) break;
      // Trim wrapping punctuation (quotes, fences, brackets, sentence ends)
      // while keeping internal structure intact.
      const token = raw.replace(/^[^\w~/@-]+/, "").replace(/[^\w/+~]+$/, "");
      if (!isTechnical(token)) continue;
      const key = token.toLowerCase();
      if (seen.has(key)) continue;
      seen.add(key);
      terms.push(token);
    }
  }
  return terms;
}
