/**
 * Host-health digest for the Problems panel.
 *
 * PortBay's SSH workspace has no language server and no open editor files, so a
 * literal port of Lapce's LSP-fed Problems panel would render empty. Instead we
 * surface the only honest equivalent for a remote host: real, currently-detected
 * health problems — failed systemd units, disk pressure, high load, and memory
 * pressure — gathered with a single cheap probe over the cached SSH exec session
 * (the same `ssh_exec_run` path Processes/GPU/Logs use) and parsed here.
 *
 * Every probe is wrapped so a missing tool (e.g. `systemctl` on macOS, no
 * `/proc`) just yields an empty section the parser skips — we only ever report
 * problems we actually detected, never fabricated ones.
 */

export type ProblemSeverity = "error" | "warning";

export interface HostProblem {
  severity: ProblemSeverity;
  /** Grouping key shown as the collapsible sub-header (e.g. "systemd", "disk"). */
  source: string;
  /** One-line summary. */
  title: string;
  /** Optional supporting detail (sizes, percentages, the raw unit line). */
  detail?: string;
  /** Optional next step the user can take (e.g. a command to run). */
  hint?: string;
}

// Section markers. A single bundled command prints each probe under its marker;
// the parser splits on them. `2>/dev/null` + the parser ignoring empty/garbage
// lines means an unavailable probe simply contributes nothing.
const M = {
  failed: "@@PB_FAILED@@",
  disk: "@@PB_DISK@@",
  load: "@@PB_LOAD@@",
  mem: "@@PB_MEM@@",
  end: "@@PB_END@@",
} as const;

/** The one shell command run over `ssh_exec_run` to collect every probe. */
export const HOST_PROBLEMS_PROBE = [
  `echo ${M.failed}`,
  `systemctl --failed --no-legend --plain 2>/dev/null`,
  `echo ${M.disk}`,
  `df -P -k 2>/dev/null`,
  `echo ${M.load}`,
  `cat /proc/loadavg 2>/dev/null`,
  `nproc 2>/dev/null`,
  `echo ${M.mem}`,
  `cat /proc/meminfo 2>/dev/null`,
  `echo ${M.end}`,
].join("; ");

// Pseudo / virtual filesystems whose "fullness" isn't actionable disk pressure.
const PSEUDO_FS = /^(tmpfs|devtmpfs|udev|overlay|none|squashfs|efivarfs|cgroup)/i;

function humanKB(kb: number): string {
  if (!Number.isFinite(kb) || kb < 0) return "—";
  let n = kb * 1024;
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  let i = 0;
  while (n >= 1024 && i < units.length - 1) {
    n /= 1024;
    i++;
  }
  return `${n >= 100 || i === 0 ? Math.round(n) : n.toFixed(1)} ${units[i]}`;
}

/** Pull the body of one marked section out of the combined probe output. */
function section(stdout: string, start: string, end: string): string {
  const i = stdout.indexOf(start);
  if (i === -1) return "";
  const from = i + start.length;
  const j = stdout.indexOf(end, from);
  return stdout.slice(from, j === -1 ? undefined : j);
}

function parseFailedUnits(body: string): HostProblem[] {
  const out: HostProblem[] = [];
  for (const raw of body.split("\n")) {
    const line = raw.trim();
    if (!line) continue;
    const unit = line.split(/\s+/)[0];
    // systemctl can print a trailing "0 loaded units listed." footer even with
    // --no-legend on some versions; only accept real unit names.
    if (!unit || !/\.(service|socket|timer|mount|target|path|scope)$/.test(unit)) continue;
    out.push({
      severity: "error",
      source: "systemd",
      title: `${unit} failed`,
      detail: line,
      hint: `Inspect with: systemctl status ${unit}`,
    });
  }
  return out;
}

function parseDisk(body: string): HostProblem[] {
  const out: HostProblem[] = [];
  const lines = body.split("\n").filter((l) => l.trim());
  // Drop the `df` header row if present.
  const rows = lines[0] && /Filesystem/i.test(lines[0]) ? lines.slice(1) : lines;
  for (const line of rows) {
    // POSIX `df -P -k`: Filesystem 1024-blocks Used Available Capacity Mounted-on
    const cols = line.trim().split(/\s+/);
    if (cols.length < 6) continue;
    const fs = cols[0];
    if (PSEUDO_FS.test(fs)) continue;
    const sizeKB = Number(cols[1]);
    const usedKB = Number(cols[2]);
    const capacity = Number(cols[4].replace("%", ""));
    const mount = cols.slice(5).join(" ");
    if (!Number.isFinite(capacity) || sizeKB <= 0) continue;
    if (capacity < 85) continue;
    out.push({
      severity: capacity >= 95 ? "error" : "warning",
      source: "disk",
      title: `${mount} is ${capacity}% full`,
      detail: `${humanKB(usedKB)} used of ${humanKB(sizeKB)} on ${fs}`,
      hint: "Free space or grow the volume before it fills.",
    });
  }
  return out;
}

function parseLoad(body: string): HostProblem[] {
  const lines = body.split("\n").map((l) => l.trim()).filter(Boolean);
  if (lines.length === 0) return [];
  const nums = lines[0].split(/\s+/);
  const load1 = Number(nums[0]);
  const load5 = Number(nums[1]);
  const load15 = Number(nums[2]);
  if (!Number.isFinite(load1)) return [];
  // A bare integer on its own line is `nproc`'s output (cpu count).
  const cpuLine = lines.find((l, i) => i > 0 && /^\d+$/.test(l));
  const cpus = cpuLine ? Number(cpuLine) : 1;
  const ratio = load1 / Math.max(1, cpus);
  if (ratio < 2) return [];
  return [
    {
      severity: ratio >= 4 ? "error" : "warning",
      source: "load",
      title: `Load ${load1.toFixed(2)} across ${cpus} CPU${cpus === 1 ? "" : "s"}`,
      detail: `1m ${load1.toFixed(2)} · 5m ${Number.isFinite(load5) ? load5.toFixed(2) : "—"} · 15m ${Number.isFinite(load15) ? load15.toFixed(2) : "—"}`,
      hint: "Find the busy process in the Processes tab.",
    },
  ];
}

function parseMem(body: string): HostProblem[] {
  const field = (name: string): number => {
    const m = body.match(new RegExp(`^${name}:\\s+(\\d+)`, "m"));
    return m ? Number(m[1]) : NaN;
  };
  const totalKB = field("MemTotal");
  const availKB = field("MemAvailable");
  if (!Number.isFinite(totalKB) || !Number.isFinite(availKB) || totalKB <= 0) return [];
  const pct = (availKB / totalKB) * 100;
  if (pct >= 12) return [];
  return [
    {
      severity: pct < 5 ? "error" : "warning",
      source: "memory",
      title: pct < 5 ? "Memory almost exhausted" : "Memory under pressure",
      detail: `${humanKB(availKB)} available of ${humanKB(totalKB)} (${pct.toFixed(0)}% free)`,
      hint: "A spike risks the OOM killer terminating processes.",
    },
  ];
}

/** Parse the combined probe stdout into the detected host problems. */
export function parseHostProblems(stdout: string): HostProblem[] {
  return [
    ...parseFailedUnits(section(stdout, M.failed, M.disk)),
    ...parseDisk(section(stdout, M.disk, M.load)),
    ...parseLoad(section(stdout, M.load, M.mem)),
    ...parseMem(section(stdout, M.mem, M.end)),
  ];
}
