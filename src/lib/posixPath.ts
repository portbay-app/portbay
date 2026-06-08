/**
 * posixPath — pure POSIX path helpers for remote (server-side) paths. Split
 * from $lib/sftp so logic modules (move planning, sorting, tests) can import
 * them without dragging in the IPC layer.
 */

/** Join a POSIX directory + child name (the remote side is always POSIX). */
export function posixJoin(dir: string, name: string): string {
  if (dir === "" || dir === "/") return `/${name}`.replace("//", "/");
  return `${dir.replace(/\/+$/, "")}/${name}`;
}

/** Parent of a POSIX path (`/a/b/c` → `/a/b`; `/a` → `/`). */
export function posixParent(path: string): string {
  const trimmed = path.replace(/\/+$/, "");
  const idx = trimmed.lastIndexOf("/");
  if (idx <= 0) return "/";
  return trimmed.slice(0, idx);
}

/** The last segment of a POSIX path. */
export function posixBasename(path: string): string {
  const trimmed = path.replace(/\/+$/, "");
  const idx = trimmed.lastIndexOf("/");
  return idx === -1 ? trimmed : trimmed.slice(idx + 1);
}
