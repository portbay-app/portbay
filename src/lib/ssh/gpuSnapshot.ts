/**
 * gpuSnapshot — a point-in-time readout of the NVIDIA GPUs on a remote host,
 * gathered by running `nvidia-smi` over `ssh_exec_run` (same credential-prompt
 * flow as the rest of the workbench) and parsing its CSV output.
 *
 * Like `hostSnapshot`, this is deliberately NOT a live gauge: exec is captured
 * per-command, not streamed. The panel labels the result as a snapshot and
 * re-runs only on request. Everything degrades gracefully — on a host without
 * an NVIDIA GPU, `nvidia-smi` is absent, every block comes back empty, and we
 * report `available: false` rather than fabricating anything.
 *
 * Two joins make the readout useful on a shared box:
 *   - compute apps → GPU, by `gpu_uuid` (which GPU each process sits on), and
 *   - pid → user, from `ps` (who owns each process) — `nvidia-smi` itself never
 *     reports the owning user, and "who is hogging GPU 3?" is the whole point.
 */
import { invokeQuiet } from "$lib/ipc";
import { connectWithPrompt } from "$lib/ssh/connectWithPrompt";
import type { ExecResult } from "$lib/types/sshTunnels";

/** A compute process occupying a GPU. */
export interface GpuProc {
  pid: string;
  /** Resolved from `ps` (pid → user); null if the pid wasn't in the ps table. */
  user: string | null;
  /** GPU memory held by this process, in MiB. */
  memMb: number | null;
  /** Process name as reported by `nvidia-smi` (e.g. "python"). */
  name: string;
}

/** Live state of one GPU. */
export interface GpuStat {
  index: number;
  name: string;
  uuid: string;
  /** Core utilization, 0–100, or null when the GPU doesn't report it (e.g. MIG). */
  utilization: number | null;
  memUsedMb: number | null;
  memTotalMb: number | null;
  /** Die temperature in °C. */
  tempC: number | null;
  /** Current board power draw in W. */
  powerW: number | null;
  /** Enforced power limit in W, for context on the draw. */
  powerLimitW: number | null;
  /** Compute processes on this GPU, biggest VRAM first. */
  procs: GpuProc[];
}

export interface GpuReadout {
  gpus: GpuStat[];
  /** NVIDIA driver version, e.g. "535.183.01". */
  driver: string | null;
  /** CUDA version from the `nvidia-smi` header, e.g. "12.2". */
  cuda: string | null;
  /** False when `nvidia-smi` isn't present / returned nothing (non-GPU host). */
  available: boolean;
  /** Raw combined stdout, for verbatim fallback / debugging. */
  raw: string;
}

// One command, marker-delimited blocks (mirrors hostSnapshot). `2>/dev/null`
// keeps a missing `nvidia-smi` from polluting a block — an absent tool just
// leaves every block empty, which parses to "no GPUs" rather than an error.
//
//   GPU  — one row per GPU: index,uuid,name,util%,mem.used,mem.total,temp,power,power.limit,driver
//   APPS — one row per compute process: gpu_uuid,pid,used_memory,process_name
//   PS   — `pid user` for every process, to resolve the owner of each app pid
//   HDR  — the nvidia-smi banner, which carries the CUDA version
const GPU_COMMAND = [
  "echo '###GPU'; nvidia-smi --query-gpu=index,uuid,name,utilization.gpu,memory.used,memory.total,temperature.gpu,power.draw,power.limit,driver_version --format=csv,noheader,nounits 2>/dev/null",
  "echo '###APPS'; nvidia-smi --query-compute-apps=gpu_uuid,pid,used_memory,process_name --format=csv,noheader,nounits 2>/dev/null",
  "echo '###PS'; ps -eo pid=,user= 2>/dev/null",
  "echo '###HDR'; nvidia-smi 2>/dev/null | head -5",
].join("; ");

/** Run the GPU command on a host, prompting once for a credential if needed. */
export async function fetchGpuReadout(
  connectionId: string,
  hostLabel: string,
): Promise<GpuReadout> {
  const result = await connectWithPrompt(connectionId, hostLabel, (cred) =>
    invokeQuiet<ExecResult>("ssh_exec_run", {
      input: {
        connectionId,
        command: GPU_COMMAND,
        password: cred?.kind === "password" ? cred.secret : undefined,
        passphrase: cred?.kind === "passphrase" ? cred.secret : undefined,
      },
    }),
  );
  return parseGpuReadout(result.stdout ?? "");
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

// `nounits` still emits "[N/A]" / "[Not Supported]" for fields a card can't
// report (no power sensor, MIG util). Treat those — and blanks — as unknown.
function num(cell: string | undefined): number | null {
  const v = (cell ?? "").trim();
  if (!v || /^\[/.test(v)) return null;
  const n = Number(v);
  return Number.isFinite(n) ? n : null;
}

function rows(block: string | undefined): string[] {
  return (block ?? "").split("\n").map((l) => l.trim()).filter(Boolean);
}

export function parseGpuReadout(stdout: string): GpuReadout {
  const b = blocks(stdout);

  // pid → user, from `ps -eo pid= user=` (headerless: "  1234 alice").
  const userByPid = new Map<string, string>();
  for (const line of rows(b.PS)) {
    const m = line.match(/^(\d+)\s+(.+)$/);
    if (m) userByPid.set(m[1], m[2].trim());
  }

  // Compute processes, grouped by the GPU uuid they run on.
  const appsByUuid = new Map<string, GpuProc[]>();
  for (const line of rows(b.APPS)) {
    const cols = line.split(",").map((c) => c.trim());
    if (cols.length < 4) continue;
    const [uuid, pid, usedMem, ...nameParts] = cols;
    if (!appsByUuid.has(uuid)) appsByUuid.set(uuid, []);
    appsByUuid.get(uuid)!.push({
      pid,
      user: userByPid.get(pid) ?? null,
      memMb: num(usedMem),
      name: nameParts.join(",") || "—",
    });
  }

  const gpus: GpuStat[] = [];
  let driver: string | null = null;
  for (const line of rows(b.GPU)) {
    const cols = line.split(",").map((c) => c.trim());
    if (cols.length < 10) continue;
    const index = num(cols[0]);
    if (index == null) continue;
    const uuid = cols[1];
    const procs = (appsByUuid.get(uuid) ?? []).sort(
      (a, z) => (z.memMb ?? 0) - (a.memMb ?? 0),
    );
    if (!driver && cols[9]) driver = cols[9];
    gpus.push({
      index,
      uuid,
      name: cols[2] || `GPU ${index}`,
      utilization: num(cols[3]),
      memUsedMb: num(cols[4]),
      memTotalMb: num(cols[5]),
      tempC: num(cols[6]),
      powerW: num(cols[7]),
      powerLimitW: num(cols[8]),
      procs,
    });
  }
  gpus.sort((a, z) => a.index - z.index);

  // CUDA version lives only in the nvidia-smi banner, not in any query field.
  const cudaMatch = (b.HDR ?? "").match(/CUDA Version:\s*([\d.]+)/i);
  const cuda = cudaMatch ? cudaMatch[1] : null;

  return { gpus, driver, cuda, available: gpus.length > 0, raw: stdout };
}
