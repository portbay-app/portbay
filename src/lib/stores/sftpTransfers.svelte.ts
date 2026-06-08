/**
 * sftpTransfers — a global, concurrency-limited queue for SFTP file transfers.
 *
 * Each job runs through the streaming `sftp_transfer` command (no size ceiling),
 * which emits throttled progress on `portbay://sftp-progress`. The store runs up
 * to `parallel` (1–8) jobs at once — concurrent calls multiplex over the one
 * cached SFTP session, so it's real parallel channels over a single login — and
 * tracks per-job progress, throughput, and ETA for the queue UI.
 *
 * Transfers can be **cancelled** mid-flight (the backend stops at a chunk
 * boundary, leaving the partial file in place) and then **resumed** from the byte
 * offset already moved, or **retried** from scratch. Checkpoints and datasets are
 * tens of GB, so a dropped connection shouldn't mean starting over. Finished jobs
 * linger until cleared.
 */
import { browser } from "$app/environment";

import { invokeQuiet } from "$lib/ipc";

export type TransferDirection = "upload" | "download";
export type TransferStatus = "pending" | "active" | "paused" | "done" | "error";

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
  /** Byte offset the next run starts from (0 = fresh). Set when paused/resumed. */
  offset: number;
  /** Smoothed throughput in bytes/sec while active (0 otherwise). */
  speedBps: number;
  /** Estimated seconds remaining, or null when unknown / not active. */
  etaSecs: number | null;
  error?: string;
}

interface ProgressEvent {
  id: string;
  transferred: number;
  total: number;
  done: boolean;
  paused: boolean;
  error: string | null;
}

const DEFAULT_PARALLEL = 4;
const MAX_PARALLEL = 8;
// Re-sample throughput at most this often, so a burst of progress events doesn't
// produce a jittery speed readout.
const SAMPLE_MS = 400;

function createSftpTransfersStore() {
  let items = $state<Transfer[]>([]);
  let parallel = $state(DEFAULT_PARALLEL);
  let active = 0;
  let listening = false;
  // Completion callbacks (e.g. refresh the browser), kept off the reactive state.
  const onDone = new Map<string, () => void>();
  // Cancel intent per id — the authoritative signal for the resolve handler, so
  // a cancelled transfer is marked paused (not done) regardless of event timing.
  const cancelIntent = new Set<string>();
  // Throughput sampling per id: last (bytes, timestamp) we measured speed from.
  const sample = new Map<string, { bytes: number; at: number }>();

  function find(id: string): Transfer | undefined {
    return items.find((x) => x.id === id);
  }

  function ensureListener() {
    if (!browser || listening) return;
    listening = true;
    void (async () => {
      const { listen } = await import("@tauri-apps/api/event");
      await listen<ProgressEvent>("portbay://sftp-progress", (event) => {
        const p = event.payload;
        const t = find(p.id);
        if (!t) return;
        if (p.total > 0) t.total = p.total;
        t.transferred = p.transferred;
        updateSpeed(t);
        if (p.error) {
          t.status = "error";
          t.error = p.error;
          t.speedBps = 0;
          t.etaSecs = null;
        } else if (p.paused) {
          // Backend stopped on a cancel — transferred is the resume offset.
          t.status = "paused";
          t.offset = p.transferred;
          t.speedBps = 0;
          t.etaSecs = null;
        }
        // `done` is finalised on the invoke's resolve (below) to avoid a race
        // between the last progress event and the command returning.
      });
    })();
  }

  // Smoothed throughput + ETA from successive progress samples (EMA so the
  // readout is steady rather than per-chunk jittery).
  function updateSpeed(t: Transfer) {
    const now = Date.now();
    const prev = sample.get(t.id);
    if (!prev) {
      sample.set(t.id, { bytes: t.transferred, at: now });
      return;
    }
    const dt = now - prev.at;
    if (dt < SAMPLE_MS) return;
    const inst = Math.max(0, ((t.transferred - prev.bytes) / dt) * 1000);
    t.speedBps = t.speedBps > 0 ? t.speedBps * 0.6 + inst * 0.4 : inst;
    t.etaSecs =
      t.speedBps > 0 && t.total > t.transferred
        ? (t.total - t.transferred) / t.speedBps
        : null;
    sample.set(t.id, { bytes: t.transferred, at: now });
  }

  function pump() {
    for (const t of items) {
      if (active >= parallel) break;
      if (t.status === "pending") run(t);
    }
  }

  function run(t: Transfer) {
    t.status = "active";
    t.error = undefined;
    active += 1;
    sample.delete(t.id);
    void invokeQuiet<number>("sftp_transfer", {
      input: {
        connectionId: t.connectionId,
        id: t.id,
        direction: t.direction,
        localPath: t.localPath,
        remotePath: t.remotePath,
        offset: t.offset,
      },
    })
      .then(() => {
        // A cancelled transfer resolves Ok with the partial count — the cancel
        // intent (not the byte count) tells us it paused rather than completed.
        if (cancelIntent.has(t.id)) {
          cancelIntent.delete(t.id);
          if (t.status !== "error") {
            t.status = "paused";
            t.offset = t.transferred;
          }
        } else if (t.status !== "error" && t.status !== "paused") {
          t.status = "done";
          if (t.total > 0) t.transferred = t.total;
        }
      })
      .catch((e: unknown) => {
        if (t.status !== "error") {
          t.status = "error";
          // Surface the backend's CommandError message (e.g. "path … was not
          // chosen through a file dialog") instead of a generic label.
          const msg =
            e && typeof e === "object" && "whatHappened" in e
              ? String((e as { whatHappened: unknown }).whatHappened)
              : null;
          t.error = t.error ?? msg ?? "Transfer failed";
        }
      })
      .finally(() => {
        active -= 1;
        t.speedBps = 0;
        t.etaSecs = null;
        // Run the completion callback only on a clean finish, not a pause.
        if (t.status === "done") {
          const cb = onDone.get(t.id);
          if (cb) {
            onDone.delete(t.id);
            cb();
          }
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
      {
        id,
        name,
        direction,
        connectionId,
        localPath,
        remotePath,
        transferred: 0,
        total: 0,
        status: "pending",
        offset: 0,
        speedBps: 0,
        etaSecs: null,
      },
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
    /** Cancel an in-flight (or queued) transfer, leaving it resumable. */
    cancel(id: string) {
      const t = find(id);
      if (!t) return;
      if (t.status === "pending") {
        // Never started — just park it as paused at its current offset.
        t.status = "paused";
        return;
      }
      if (t.status !== "active") return;
      cancelIntent.add(id);
      void invokeQuiet("sftp_transfer_cancel", { id }).catch(() => {});
    },
    /** Resume a paused/failed transfer from the bytes already moved. */
    resume(id: string) {
      const t = find(id);
      if (!t || (t.status !== "paused" && t.status !== "error")) return;
      t.offset = t.transferred;
      t.status = "pending";
      pump();
    },
    /** Restart a transfer from the beginning (discarding partial progress). */
    retry(id: string) {
      const t = find(id);
      if (!t) return;
      t.offset = 0;
      t.transferred = 0;
      t.status = "pending";
      pump();
    },
    /** Drop a single transfer from the list. */
    remove(id: string) {
      cancelIntent.delete(id);
      sample.delete(id);
      onDone.delete(id);
      items = items.filter((t) => t.id !== id);
    },
    /** Drop finished (done/error/paused) jobs from the list. */
    clearFinished() {
      items = items.filter((t) => t.status === "pending" || t.status === "active");
    },
  };
}

export const sftpTransfers = createSftpTransfersStore();
