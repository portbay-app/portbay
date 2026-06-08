/**
 * Terminal scrollback as dictation vocabulary — the reader registry.
 *
 * The SSH workspace's terminal panes hold exactly the identifiers the user
 * dictates about (hostnames, services, paths, container names) — the richest
 * jargon source any surface has, and the one ASR mangles hardest. The agent
 * composer and command gate live in a sibling component tree, so panes
 * register a bounded buffer reader here and the dictation vocabulary harvest
 * reads through the connection id.
 *
 * Privacy posture matches the rest of the surface-vocabulary mechanism: the
 * text is reduced to technical-shaped TOKENS by `extractTechnicalTerms`
 * before it goes anywhere near a prompt (spelling reference only — "never
 * insert a term that was not spoken"), the model is local, and the
 * unanchored-injection guard rejects outputs using terms that weren't
 * plausibly spoken. Secrets are scrubbed centrally here as well — a token on
 * screen (API key, bearer header) is exactly letter+digit-mixed enough to
 * pass the technical-shape test otherwise.
 */
import { redactSecrets } from "$lib/ide/terminal/terminalContext";

/** How many trailing buffer rows one pane contributes (the card's "visible/
 * recent" bound — roughly one screen plus a little history). */
export const SCROLLBACK_VOCAB_LINES = 50;

interface ScrollbackReader {
  /** Plain-text tail of the pane's buffer (≤ SCROLLBACK_VOCAB_LINES rows). */
  read: () => string;
  /** Whether this pane is the active/visible one — its text leads. */
  isActive: () => boolean;
}

const readers = new Map<string, Set<ScrollbackReader>>();

/** Register one pane's buffer reader. Returns the unregister handle (call on
 * pane teardown — a dead xterm must never be read). */
export function registerScrollbackReader(
  connectionId: string,
  reader: ScrollbackReader,
): () => void {
  let set = readers.get(connectionId);
  if (!set) {
    set = new Set();
    readers.set(connectionId, set);
  }
  set.add(reader);
  return () => {
    set.delete(reader);
    if (set.size === 0) readers.delete(connectionId);
  };
}

/**
 * The connection's recent terminal text, active pane first, secrets scrubbed.
 * Empty string when no terminal is open — the harvest degrades to the other
 * sources. Reads are cheap (xterm buffer rows), and the caller's extractor
 * cap bounds what can reach the prompt regardless of how much text panes
 * return.
 */
export function readScrollback(connectionId: string): string {
  const set = readers.get(connectionId);
  if (!set || set.size === 0) return "";
  const panes = [...set].sort((a, b) => Number(b.isActive()) - Number(a.isActive()));
  const parts: string[] = [];
  for (const pane of panes) {
    try {
      const text = pane.read();
      if (text) parts.push(text);
    } catch {
      // A pane mid-teardown (disposed xterm) reads as absent, never throws
      // into the dictation path.
    }
  }
  return redactSecrets(parts.join("\n"));
}
