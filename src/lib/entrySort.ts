/**
 * entrySort — Finder-style ordering for SFTP listings. Folders always group
 * before files (the dev-tool convention); within each group the active sort
 * key applies, with name as the tiebreak. Pure, so the comparator rules are
 * unit-tested rather than re-derived per view.
 */
import type { SftpEntry } from "$lib/types/sshTunnels";

export type SortKey = "name" | "size" | "mtime";
export type SortDir = "asc" | "desc";

/** Locale-aware, numeric-aware, case-insensitive — "file2" before "file10". */
const byName = (a: SftpEntry, b: SftpEntry) =>
  a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: "base" });

export function sortEntries(
  entries: readonly SftpEntry[],
  key: SortKey,
  dir: SortDir,
): SftpEntry[] {
  const sign = dir === "asc" ? 1 : -1;
  return [...entries].sort((a, b) => {
    // Folders first, regardless of key or direction.
    if (a.isDir !== b.isDir) return a.isDir ? -1 : 1;
    let cmp = 0;
    if (key === "name") {
      cmp = byName(a, b) * sign;
    } else if (key === "size") {
      // Directory sizes are meaningless server-side — folders stay name-sorted.
      cmp = a.isDir ? 0 : (a.size - b.size) * sign;
    } else {
      // Unknown mtimes sort last in either direction (they carry no info).
      const am = a.mtimeSecs;
      const bm = b.mtimeSecs;
      if (am === null && bm === null) cmp = 0;
      else if (am === null) cmp = 1;
      else if (bm === null) cmp = -1;
      else cmp = (am - bm) * sign;
    }
    return cmp !== 0 ? cmp : byName(a, b);
  });
}
