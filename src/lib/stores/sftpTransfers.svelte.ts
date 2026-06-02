/**
 * sftpTransfers — a global, concurrency-limited queue for SFTP file transfers.
 *
 * Each job runs through the streaming `sftp_transfer` command (no size ceiling),
 * which emits throttled progress on `portbay://sftp-progress`. The store runs up
 * to `parallel` (1–8) jobs at once — concurrent calls multiplex over the one
 * cached SFTP session, so it's real parallel channels over a single login — and
 * tracks per-job progress for the queue UI. Finished jobs linger until cleared.
 */
import { browser } from "$app/environment";

import { invokeQuiet } from "$lib/ipc";

export type TransferDirection = "upload" | "download";
export type TransferStatus = "pending" | "active" | "done" | "error";

export interface Transfer {
  id: string;
  name: string;
  direction: TransferDirection;
  connectionId: string;
  localPath: string;
  remotePath: string;
  transferred: number;
  total: number;
  status: TransferStatus;
  error?: string;
}

interface ProgressEvent {
  id: string;
  transferred: number;
  total: number;
  done: boolean;
  error: string | null;
}

const DEFAULT_PARALLEL = 4;
const MAX_PARALLEL = 8;

function createSftpTransfersStore() {
  let items = $state<Transfer[]>([]);
  let parallel = $state(DEFAULT_PARALLEL);
  let active = 0;
  let listening = false;
  // Completion callbacks (e.g. refresh the browser), kept off the reactive state.
  const onDone = new Map<string, () => void>();

  function ensureListener() {
    if (!browser || listening) return;
    listening = true;
    void (async () => {
      const { listen } = await import("@tauri-apps/api/event");
      await listen<ProgressEvent>("portbay://sftp-progress", (event) => {
        const p = event.payload;
        const t = items.find((x) => x.id === p.id);
        if (!t) return;
        if (p.total > 0) t.total = p.total;
        t.transferred = p.transferred;
        if (p.error) {
          t.status = "error";
          t.error = p.error;
        }
        // `done` is finalised on the invoke's resolve (below) to avoid a race
        // between the last progress event and the command returning.
      });
    })();
  }

  function pump() {
    for (const t of items) {
      if (active >= parallel) break;
      if (t.status === "pending") run(t);
    }
  }

  function run(t: Transfer) {
    t.status = "active";
    active += 1;
    void invokeQuiet<number>("sftp_transfer", {
      input: {
        connectionId: t.connectionId,
        id: t.id,
        direction: t.direction,
        localPath: t.localPath,
        remotePath: t.remotePath,
      },
    })
      .then(() => {
        if (t.status !== "error") {
          t.status = "done";
          if (t.total > 0) t.transferred = t.total;
        }
      })
      .catch(() => {
        if (t.status !== "error") {
          t.status = "error";
          t.error = t.error ?? "Transfer failed";
        }
      })
      .finally(() => {
        active -= 1;
        const cb = onDone.get(t.id);
        if (cb) {
          onDone.delete(t.id);
          cb();
        }
        pump();
      });
  }

  function enqueue(
    direction: TransferDirection,
    connectionId: string,
    localPath: string,
    remotePath: string,
    name: string,
    done?: () => void,
  ): string {
    ensureListener();
    const id = crypto.randomUUID();
    if (done) onDone.set(id, done);
    items = [
      ...items,
      { id, name, direction, connectionId, localPath, remotePath, transferred: 0, total: 0, status: "pending" },
    ];
    pump();
    return id;
  }

  return {
    get value() {
      return items;
    },
    get parallel() {
      return parallel;
    },
    get activeCount() {
      return items.filter((t) => t.status === "active" || t.status === "pending").length;
    },
    setParallel(n: number) {
      parallel = Math.min(MAX_PARALLEL, Math.max(1, Math.round(n)));
      pump();
    },
    enqueueUpload(connectionId: string, localPath: string, remotePath: string, name: string, done?: () => void) {
      return enqueue("upload", connectionId, localPath, remotePath, name, done);
    },
    enqueueDownload(connectionId: string, remotePath: string, localPath: string, name: string, done?: () => void) {
      return enqueue("download", connectionId, localPath, remotePath, name, done);
    },
    /** Drop finished (done/error) jobs from the list. */
    clearFinished() {
      items = items.filter((t) => t.status === "pending" || t.status === "active");
    },
  };
}

export const sftpTransfers = createSftpTransfersStore();
