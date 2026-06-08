/**
 * jobsSnapshot — a point-in-time readout of the long-running work on a remote
 * host that outlives the SSH session: persistent terminal multiplexer sessions
 * (`tmux ls` / `screen -ls`) and, on a cluster, the user's SLURM jobs
 * (`squeue` now + a little `sacct` history). Gathered by one marker-delimited
 * command over `ssh_exec_run` — the same credential-prompt flow as the GPU and
 * Processes panels.
 *
 * Like the sibling snapshots this is deliberately NOT live: exec is captured
 * per-command, not streamed; the panel labels it a snapshot and re-runs only on
 * request. SLURM is gated behind `command -v squeue`: on a box without a
 * scheduler `hasSlurm` is false and the UI hides that whole section rather than
 * showing an empty scheduler — the same graceful-degrade `gpuSnapshot` does for
 * a missing `nvidia-smi`.
 */
import { invokeQuiet } from "$lib/ipc";
import { connectWithPrompt } from "$lib/ssh/connectWithPrompt";
import type { ExecResult } from "$lib/types/sshTunnels";

export type SessionKind = "tmux" | "screen";

/** A persistent multiplexer session — the thing a dropped SSH won't kill. */
export interface PersistentSession {
  kind: SessionKind;
  /** Attach target: tmux session name, or screen's `<pid>.<name>` socket. */
  target: string;
  /** Human label (tmux name, or screen's name part after the pid). */
  label: string;
  /** Whether a client is currently attached to the session. */
  attached: boolean;
  /** tmux window count, when reported; null for screen. */
  windows: number | null;
}

/** A live SLURM job from `squeue` (one the user can still cancel). */
export interface SlurmJob {
  id: string;
  name: string;
  /** RUNNING, PENDING, CONFIGURING, … (squeue %T). */
  state: string;
  /** Node list when running, or the pending reason in (parens). */
  where: string;
  /** Elapsed wallclock, e.g. "1-02:14:09". */
  elapsed: string;
  /** Allocated/requested node count. */
  nodes: string | null;
  partition: string | null;
}

/** A finished SLURM job from `sacct` — recent history, not actionable. */
export interface SlurmHistoryEntry {
  id: string;
  name: string;
  /** COMPLETED, FAILED, CANCELLED, TIMEOUT, … (first word of sacct State). */
  state: string;
  elapsed: string;
  /** End timestamp as sacct reports it (may be "Unknown"). */
  end: string;
}

export interface JobsReadout {
  sessions: PersistentSession[];
  /** False when `squeue` isn't on PATH (non-cluster host) — hide SLURM entirely. */
  hasSlurm: boolean;
  jobs: SlurmJob[];
  history: SlurmHistoryEntry[];
  /** Raw combined stdout, for verbatim fallback / debugging. */
  raw: string;
}

// One marker-delimited command (mirrors gpuSnapshot). `2>/dev/null` keeps a
// missing tool from polluting a block — an absent `tmux`/`screen`/`squeue` just
// leaves its block empty, which parses to "none" rather than an error.
//
//   TMUX   — `tmux ls`: one line per session ("name: N windows … (attached)")
//   SCREEN — `screen -ls`: indented "<pid>.<name>\t(Detached|Attached)" lines
//   SLURM  — "yes"/"no": is `squeue` even installed on this host
//   SQUEUE — the user's live jobs, pipe-delimited: id|name|state|where|time|nodes|part
//   SACCT  — recent finished jobs, pipe-delimited: id|name|state|elapsed|end
const JOBS_COMMAND = [
  "echo '###TMUX'; tmux ls 2>/dev/null",
  "echo '###SCREEN'; screen -ls 2>/dev/null",
  "echo '###SLURM'; command -v squeue >/dev/null 2>&1 && echo yes || echo no",
  'echo \'###SQUEUE\'; command -v squeue >/dev/null 2>&1 && squeue -u "$USER" --noheader -o \'%i|%j|%T|%R|%M|%D|%P\' 2>/dev/null',
  'echo \'###SACCT\'; command -v sacct >/dev/null 2>&1 && sacct -u "$USER" -X --noheader -P --format=JobID,JobName,State,Elapsed,End 2>/dev/null | tail -n 12',
].join("\n");

