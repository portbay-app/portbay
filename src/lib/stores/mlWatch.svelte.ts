/**
 * mlWatch — "tell me when this run is done." A small, opt-in watch list for the
 * long-running things an ML researcher walks away from: a training PID, or a GPU
 * they're waiting to free up. While there's at least one watch, a single light
 * poll runs over the *already-warm* exec session (no credential prompts, no
 * background hammering — one exec per host per tick) and fires a notification
 * into the existing bell (`errorBus`) the moment something changes:
 *
 *   - a watched process exits — reported honestly as "finished", upgraded to
 *     "crashed (out of memory)" when `dmesg` confirms the OOM-killer ended it
 *     (the one crash cause we can actually read for a process we don't parent);
 *   - a watched GPU goes idle — 0% utilization and no compute processes.
 *
 * Watches are one-shot: once the event fires, the watch is removed. State is
 * in-memory for the session (a watch means "for this run"); it intentionally
 * doesn't survive an app restart.
 */
import { invokeQuiet } from "$lib/ipc";
import { errorBus } from "$lib/stores/errors.svelte";
import type { ExecResult } from "$lib/types/sshTunnels";

const POLL_MS = 20_000;

export type WatchKind = "process" | "gpu";

export interface Watch {
  /** `${connectionId}:${kind}:${ref}` — stable identity for toggle/dedupe. */
  id: string;
  connectionId: string;
  /** Host display name, for the notification copy. */
  hostLabel: string;
  kind: WatchKind;
  /** PID (process) or GPU index (gpu). */
  ref: string;
  /** Command line / GPU model — what the user actually recognizes. */
  name: string;
  createdAt: number;
}

function watchId(connectionId: string, kind: WatchKind, ref: string): string {
  return `${connectionId}:${kind}:${ref}`;
}

