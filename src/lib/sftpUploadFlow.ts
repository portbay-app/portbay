/**
 * Shared "upload these local paths into that remote directory" flow, used by
 * the Explorer tree, the SFTP file manager, and drag-and-drop. Accepts a mix
 * of files and folders: folders are walked recursively (host-side), their
 * directory skeleton is recreated remotely with `sftp_mkdir`, and every file —
 * loose or nested — goes through the global transfer queue so progress, errors,
 * cancel and resume all surface in the transfers UI.
 *
 * Approval note: callers must make sure the paths are in the backend's
 * approved set first (OS picker / OS drop populate it automatically;
 * renderer-named paths go through `ensureLocalAccess`).
 */
import { localStat, localWalkFiles } from "$lib/deploy";
import { invokeQuiet } from "$lib/ipc";
import { posixJoin, posixParent, posixBasename } from "$lib/sftp";
import { confirmDialog } from "$lib/stores/confirm.svelte";
import { sftpTransfers } from "$lib/stores/sftpTransfers.svelte";
import type { SftpEntry } from "$lib/types/sshTunnels";

/** dataTransfer type for in-app drags of local paths (payload: JSON string[]). */
export const LOCAL_DRAG_MIME = "application/x-portbay-local-paths";

/** One file ready to enqueue. */
export interface PlannedUpload {
  localPath: string;
  remotePath: string;
  name: string;
}

export interface UploadPlan {
  files: PlannedUpload[];
  /** Remote directories to create, shallowest first. */
  dirs: string[];
}

/** Local-path basename — handles both `/` and Windows `\` separators. */
export function localBasename(p: string): string {
  const seg = p.split(/[\\/]/).filter(Boolean);
  return seg.length ? seg[seg.length - 1] : p;
}

/**
 * Expand a mix of local files and folders into a flat upload plan rooted at
 * `remoteDir`. A folder `…/site` uploads as `remoteDir/site/**`.
 */
export async function planUploads(localPaths: string[], remoteDir: string): Promise<UploadPlan> {
  const files: PlannedUpload[] = [];
  const dirSet = new Set<string>();
  for (const local of localPaths) {
    const stat = await localStat(local);
    if (!stat.isDir) {
      files.push({ localPath: local, remotePath: posixJoin(remoteDir, stat.name), name: stat.name });
      continue;
    }
    const base = posixJoin(remoteDir, stat.name);
    dirSet.add(base);
    const walked = await localWalkFiles(local);
    for (const f of walked) {
      // Register every intermediate directory of the relative path.
      const parts = f.rel.split("/");
      let acc = base;
      for (const part of parts.slice(0, -1)) {
        acc = posixJoin(acc, part);
        dirSet.add(acc);
      }
      files.push({
        localPath: f.path,
        remotePath: posixJoin(base, f.rel),
        name: f.rel,
      });
    }
  }
  // Shallowest first so parents exist before children.
  const dirs = [...dirSet].sort((a, b) => a.split("/").length - b.split("/").length);
  return { files, dirs };
}

/**
 * Check every planned destination against the **live** remote listing and ask
 * the user what to do about clashes — the standard SFTP replace prompt.
 * Returns the plan to run (possibly filtered to skip the existing files), or
 * `null` when the user cancels. Destinations whose directory doesn't exist yet
 * (folder uploads create them) can't clash and are skipped cheaply.
 */
export async function confirmReplacements(
  connectionId: string,
  plan: UploadPlan,
): Promise<UploadPlan | null> {
  const byDir = new Map<string, PlannedUpload[]>();
  for (const f of plan.files) {
    const dir = posixParent(f.remotePath);
    const list = byDir.get(dir) ?? [];
    list.push(f);
    byDir.set(dir, list);
  }
  const clashing = new Set<string>(); // remotePath of files that already exist
  for (const [dir, files] of byDir) {
    let existing: SftpEntry[];
    try {
      existing = await invokeQuiet<SftpEntry[]>("sftp_list_dir", {
        input: { connectionId, path: dir },
      });
    } catch {
      continue; // directory doesn't exist yet — nothing to replace
    }
    const names = new Set(existing.filter((e) => !e.isDir).map((e) => e.name));
    for (const f of files) {
      if (names.has(posixBasename(f.remotePath))) clashing.add(f.remotePath);
    }
  }
  if (clashing.size === 0) return plan;

  const shown = [...clashing].slice(0, 6).map((p) => posixBasename(p));
  const more = clashing.size - shown.length;
  const choice = await confirmDialog.open({
    title:
      clashing.size === 1 ? `Replace “${shown[0]}”?` : `Replace ${clashing.size} existing files?`,
    message:
      (clashing.size === 1
        ? "A file with this name already exists in the destination."
        : `Already in the destination: ${shown.join(", ")}${more > 0 ? ` and ${more} more` : ""}.`) +
      " Each replacement is staged and only swapped in once its upload completes — a declined or failed upload leaves the original untouched.",
    destructive: true,
    icon: "file-plus",
    actions: [
      { label: "Replace", value: "replace", tone: "destructive", icon: "file-plus" },
      // Skipping is only meaningful when something would still upload.
      ...(clashing.size < plan.files.length
        ? [{ label: "Skip existing", value: "skip" } as const]
        : []),
    ],
  });
  if (choice === "replace") return plan;
  if (choice === "skip") {
    return { dirs: plan.dirs, files: plan.files.filter((f) => !clashing.has(f.remotePath)) };
  }
  return null;
}

/**
 * The full upload pipeline: plan → confirm replacements → run. Returns `false`
 * when nothing was uploaded (empty pick or user cancel). This is what every
 * upload entry point (picker, drag-and-drop, local pane) should call.
 */
export async function uploadWithConfirm(
  connectionId: string,
  localPaths: string[],
  remoteDir: string,
  onSettled?: () => void,
): Promise<boolean> {
  const planned = await planUploads(localPaths, remoteDir);
  if (planned.files.length === 0 && planned.dirs.length === 0) return false;
  const plan = await confirmReplacements(connectionId, planned);
  if (!plan) return false;
  await runUploadPlan(connectionId, plan, onSettled);
  return true;
}

/**
 * Execute a plan: create the remote directory skeleton (ignoring
 * already-exists errors), then enqueue every file on the transfer queue.
 * `onSettled` fires — debounced — as transfers complete, so callers can
 * refresh a listing once instead of per file. Returns the number of files
 * enqueued.
 */
export async function runUploadPlan(
  connectionId: string,
  plan: UploadPlan,
  onSettled?: () => void,
): Promise<number> {
  for (const dir of plan.dirs) {
    try {
      await invokeQuiet<void>("sftp_mkdir", { input: { connectionId, path: dir } });
    } catch {
      /* most likely "already exists" — uploads into it will fail loudly if not */
    }
  }
  // An empty folder still created directories above — refresh for those.
  if (plan.files.length === 0) {
    if (plan.dirs.length > 0) onSettled?.();
    return 0;
  }
  let timer: ReturnType<typeof setTimeout> | null = null;
  const settle = onSettled
    ? () => {
        if (timer) clearTimeout(timer);
        timer = setTimeout(onSettled, 400);
      }
    : undefined;
  for (const f of plan.files) {
    sftpTransfers.enqueueUpload(connectionId, f.localPath, f.remotePath, f.name, settle);
  }
  return plan.files.length;
}
