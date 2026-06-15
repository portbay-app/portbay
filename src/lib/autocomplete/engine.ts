/**
 * CompletionEngine — the one "smart" core both inline-completion surfaces share
 * (the editor and the terminal). It owns the three things Tabby and Continue
 * teach you not to reinvent per surface:
 *
 *  • Debounce — wait for a typing pause before asking the model.
 *  • Cancellation — every new request aborts the previous in-flight one
 *    (Tabby's core lesson: one active request, abort on each keystroke), so a
 *    stale completion never lands after the user has moved on.
 *  • Caching + post-processing — a longest-prefix LRU (Continue) serves
 *    accept/retype instantly, and the raw emission is cleaned before display.
 *
 * Surfaces stay dumb: they supply a `CompletionFetcher` and a context, and
 * render whatever string comes back.
 */
import { PrefixCache } from "./cache";
import { postprocess } from "./postprocess";
import type { CompletionContext, CompletionFetcher } from "./types";

export interface EngineOptions {
  fetcher: CompletionFetcher;
  /** Typing-pause before a request fires. Continue defaults to 250ms; we use a
      touch less since the model is one SSH hop away and feels laggier. */
  debounceMs?: number;
  /** Don't request until at least this many non-space chars of prefix exist. */
  minPrefix?: number;
  cacheSize?: number;
}

interface Pending {
  timer: ReturnType<typeof setTimeout>;
  controller: AbortController;
  resolve: (value: string | null) => void;
}

export class CompletionEngine {
  private caches = new Map<string, PrefixCache>();
  private pending: Pending | null = null;

  constructor(private opts: EngineOptions) {}

  private cacheFor(scope: string): PrefixCache {
    let c = this.caches.get(scope);
    if (!c) {
      c = new PrefixCache(this.opts.cacheSize ?? 200);
      this.caches.set(scope, c);
    }
    return c;
  }

  /** Cancel any pending debounce / in-flight request, resolving it to null so
      its awaiter never hangs. */
  cancel(): void {
    const p = this.pending;
    this.pending = null;
    if (p) {
      clearTimeout(p.timer);
      p.controller.abort();
      p.resolve(null);
    }
  }

  /**
   * Request a suggestion for `ctx`. Cache hits resolve immediately (no round
   * trip); otherwise the fetcher fires after the debounce. Resolves to the
   * post-processed ghost text, or null for "show nothing".
   */
  request(ctx: CompletionContext): Promise<string | null> {
    this.cancel();

    const cached = this.cacheFor(ctx.scope).get(ctx.prefix);
    if (cached) {
      return Promise.resolve(
        postprocess({ completion: cached, prefix: ctx.prefix, suffix: ctx.suffix, multiline: ctx.multiline }),
      );
    }
    if (ctx.prefix.replace(/\s+$/, "").length < (this.opts.minPrefix ?? 2)) {
      return Promise.resolve(null);
    }

    return new Promise<string | null>((resolve) => {
      const controller = new AbortController();
      const timer = setTimeout(async () => {
        let raw: string | null = null;
        try {
          raw = await this.opts.fetcher(ctx, controller.signal);
        } catch {
          raw = null;
        }
        // Superseded by a newer request while the fetcher was in flight.
        if (this.pending?.controller !== controller) {
          resolve(null);
          return;
        }
        this.pending = null;
        if (raw == null) {
          resolve(null);
          return;
        }
        this.cacheFor(ctx.scope).set(ctx.prefix, raw);
        resolve(
          postprocess({ completion: raw, prefix: ctx.prefix, suffix: ctx.suffix, multiline: ctx.multiline }),
        );
      }, this.opts.debounceMs ?? 200);

      this.pending = { timer, controller, resolve };
    });
  }

  /** Drop all cached completions. The terminal calls this when its cwd changes,
      since path suggestions are keyed only by the typed prefix and would
      otherwise resolve against the old directory. */
  clearCache(): void {
    this.caches.clear();
  }

  dispose(): void {
    this.cancel();
    this.caches.clear();
  }
}