/** Run the jobs command on a host, prompting once for a credential if needed. */
export async function fetchJobsReadout(
  connectionId: string,
  hostLabel: string,
): Promise<JobsReadout> {
  const result = await connectWithPrompt(connectionId, hostLabel, (cred) =>
    invokeQuiet<ExecResult>("ssh_exec_run", {
      input: {
        connectionId,
        command: JOBS_COMMAND,
        password: cred?.kind === "password" ? cred.secret : undefined,
        passphrase: cred?.kind === "passphrase" ? cred.secret : undefined,
      },
    }),
  );
  return parseJobsReadout(result.stdout ?? "");
}

/** Split the marker-delimited stdout into its blocks. */
function blocks(stdout: string): Record<string, string> {
  const out: Record<string, string> = {};
  let key = "";
  for (const line of stdout.split("\n")) {
    const marker = line.match(/^###(\w+)\s*$/);
    if (marker) {
      key = marker[1];
      out[key] = "";
    } else if (key) {
      out[key] += (out[key] ? "\n" : "") + line;
    }
  }
  return out;
}

function lines(block: string | undefined): string[] {
  return (block ?? "").split("\n").filter((l) => l.trim().length > 0);
}

/** First whitespace-delimited token — sacct states can read "CANCELLED by 1000". */
function firstWord(s: string): string {
  return s.trim().split(/\s+/)[0] ?? s.trim();
}

export function parseJobsReadout(stdout: string): JobsReadout {
  const b = blocks(stdout);
  const sessions: PersistentSession[] = [];

  // tmux: "main: 3 windows (created Mon Jun 2 …) (attached)". A line without the
  // "N windows" shape (e.g. an error that slipped past 2>/dev/null) is skipped.
  for (const line of lines(b.TMUX)) {
    const m = line.match(/^([^:]+):\s+(\d+)\s+windows?/);
    if (!m) continue;
    const name = m[1].trim();
    sessions.push({
      kind: "tmux",
      target: name,
      label: name,
      attached: /\(attached\)/i.test(line),
      windows: Number(m[2]),
    });
  }

  // screen: indented "<pid>.<name>\t(Detached)" rows; the header/footer lines
  // ("There are screens on:", "N Sockets in …") don't match the socket shape.
  for (const line of lines(b.SCREEN)) {
    const m = line.match(/^\s+(\d+\.\S+)\s+\((Attached|Detached|Multi)/i);
    if (!m) continue;
    const target = m[1];
    const dot = target.indexOf(".");
    sessions.push({
      kind: "screen",
      target,
      label: dot >= 0 ? target.slice(dot + 1) : target,
      attached: /^Attached/i.test(m[2]) || /^Multi/i.test(m[2]),
      windows: null,
    });
  }

  const hasSlurm = (b.SLURM ?? "").trim() === "yes";

  const jobs: SlurmJob[] = [];
  const liveIds = new Set<string>();
  if (hasSlurm) {
    for (const line of lines(b.SQUEUE)) {
      const c = line.split("|");
      if (c.length < 7 || !c[0].trim()) continue;
      liveIds.add(c[0].trim());
      jobs.push({
        id: c[0].trim(),
        name: c[1].trim() || "(job)",
        state: c[2].trim() || "UNKNOWN",
        where: c[3].trim(),
        elapsed: c[4].trim(),
        nodes: c[5].trim() || null,
        partition: c[6].trim() || null,
      });
    }
  }

  // sacct history: terminal states only, and never a job already shown as live
  // above (sacct overlaps squeue for in-flight allocations).
  const history: SlurmHistoryEntry[] = [];
  if (hasSlurm) {
    for (const line of lines(b.SACCT)) {
      const c = line.split("|");
      if (c.length < 5 || !c[0].trim()) continue;
      const id = c[0].trim();
      const state = firstWord(c[2]);
      if (liveIds.has(id) || state === "RUNNING" || state === "PENDING") continue;
      history.push({
        id,
        name: c[1].trim() || "(job)",
        state,
        elapsed: c[3].trim(),
        end: c[4].trim(),
      });
    }
    history.reverse(); // most-recent first (sacct lists oldest → newest)
  }

  return { sessions, hasSlurm, jobs, history, raw: stdout };
}
