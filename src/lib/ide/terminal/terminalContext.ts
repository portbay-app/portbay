/**
 * Pure helpers for the terminal next-command ghost: reading the xterm buffer
 * tail as model context and scrubbing it. Kept import-light (only an xterm
 * *type* import) so it's unit-testable without SvelteKit's virtual modules.
 */
import type { Terminal } from "@xterm/xterm";

/** Most terminal output lines to feed the model as context (capped for cost +
    to bound what leaves the pane). */
export const MAX_BUFFER_LINES = 15;

/** Leading word / whitespace / punctuation run of `text` (for Cmd/Ctrl+→). */
export function nextWord(text: string): string {
  const m = text.match(/^(\s*[A-Za-z0-9_$.\-/]+|\s*[^A-Za-z0-9_$\s]+|\s+)/);
  return m ? m[0] : text;
}

/** Last `maxLines` rows of the live terminal buffer, as plain text. */
export function readTerminalTail(term: Terminal, maxLines: number): string {
  const buf = term.buffer.active;
  const endRow = buf.baseY + buf.cursorY;
  const out: string[] = [];
  for (let row = endRow; row >= 0 && out.length < maxLines; row--) {
    const line = buf.getLine(row);
    if (!line) continue;
    out.unshift(line.translateToString(true));
  }
  return out.join("\n").replace(/[ \t]+\n/g, "\n").trimEnd();
}

/** Light secret scrub before any buffer text leaves the pane. Not exhaustive —
    a safety net over the on-host transport, paired with the opt-in + cap. */
export function redactSecrets(text: string): string {
  return text
    .replace(
      /((?:token|api[_-]?key|secret|password|passwd|pwd|bearer|auth(?:orization)?)\s*[:=]\s*)\S+/gi,
      "$1***",
    )
    .replace(/\bAKIA[0-9A-Z]{16}\b/g, "AKIA***")
    .replace(/\bgh[pousr]_[A-Za-z0-9]{20,}\b/g, "gh_***")
    .replace(/\bsk-[A-Za-z0-9]{20,}\b/g, "sk-***")
    .replace(/\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\b/g, "jwt***");
}
