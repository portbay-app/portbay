/**
 * listSelection — the shared multi-select model for file listings (the SFTP
 * browser's rows and the Explorer tree). Finder/VS Code semantics:
 *
 *   - plain click          → select just that row (it becomes the anchor)
 *   - ⌘/Ctrl-click          → toggle that row, keep the rest (anchor moves)
 *   - ⇧-click               → select the visible range anchor→row (replacing,
 *                             or adding with ⌘/Ctrl held)
 *   - checkbox              → toggle that row, keep the rest (anchor moves);
 *                             ⇧-checkbox extends the range additively
 *
 * Pure functions over an immutable `Selection` so every interaction sequence
 * is unit-testable; the components hold the returned state in runes.
 */

export interface Selection {
  /** Selected row keys (remote paths). */
  paths: ReadonlySet<string>;
  /** The last plainly-targeted row — the pivot ⇧-click ranges from. */
  anchor: string | null;
}

export const EMPTY_SELECTION: Selection = { paths: new Set(), anchor: null };

/** Plain click: the row becomes the whole selection and the anchor. */
export function plainSelect(path: string): Selection {
  return { paths: new Set([path]), anchor: path };
}

/** ⌘/Ctrl-click or checkbox: toggle the row, keep everything else.
 *  The anchor moves to the row so a following ⇧-click ranges from it. */
export function toggleSelect(sel: Selection, path: string): Selection {
  const next = new Set(sel.paths);
  if (next.has(path)) next.delete(path);
  else next.add(path);
  return { paths: next, anchor: path };
}

/** ⇧-click: select every row between the anchor and `path` as they appear in
 *  `order` (the listing's current render order). `additive` (⌘⇧ / ⇧-checkbox)
 *  keeps the existing selection; otherwise the range replaces it. With no
 *  usable anchor it degrades to selecting just `path`. The anchor stays put so
 *  successive ⇧-clicks re-pivot around it. */
export function rangeSelect(
  sel: Selection,
  order: readonly string[],
  path: string,
  additive: boolean,
): Selection {
  const to = order.indexOf(path);
  if (to < 0) return sel;
  const from = sel.anchor ? order.indexOf(sel.anchor) : -1;
  if (from < 0) {
    return {
      paths: additive ? new Set([...sel.paths, path]) : new Set([path]),
      anchor: path,
    };
  }
  const [a, b] = from <= to ? [from, to] : [to, from];
  const next = additive ? new Set(sel.paths) : new Set<string>();
  for (let i = a; i <= b; i++) next.add(order[i]);
  return { paths: next, anchor: sel.anchor };
}

/** Header checkbox: select all of `order`, or clear if it's all selected. */
export function toggleSelectAll(sel: Selection, order: readonly string[]): Selection {
  const all = order.length > 0 && order.every((p) => sel.paths.has(p));
  if (all) return EMPTY_SELECTION;
  return { paths: new Set(order), anchor: sel.anchor };
}

/** After a refresh: drop selected paths that no longer exist. The anchor is
 *  kept only while its row is still alive. */
export function pruneSelection(sel: Selection, alive: ReadonlySet<string>): Selection {
  const next = new Set([...sel.paths].filter((p) => alive.has(p)));
  if (next.size === sel.paths.size && (sel.anchor === null || alive.has(sel.anchor))) return sel;
  return {
    paths: next,
    anchor: sel.anchor !== null && alive.has(sel.anchor) ? sel.anchor : null,
  };
}
