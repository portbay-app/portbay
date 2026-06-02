/**
 * hostSnapshot — a point-in-time readout of a remote host, gathered by running
 * one real shell command over `ssh_exec_run` (so it goes through the same
 * credential-prompt flow as everything else) and parsing the output.
 *
 * This is deliberately NOT a live gauge: exec is captured per-command, not
 * streamed, and there's no resource-sampling API. The workspace labels the
 * result as a snapshot ("as of …") and refreshes only on explicit request.
 *
 * Everything is best-effort and Linux-leaning (`free`, `df`, `uptime`). Missing
 * tools (e.g. `free` on macOS/BSD) just leave their fields null — we never
 * fabricate a number. The raw output is kept so the UI can fall back to showing
 * it verbatim.
 */
import { invokeQuiet } from "$lib/ipc";
import { connectWithPrompt } from "$lib/ssh/connectWithPrompt";
import type { ExecResult } from "$lib/types/sshTunnels";

export interface HostSnapshot {
  /** `whoami` — the account the session authenticated as. */
  user: string | null;
  /** `uname -sr` — kernel name + release. */
  os: string | null;
  /** Human uptime, e.g. "13 days, 6:24". */
  uptime: string | null;
  /** 1-minute load average. */
  load1: string | null;
  /** Memory in MiB (from `free -m`). */
  memUsedMb: number | null;
  memTotalMb: number | null;
  /** Root-filesystem usage (from `df -h /`). */
  diskUsed: string | null;
  diskTotal: string | null;
  diskPercent: number | null;
  /** Raw combined stdout, for verbatim fallback / debugging. */
  raw: string;
}

// One command, marker-delimited blocks. `2>/dev/null` keeps a missing tool from
// polluting a block; absent blocks parse to null rather than an error.
const SNAPSHOT_COMMAND = [
  "echo '###USER'; whoami",
  "echo '###OS'; uname -sr 2>/dev/null",
  "echo '###UP'; uptime 2>/dev/null",
  "echo '###MEM'; free -m 2>/dev/null",
  "echo '###DISK'; df -h / 2>/dev/null",
].join("; ");

/** Run the snapshot command on a host, prompting once for a credential if needed. */
export async function fetchHostSnapshot(
  connectionId: string,
  hostLabel: string,
): Promise<HostSnapshot> {
  const result = await connectWithPrompt(connectionId, hostLabel, (cred) =>
    invokeQuiet<ExecResult>("ssh_exec_run", {
      input: {
        connectionId,
        command: SNAPSHOT_COMMAND,
        password: cred?.kind === "password" ? cred.secret : undefined,
        passphrase: cred?.kind === "passphrase" ? cred.secret : undefined,
      },
    }),
  );
  return parseSnapshot(result.stdout ?? "");
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

export function parseSnapshot(stdout: string): HostSnapshot {
  const b = blocks(stdout);
  const trimOrNull = (s: string | undefined) => {
    const v = (s ?? "").trim();
    return v ? v : null;
  };

  // uptime: "10:25:41 up 13 days,  6:24,  2 users,  load average: 0.31, 0.28, 0.25"
  const up = trimOrNull(b.UP);
  let uptime: string | null = null;
  let load1: string | null = null;
  if (up) {
    const upMatch = up.match(/\bup\s+(.+?),\s+\d+\s+users?/i) ?? up.match(/\bup\s+(.+?),\s+load/i);
    uptime = upMatch ? upMatch[1].trim() : null;
    const loadMatch = up.match(/load averages?:\s*([\d.]+)/i);
    load1 = loadMatch ? loadMatch[1] : null;
  }

  // free -m second line: "Mem:  <total> <used> <free> ..."
  let memTotalMb: number | null = null;
  let memUsedMb: number | null = null;
  const memLine = (b.MEM ?? "").split("\n").find((l) => /^Mem:/i.test(l.trim()));
  if (memLine) {
    const nums = memLine.trim().split(/\s+/).slice(1).map(Number);
    if (Number.isFinite(nums[0])) memTotalMb = nums[0];
    if (Number.isFinite(nums[1])) memUsedMb = nums[1];
  }

  // df -h / second line: "<fs> <size> <used> <avail> <use%> <mount>"
  let diskTotal: string | null = null;
  let diskUsed: string | null = null;
  let diskPercent: number | null = null;
  const dfLine = (b.DISK ?? "")
    .split("\n")
    .map((l) => l.trim())
    .filter(Boolean)
    .find((l) => !/^Filesystem/i.test(l));
  if (dfLine) {
    const cols = dfLine.split(/\s+/);
    // Tolerate a filesystem name that wrapped: take the size/used/use% by the
    // trailing layout (… size used avail use% mountpoint).
    const pctIdx = cols.findIndex((c) => /^\d+%$/.test(c));
    if (pctIdx >= 3) {
      diskTotal = cols[pctIdx - 3] ?? null;
      diskUsed = cols[pctIdx - 2] ?? null;
      diskPercent = Number(cols[pctIdx].replace("%", ""));
      if (!Number.isFinite(diskPercent)) diskPercent = null;
    }
  }

  return {
    user: trimOrNull(b.USER),
    os: trimOrNull(b.OS),
    uptime,
    load1,
    memUsedMb,
    memTotalMb,
    diskUsed,
    diskTotal,
    diskPercent,
    raw: stdout,
  };
}
