/**
 * sftpGuards — destination-existence guards for every operation that names a
 * target path (rename, move, compress, extract, new file/folder). SFTP and
 * the server-side tools happily clobber — or worse, merge into — existing
 * paths; these helpers make "already exists" an explicit decision, never a
 * silent one.
 */
import { invokeQuiet } from "$lib/ipc";
import { errorBus } from "$lib/stores/errors.svelte";
import type { SftpEntry } from "$lib/types/sshTunnels";

/** Quiet existence probe — absence is the happy path, so no toast. */
export async function remoteExists(
  connectionId: string,
  path: string,
): Promise<SftpEntry | null> {
  try {
    return await invokeQuiet<SftpEntry>("sftp_stat", { input: { connectionId, path } });
  } catch {
    return null;
  }
}

/** "Name taken" toast for the flows that refuse rather than replace
 *  (rename, new file/folder — there's never a legit overwrite intent). */
export function pushNameTaken(name: string, where: string): void {
  errorBus.push({
    code: "SFTP_NAME_TAKEN",
    category: "infrastructure",
    whatHappened: `“${name}” already exists in ${where}.`,
    whyItMatters: "Nothing was changed — pick a different name.",
    whoCausedIt: "user",
    actions: [],
  });
}
