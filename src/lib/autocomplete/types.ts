/**
 * Shared inline-completion types. The engine is framework-agnostic so the same
 * caching / cancellation / post-processing serves both the CodeMirror editor and
 * the terminal next-command hint — only the `CompletionFetcher` differs per
 * surface (see `$lib/ssh/complete`).
 */

export interface CompletionContext {
  /** Stable per-document/surface key, so cache + in-flight state don't bleed
      across files or panes. */
  scope: string;
  /** Text before the cursor (the FIM `prompt`). */
  prefix: string;
  /** Text after the cursor (the FIM `suffix`); empty for single-line surfaces. */
  suffix: string;
  /** Single-line surfaces (the terminal command line) cap the hint to one line. */
  multiline: boolean;
}

/**
 * Produces a raw (un-post-processed) completion for a context. Must honour
 * `signal` — abort the underlying request when it fires. Returns `null` for "no
 * suggestion" or any failure (failures are silent: no ghost, never a toast).
 */
export type CompletionFetcher = (
  ctx: CompletionContext,
  signal: AbortSignal,
) => Promise<string | null>;
