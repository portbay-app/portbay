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
  /** Number of NVIDIA GPUs (`nvidia-smi`); 0 / null on non-GPU hosts. */
  gpuCount: number | null;
  /** GPU model, or "Mixed" when the box has more than one kind. */
  gpuModel: string | null;
  /** Sum of every GPU's total VRAM, in MiB. */
  gpuTotalVramMb: number | null;
  /** NVIDIA driver version, e.g. "535.183.01". */
  driverVersion: string | null;
  /** CUDA version from the `nvidia-smi` header, e.g. "12.2". */
  cudaVersion: string | null;
  /** Python interpreter version on the active PATH, e.g. "3.11.5". */
  pythonVersion: string | null;
  /** Active conda environment (`$CONDA_DEFAULT_ENV`), e.g. "ml-train" or "base". */
  condaEnv: string | null;
  /** Active virtualenv name — the basename of `$VIRTUAL_ENV`. */
  virtualenv: string | null;
  /** Loaded HPC environment modules (`module list`), e.g. ["cuda/12.4", …]. */
  modules: string[] | null;
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
  "echo '###GPU'; nvidia-smi --query-gpu=name,memory.total,driver_version --format=csv,noheader,nounits 2>/dev/null",
  "echo '###GPUHDR'; nvidia-smi 2>/dev/null | head -5",
  // Environment block. Run under a login shell (`bash -lc`) so HPC `module`
  // init (/etc/profile.d) and conda/venv profile hooks are loaded — a plain
  // non-login exec shell has none of them, so `$CONDA_DEFAULT_ENV` would be
  // empty and `module` undefined. Internal `@@` markers keep it to a single
  // login shell (less profile noise than three). `module` prints its listing
  // to stderr by design, so that one is merged with `2>&1`; the outer
  // `2>/dev/null` only hides bash's own startup errors (and "bash: not found"
  // on hosts without bash, which degrades the whole block to empty).
  "echo '###ENV'; bash -lc 'printf \"@@PY \"; { python3 --version || python --version; } 2>&1; printf \"@@CONDA \"; echo \"${CONDA_DEFAULT_ENV:-}\"; printf \"@@VENV \"; echo \"${VIRTUAL_ENV:-}\"; echo @@MODULE; module list 2>&1' 2>/dev/null",
].join("; ");

/** Run the snapshot command on a host, prompting once for a credential if needed. */
export async function fetchHostSnapshot(
  connectionId: string,
  hostLabel: string,
): Promise<HostSnapshot> {
  // Lazy-import the IPC layer so this module's parser (parseSnapshot) stays a
  // pure, dependency-free import — matching portScan and keeping it unit-testable
  // without booting SvelteKit's rune stores.
  const { invokeQuiet } = await import("$lib/ipc");
  const { connectWithPrompt } = await import("$lib/ssh/connectWithPrompt");
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

  // GPU summary: one `name, memTotalMiB, driver` row per GPU. Absent `nvidia-smi`
  // → empty block → gpuCount 0 (a non-GPU host), never a fabricated number.
  let gpuCount: number | null = null;
  let gpuModel: string | null = null;
  let gpuTotalVramMb: number | null = null;
  let driverVersion: string | null = null;
  const gpuRows = (b.GPU ?? "").split("\n").map((l) => l.trim()).filter(Boolean);
  if (gpuRows.length) {
    const names: string[] = [];
    let vramSum = 0;
    let vramSeen = false;
    for (const row of gpuRows) {
      const cols = row.split(",").map((c) => c.trim());
      if (cols[0]) names.push(cols[0]);
      const mb = Number(cols[1]);
      if (Number.isFinite(mb)) {
        vramSum += mb;
        vramSeen = true;
      }
      if (!driverVersion && cols[2]) driverVersion = cols[2];
    }
    gpuCount = gpuRows.length;
    const uniq = [...new Set(names)];
    gpuModel = uniq.length === 1 ? uniq[0] : uniq.length > 1 ? "Mixed" : null;
    gpuTotalVramMb = vramSeen ? vramSum : null;
  }
  // CUDA version lives only in the nvidia-smi banner, not in any query field.
  const cudaMatch = (b.GPUHDR ?? "").match(/CUDA Version:\s*([\d.]+)/i);
  const cudaVersion = cudaMatch ? cudaMatch[1] : null;

  // Environment block: one login shell with internal `@@` markers. Any leading
  // profile/MOTD noise is ignored since we key off the `@@` prefixes.
  const env = b.ENV ?? "";
  let pythonVersion: string | null = null;
  let condaEnv: string | null = null;
  let virtualenv: string | null = null;
  let modules: string[] | null = null;
  if (env) {
    for (const line of env.split("\n")) {
      if (line.startsWith("@@PY")) {
        const m = line.match(/Python\s+([\w.]+)/i);
        if (m) pythonVersion = m[1];
      } else if (line.startsWith("@@CONDA")) {
        const v = line.slice("@@CONDA".length).trim();
        if (v) condaEnv = v;
      } else if (line.startsWith("@@VENV")) {
        const v = line.slice("@@VENV".length).trim();
        if (v) virtualenv = v.split("/").filter(Boolean).pop() ?? null;
      }
    }
    // Modules: everything after the `@@MODULE` marker (kept last as it spans
    // multiple lines). A host without `module` surfaces "command not found"
    // here (stderr was merged), and an idle one says "No modules loaded" — both
    // mean "nothing to show".
    const modIdx = env.indexOf("@@MODULE");
    if (modIdx >= 0) {
      const modText = env.slice(modIdx + "@@MODULE".length);
      if (!/command not found|not found|no modules|no modulefiles/i.test(modText)) {
        const found = [...modText.matchAll(/\d+\)\s+(\S+)/g)].map((m) => m[1]);
        if (found.length) modules = found;
      }
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
    gpuCount,
    gpuModel,
    gpuTotalVramMb,
    driverVersion,
    cudaVersion,
    pythonVersion,
    condaEnv,
    virtualenv,
    modules,
    raw: stdout,
  };
}
