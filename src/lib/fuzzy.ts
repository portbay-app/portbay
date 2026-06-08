/**
 * Tiny fuzzy subsequence matcher — the JS stand-in for Lapce's `nucleo`-backed
 * completion filter. Returns the matched character positions so callers can
 * highlight them (like Lapce's completion list), plus a score for ranking.
 *
 * Scoring rewards the things that make a match feel "right": consecutive runs,
 * matches at the start of the string, and matches right after a word boundary
 * (space, `-`, `_`, `.`, `/`). Higher score = better. Returns `null` when not
 * every query character is found in order.
 */

export interface FuzzyMatch {
  score: number;
  /** Indices into `target` of the matched characters, ascending. */
  indices: number[];
}

const BOUNDARY = /[\s\-_./:]/;

export function fuzzyMatch(query: string, target: string): FuzzyMatch | null {
  const q = query.toLowerCase();
  // An empty query matches everything with a neutral score and no highlights.
  if (!q) return { score: 0, indices: [] };
  const t = target.toLowerCase();

  const indices: number[] = [];
  let qi = 0;
  let score = 0;
  let prevMatch = -2;
  let run = 0;

  for (let ti = 0; ti < t.length && qi < q.length; ti++) {
    if (t[ti] !== q[qi]) continue;
    indices.push(ti);
    if (prevMatch === ti - 1) {
      run += 1;
      score += 8 + run * 4; // consecutive chars compound
    } else {
      run = 0;
      score += 4;
    }
    if (ti === 0) score += 12;
    else if (BOUNDARY.test(t[ti - 1])) score += 10; // start of a word
    prevMatch = ti;
    qi += 1;
  }

  if (qi < q.length) return null; // some query chars never matched

  // Prefer matches that start earlier and targets that are tighter to the query.
  score -= indices[0];
  score -= (t.length - q.length) * 0.5;
  return { score, indices };
}
