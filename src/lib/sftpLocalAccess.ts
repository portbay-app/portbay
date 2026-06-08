/**
 * Host-mediated approval for renderer-named local paths.
 *
 * The SFTP transfer commands only touch local files whose paths are in the
 * backend's approved set (see `ensure_local_path_approved` in
 * `src-tauri/src/commands/sftp.rs`). OS pickers and real drag-drops populate it
 * automatically; the in-app local file pane and in-app drag-and-drop name paths
 * directly, so they must ask the user first via `sftp_request_local_access` — a
 * native, host-rendered confirm the webview can't click for itself.
 *
 * Grants are session-long on the backend; we mirror them here so a folder the
 * user already allowed never re-prompts (directories cover their subtree).
 */
import { safeInvoke } from "$lib/ipc";

const granted: string[] = [];

function covered(path: string): boolean {
  return granted.some((g) => path === g || path.startsWith(`${g}/`));
}

/**
 * Ensure every path is approved for upload, prompting the user once (native
 * dialog, host-side) for any that aren't yet covered. Returns `false` when the
 * user declines — callers should simply not enqueue anything.
 */
export async function ensureLocalAccess(paths: string[], hostLabel: string): Promise<boolean> {
  const missing = paths.filter((p) => !covered(p));
  if (missing.length === 0) return true;
  const allowed = await safeInvoke<boolean>("sftp_request_local_access", {
    paths: missing,
    hostLabel,
  });
  if (allowed) granted.push(...missing);
  return allowed;
}

/** Record a path as granted without prompting (e.g. it came from an OS picker
    or drag-drop, which the backend already approved host-side). */
export function markLocalAccessGranted(paths: string[]) {
  for (const p of paths) if (!covered(p)) granted.push(p);
}
