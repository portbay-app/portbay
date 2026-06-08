/**
 * portScan — pure helpers for the Ports panel: the command that lists a host's
 * listening TCP ports, a parser for its output, and well-known port labels.
 *
 * Kept free of IPC/store imports so it's unit-testable in isolation; the panel
 * component runs {@link SCAN_COMMAND} over `ssh_exec_run` (the cached exec
 * session, same credential flow as everything else) and feeds the stdout here.
 *
 * Like {@link ./hostSnapshot}, this is a point-in-time readout, not a live feed:
 * exec is captured per-command, so the panel refreshes on demand. We try `ss`
 * first (modern Linux) and fall back to `netstat`; process names need root to
 * read for *other* users' sockets, so they're best-effort — a missing name just
 * leaves the field null, never fabricated.
 */

export interface DetectedPort {
  /** The listening TCP port. */
  port: number;
  /** Bind address as reported (`0.0.0.0`, `127.0.0.1`, `::`, `*`, …). */
  address: string;
  /** Owning process name, when the host let us read it. */
  processName: string | null;
  /** Owning PID, when reported. */
  pid: number | null;
}

// `-H` drops ss's header; if an old `ss` rejects it the `||` chain falls through
// to netstat. `2>/dev/null` keeps a "permission denied" line for `-p` from
// polluting the parse — we still get the port, just no process name.
export const SCAN_COMMAND =
  "ss -tlnpH 2>/dev/null || netstat -tlnp 2>/dev/null || netstat -tln 2>/dev/null";

/** Curated well-known port → human label, including the ML-tooling ports the
 *  quick-forward presets build on. Absent ports just show their process name. */
const WELL_KNOWN: Record<number, string> = {
  22: "SSH",
  80: "HTTP",
  443: "HTTPS",
  3000: "Dev server",
  3001: "Dev server",
  4000: "Dev server",
  5000: "Flask / dev",
  5173: "Vite",
  8000: "HTTP (alt)",
  8080: "HTTP (alt)",
  8443: "HTTPS (alt)",
  3306: "MySQL",
  5432: "PostgreSQL",
  6379: "Redis",
  11211: "Memcached",
  27017: "MongoDB",
  9200: "Elasticsearch",
  5601: "Kibana",
  9090: "Prometheus",
  8888: "Jupyter",
  6006: "TensorBoard",
  8265: "Ray dashboard",
};

/** Human label for a well-known port, or null when it isn't one we recognise. */
export function wellKnownLabel(port: number): string | null {
  return WELL_KNOWN[port] ?? null;
}

function isLoopback(addr: string): boolean {
  return addr === "127.0.0.1" || addr === "::1" || addr === "[::1]";
}

/**
 * Parse `ss -tlnp` / `netstat -tlnp` output into one entry per listening port.
 * Handles both tools and merges the IPv4/IPv6 rows a single service produces,
 * preferring a broader bind address and any row that carried a process name.
 */
export function parseListeningPorts(stdout: string): DetectedPort[] {
  const byPort = new Map<number, DetectedPort>();

  for (const raw of stdout.split("\n")) {
    const line = raw.trim();
    if (!line) continue;
    const tokens = line.split(/\s+/);
    const head = (tokens[0] ?? "").toLowerCase();

    let localAddr: string | undefined;
    let processName: string | null = null;
    let pid: number | null = null;

    if (head === "listen") {
      // ss:  STATE Recv-Q Send-Q  Local:Port  Peer:Port  [users:(("p",pid=N,…))]
      localAddr = tokens[3];
      const m = line.match(/\(\("([^"]+)",pid=(\d+)/);
      if (m) {
        processName = m[1];
        pid = Number(m[2]);
      }
    } else if (head.startsWith("tcp") && tokens.includes("LISTEN")) {
      // netstat: proto Recv-Q Send-Q  Local  Foreign  State  PID/Program
      localAddr = tokens[3];
      const m = (tokens[tokens.length - 1] ?? "").match(/^(\d+)\/(.+)$/);
      if (m) {
        pid = Number(m[1]);
        processName = m[2];
      }
    } else {
      continue;
    }

    if (!localAddr) continue;
    const colon = localAddr.lastIndexOf(":");
    if (colon < 0) continue;
    const port = Number(localAddr.slice(colon + 1));
    if (!Number.isInteger(port) || port <= 0 || port > 65535) continue;
    const address = localAddr.slice(0, colon) || "*";

    const existing = byPort.get(port);
    if (existing) {
      if (!existing.processName && processName) {
        existing.processName = processName;
        existing.pid = pid;
      }
      if (isLoopback(existing.address) && !isLoopback(address)) {
        existing.address = address;
      }
    } else {
      byPort.set(port, { port, address, processName, pid });
    }
  }

  return [...byPort.values()].sort((a, b) => a.port - b.port);
}
