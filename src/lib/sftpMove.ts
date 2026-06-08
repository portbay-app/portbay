/**
 * sftpMove — planning for drag-to-move inside the remote browser: dropping
 * entries onto a folder moves them there (server-side rename). The planner is
 * pure so the safety rules — no folder into itself or its own subtree, no
 * no-op moves — are unit-tested; the component just executes the plan.
 */
import { posixBasename, posixJoin, posixParent } from "$lib/posixPath";

export interface MovePlan {
  from: string;
  to: string;
}

/**
 * Plan moving `sources` into `destDir`. Filters out:
 *  - a path dropped onto itself,
 *  - paths already directly inside `destDir` (no-op),
 *  - `destDir` being the source or inside the source's subtree (a folder can
 *    never move into itself or a descendant — the classic data-loss footgun).
 * Duplicate sources are deduped; order is preserved.
 */
/** Partition a move plan by whether each destination already exists — the
 *  conflicted half needs an explicit Replace / Skip decision from the user. */
export function splitMoveConflicts(
  plan: readonly MovePlan[],
  existing: ReadonlySet<string>,
): { clean: MovePlan[]; conflicted: MovePlan[] } {
  const clean: MovePlan[] = [];
  const conflicted: MovePlan[] = [];
  for (const m of plan) (existing.has(m.to) ? conflicted : clean).push(m);
  return { clean, conflicted };
}

export function planMoves(sources: readonly string[], destDir: string): MovePlan[] {
  const out: MovePlan[] = [];
  const seen = new Set<string>();
  for (const raw of sources) {
    const src = raw.replace(/\/+$/, "");
    if (!src || src === "/" || seen.has(src)) continue;
    seen.add(src);
    if (src === destDir) continue; // dropped onto itself
    if (posixParent(src) === destDir) continue; // already there
    if (destDir === src || destDir.startsWith(`${src}/`)) continue; // own subtree
    out.push({ from: src, to: posixJoin(destDir, posixBasename(src)) });
  }
  return out;
}
