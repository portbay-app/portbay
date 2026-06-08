/**
 * Longest-prefix LRU cache for completions — ported from Continue's
 * `AutocompleteLruCache` (and the same idea as Tabby's KV cache). The win: when
 * the user accepts part of a suggestion, dismisses and retypes, or types a
 * character that the previous suggestion already predicted, we serve the
 * remaining completion instantly with no model round-trip.
 *
 * Lookup is "longest cached prefix the query starts with, whose stored
 * completion still agrees with what was typed since" — then return the
 * not-yet-typed remainder.
 */
export class PrefixCache {
  // Insertion-ordered (Map preserves order) → front = oldest for LRU eviction.
  private map = new Map<string, string>();

  constructor(private max = 200) {}

  set(prefix: string, completion: string): void {
    if (!completion) return;
    this.map.delete(prefix);
    this.map.set(prefix, completion);
    if (this.map.size > this.max) {
      const oldest = this.map.keys().next().value;
      if (oldest !== undefined) this.map.delete(oldest);
    }
  }

  /** The completion remaining for `prefix`, or null. */
  get(prefix: string): string | null {
    const exact = this.map.get(prefix);
    if (exact !== undefined) {
      this.touch(prefix, exact);
      return exact;
    }
    let best: string | null = null;
    let bestKeyLen = -1;
    for (const [key, comp] of this.map) {
      if (key.length <= bestKeyLen || key.length >= prefix.length) continue;
      if (!prefix.startsWith(key)) continue;
      const typedSince = prefix.slice(key.length);
      // The chars typed since must be exactly what this completion predicted,
      // and there must be something left to suggest.
      if (comp.startsWith(typedSince) && typedSince.length < comp.length) {
        best = comp.slice(typedSince.length);
        bestKeyLen = key.length;
      }
    }
    return best;
  }

  private touch(key: string, value: string): void {
    this.map.delete(key);
    this.map.set(key, value);
  }

  clear(): void {
    this.map.clear();
  }
}
