/**
 * Typed wrappers over the SFTP file-manager IPC commands. Every call goes
 * through `safeInvoke`, so failures surface as toasts and reject; callers
 * usually `await` and refresh on success, letting the toast report errors.
 */
import { safeInvoke } from "$lib/ipc";
import type { SftpEntry } from "$lib/types/sshTunnels";

/** Canonical home/default directory for the connection. */
export function sftpHomeDir(connectionId: string): Promise<string> {
  return safeInvoke<string>("sftp_home_dir", { connectionId });
}

/** List a remote directory (dirs first, then name). */
export function sftpListDir(connectionId: string, path: string): Promise<SftpEntry[]> {
  return safeInvoke<SftpEntry[]>("sftp_list_dir", { input: { connectionId, path } });
}

export function sftpStat(connectionId: string, path: string): Promise<SftpEntry> {
  return safeInvoke<SftpEntry>("sftp_stat", { input: { connectionId, path } });
}

export function sftpMkdir(connectionId: string, path: string): Promise<void> {
  return safeInvoke<void>("sftp_mkdir", { input: { connectionId, path } });
}

export function sftpRename(connectionId: string, from: string, to: string): Promise<void> {
  return safeInvoke<void>("sftp_rename", { input: { connectionId, from, to } });
}

export function sftpRemoveFile(connectionId: string, path: string): Promise<void> {
  return safeInvoke<void>("sftp_remove_file", { input: { connectionId, path } });
}

export function sftpRemoveDir(connectionId: string, path: string): Promise<void> {
  return safeInvoke<void>("sftp_remove_dir", { input: { connectionId, path } });
}

export function sftpChmod(connectionId: string, path: string, mode: number): Promise<void> {
  return safeInvoke<void>("sftp_chmod", { input: { connectionId, path, mode } });
}

export function sftpReadText(connectionId: string, path: string): Promise<string> {
  return safeInvoke<string>("sftp_read_text", { input: { connectionId, path } });
}

/** A remote-file preview: an image (base64), decoded text, or an opaque binary. */
export interface SftpPreview {
  kind: "image" | "text" | "binary";
  mime: string | null;
  base64: string | null;
  text: string | null;
  size: number;
}

/** Read a remote file for preview (image → base64, text → string, else binary). */
export function sftpReadPreview(connectionId: string, path: string): Promise<SftpPreview> {
  return safeInvoke<SftpPreview>("sftp_read_preview", { input: { connectionId, path } });
}

export function sftpWriteText(connectionId: string, path: string, contents: string): Promise<void> {
  return safeInvoke<void>("sftp_write_text", { input: { connectionId, path, contents } });
}

/** Upload a local file to a remote path. Resolves with bytes written. */
export function sftpUpload(connectionId: string, localPath: string, remotePath: string): Promise<number> {
  return safeInvoke<number>("sftp_upload", { input: { connectionId, localPath, remotePath } });
}

/** Download a remote file to a local path. Resolves with bytes written. */
export function sftpDownload(connectionId: string, remotePath: string, localPath: string): Promise<number> {
  return safeInvoke<number>("sftp_download", { input: { connectionId, remotePath, localPath } });
}

export function sftpDisconnect(connectionId: string): Promise<void> {
  return safeInvoke<void>("sftp_disconnect", { connectionId });
}

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

/** Render mode bits as an `rwxr-xr-x`-style string (perms only, no type). */
export function formatMode(mode: number | null): string {
  if (mode === null) return "—";
  const bits = mode & 0o777;
  const rwx = (n: number) =>
    `${n & 4 ? "r" : "-"}${n & 2 ? "w" : "-"}${n & 1 ? "x" : "-"}`;
  return `${rwx((bits >> 6) & 7)}${rwx((bits >> 3) & 7)}${rwx(bits & 7)}`;
}

/** Human-readable byte size. */
export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let n = bytes / 1024;
  let i = 0;
  while (n >= 1024 && i < units.length - 1) {
    n /= 1024;
    i += 1;
  }
  return `${n.toFixed(n < 10 ? 1 : 0)} ${units[i]}`;
}