function createMlWatch() {
  let watches = $state<Watch[]>([]);
  let timer: ReturnType<typeof setInterval> | null = null;
  // Guard so overlapping ticks (a slow exec) never double-fire on one host.
  let polling = false;

  function ensureRunning() {
    if (timer || watches.length === 0) return;
    timer = setInterval(() => void poll(), POLL_MS);
  }

  function stopIfIdle() {
    if (timer && watches.length === 0) {
      clearInterval(timer);
      timer = null;
    }
  }

  function isWatched(connectionId: string, kind: WatchKind, ref: string): boolean {
    return watches.some((w) => w.id === watchId(connectionId, kind, ref));
  }

  function add(w: Omit<Watch, "id" | "createdAt">) {
    const id = watchId(w.connectionId, w.kind, w.ref);
    if (watches.some((x) => x.id === id)) return;
    watches = [...watches, { ...w, id, createdAt: Date.now() }];
    ensureRunning();
  }

  function remove(id: string) {
    watches = watches.filter((w) => w.id !== id);
    stopIfIdle();
  }

  function toggle(w: Omit<Watch, "id" | "createdAt">) {
    const id = watchId(w.connectionId, w.kind, w.ref);
    if (watches.some((x) => x.id === id)) remove(id);
    else add(w);
  }

  function countFor(connectionId: string): number {
    return watches.filter((w) => w.connectionId === connectionId).length;
  }

  async function exec(connectionId: string, command: string): Promise<string | null> {
    try {
      const r = await invokeQuiet<ExecResult>("ssh_exec_run", {
        input: { connectionId, command },
      });
      return r.stdout ?? "";
    } catch {
      // Session not warm / host unreachable — skip this tick silently. We never
      // prompt for a credential from a background poll.
      return null;
    }
  }

  // Split a marker-delimited block dump (`###KEY` … lines …) into a map.
  function blocks(stdout: string): Record<string, string> {
    const out: Record<string, string> = {};
    let key = "";
    for (const line of stdout.split("\n")) {
      const m = line.match(/^###(\w+)\s*$/);
      if (m) {
        key = m[1];
        out[key] = "";
      } else if (key) {
        out[key] += (out[key] ? "\n" : "") + line;
      }
    }
    return out;
  }

  async function poll() {
    if (polling || watches.length === 0) return;
    polling = true;
    try {
      // One exec per host, covering all of that host's watches at once.
      const byHost = new Map<string, Watch[]>();
      for (const w of watches) {
        if (!byHost.has(w.connectionId)) byHost.set(w.connectionId, []);
        byHost.get(w.connectionId)!.push(w);
      }
      for (const [connectionId, hostWatches] of byHost) {
        await pollHost(connectionId, hostWatches);
      }
    } finally {
      polling = false;
    }
  }

  async function pollHost(connectionId: string, hostWatches: Watch[]) {
    const pids = hostWatches.filter((w) => w.kind === "process").map((w) => w.ref);
    const gpuWatched = hostWatches.some((w) => w.kind === "gpu");

    const parts: string[] = [];
    if (pids.length) {
      // `kill -0` is a no-signal liveness probe: exit 0 = alive. One line each.
      parts.push(
        `echo '###PROC'; for p in ${pids.join(" ")}; do if kill -0 $p 2>/dev/null; then echo "$p ALIVE"; else echo "$p GONE"; fi; done`,
      );
    }
    if (gpuWatched) {
      parts.push(
        "echo '###G'; nvidia-smi --query-gpu=index,uuid,utilization.gpu --format=csv,noheader,nounits 2>/dev/null",
        "echo '###A'; nvidia-smi --query-compute-apps=gpu_uuid --format=csv,noheader,nounits 2>/dev/null",
      );
    }
    if (!parts.length) return;

    const stdout = await exec(connectionId, parts.join("; "));
    if (stdout == null) return;
    const b = blocks(stdout);

    // --- Process watches: fire when a watched PID is no longer running. ---
    if (pids.length) {
      const gone = new Set<string>();
      for (const line of (b.PROC ?? "").split("\n")) {
        const m = line.trim().match(/^(\d+)\s+(ALIVE|GONE)$/);
        if (m && m[2] === "GONE") gone.add(m[1]);
      }
      for (const w of hostWatches.filter((x) => x.kind === "process" && gone.has(x.ref))) {
        await fireProcess(connectionId, w);
        remove(w.id);
      }
    }

    // --- GPU watches: fire when a watched GPU goes idle (0% + no procs). ---
    if (gpuWatched) {
      const busyUuids = new Set(
        (b.A ?? "")
          .split("\n")
          .map((l) => l.trim())
          .filter(Boolean),
      );
      // index → { uuid, util }
      const gpus = new Map<string, { uuid: string; util: number }>();
      for (const line of (b.G ?? "").split("\n")) {
        const cols = line.split(",").map((c) => c.trim());
        if (cols.length < 3) continue;
        const idx = cols[0];
        const util = Number(cols[2]);
        gpus.set(idx, { uuid: cols[1], util: Number.isFinite(util) ? util : -1 });
      }
      for (const w of hostWatches.filter((x) => x.kind === "gpu")) {
        const g = gpus.get(w.ref);
        if (!g) continue; // GPU not reported this tick — wait for the next.
        const idle = g.util === 0 && !busyUuids.has(g.uuid);
        if (idle) {
          fireGpuFree(w);
          remove(w.id);
        }
      }
    }
  }

  async function fireProcess(connectionId: string, w: Watch) {
    // Best-effort crash signal: did the OOM-killer take it? That's the one
    // failure we can read for a process we don't parent (and the #1 ML crash).
    const oom = await exec(
      connectionId,
      `(dmesg 2>/dev/null || journalctl -k --no-pager 2>/dev/null) | grep -iE "killed process ${w.ref}\\b|out of memory.*${w.ref}\\b" | tail -1`,
    );
    const oomKilled = !!oom && oom.trim().length > 0;

    if (oomKilled) {
      errorBus.push({
        code: "ML_RUN_CRASH",
        category: "infrastructure",
        severity: "error",
        whoCausedIt: "system",
        whatHappened: `Watched process ${w.ref} was killed on ${w.hostLabel}`,
        whyItMatters: `Out of memory — the kernel OOM-killer ended ${w.name}.`,
        actions: [],
      });
    } else {
      errorBus.push({
        code: "ML_RUN_DONE",
        category: "infrastructure",
        severity: "success",
        whoCausedIt: "system",
        whatHappened: `Watched process ${w.ref} finished on ${w.hostLabel}`,
        whyItMatters: `${w.name} is no longer running. (Exit status isn't readable for a process PortBay didn't launch.)`,
        actions: [],
      });
    }
  }

  function fireGpuFree(w: Watch) {
    errorBus.push({
      code: "ML_GPU_FREE",
      category: "infrastructure",
      severity: "success",
      whoCausedIt: "system",
      whatHappened: `GPU ${w.ref} is free on ${w.hostLabel}`,
      whyItMatters: `0% utilization and no compute processes — ${w.name} is yours to grab.`,
      actions: [],
    });
  }

  return {
    get value() {
      return watches;
    },
    isWatched,
    add,
    remove,
    toggle,
    countFor,
  };
}

export const mlWatch = createMlWatch();
