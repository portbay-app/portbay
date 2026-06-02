import { describe, expect, it } from "vitest";

import { parseListeningPorts, wellKnownLabel } from "../src/lib/ssh/portScan";

describe("parseListeningPorts — ss output", () => {
  // `ss -tlnpH` (no header), with and without process info.
  const ss = [
    'LISTEN 0      128          0.0.0.0:22         0.0.0.0:*    users:(("sshd",pid=812,fd=3))',
    'LISTEN 0      128             [::]:22            [::]:*     users:(("sshd",pid=812,fd=3))',
    'LISTEN 0      511          127.0.0.1:3000       0.0.0.0:*   users:(("node",pid=2233,fd=18))',
    "LISTEN 0      4096         127.0.0.1:5432       0.0.0.0:*",
  ].join("\n");

  it("parses ports, addresses, and process info", () => {
    const ports = parseListeningPorts(ss);
    expect(ports.map((p) => p.port)).toEqual([22, 3000, 5432]);

    const node = ports.find((p) => p.port === 3000)!;
    expect(node.processName).toBe("node");
    expect(node.pid).toBe(2233);
    expect(node.address).toBe("127.0.0.1");
  });

  it("merges the IPv4 + IPv6 rows of one service into a single entry", () => {
    const ports = parseListeningPorts(ss);
    expect(ports.filter((p) => p.port === 22)).toHaveLength(1);
  });

  it("leaves process info null when the host did not report it", () => {
    const pg = parseListeningPorts(ss).find((p) => p.port === 5432)!;
    expect(pg.processName).toBeNull();
    expect(pg.pid).toBeNull();
  });
});

describe("parseListeningPorts — netstat output", () => {
  const netstat = [
    "Active Internet connections (only servers)",
    "Proto Recv-Q Send-Q Local Address           Foreign Address         State       PID/Program name",
    "tcp        0      0 0.0.0.0:22              0.0.0.0:*               LISTEN      812/sshd",
    "tcp6       0      0 :::22                   :::*                    LISTEN      812/sshd",
    "tcp        0      0 127.0.0.1:6379          0.0.0.0:*               LISTEN      2901/redis-server",
    "tcp        0      0 127.0.0.1:5432          0.0.0.0:*               LISTEN      -",
  ].join("\n");

  it("parses ports and PID/Program, skipping headers and non-LISTEN lines", () => {
    const ports = parseListeningPorts(netstat);
    expect(ports.map((p) => p.port)).toEqual([22, 5432, 6379]);

    const redis = ports.find((p) => p.port === 6379)!;
    expect(redis.processName).toBe("redis-server");
    expect(redis.pid).toBe(2901);
  });

  it("treats an unprivileged `-` program column as no process", () => {
    const pg = parseListeningPorts(netstat).find((p) => p.port === 5432)!;
    expect(pg.processName).toBeNull();
    expect(pg.pid).toBeNull();
  });
});

describe("parseListeningPorts — edge cases", () => {
  it("returns an empty list for empty or junk input", () => {
    expect(parseListeningPorts("")).toEqual([]);
    expect(parseListeningPorts("bash: ss: command not found\n")).toEqual([]);
  });

  it("handles wildcard binds (`*:port`)", () => {
    const ports = parseListeningPorts("LISTEN 0 128 *:80 *:*");
    expect(ports).toEqual([{ port: 80, address: "*", processName: null, pid: null }]);
  });

  it("ignores out-of-range ports", () => {
    expect(parseListeningPorts("LISTEN 0 128 0.0.0.0:70000 0.0.0.0:*")).toEqual([]);
  });

  it("prefers a broader bind address when a service binds both loopback and all-interfaces", () => {
    const both = [
      "LISTEN 0 128 127.0.0.1:8080 0.0.0.0:*",
      "LISTEN 0 128 0.0.0.0:8080 0.0.0.0:*",
    ].join("\n");
    expect(parseListeningPorts(both)).toEqual([
      { port: 8080, address: "0.0.0.0", processName: null, pid: null },
    ]);
  });
});

describe("wellKnownLabel", () => {
  it("labels common and ML-tooling ports", () => {
    expect(wellKnownLabel(5432)).toBe("PostgreSQL");
    expect(wellKnownLabel(8888)).toBe("Jupyter");
    expect(wellKnownLabel(6006)).toBe("TensorBoard");
  });

  it("returns null for an unknown port", () => {
    expect(wellKnownLabel(54321)).toBeNull();
  });
});
