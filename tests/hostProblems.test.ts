import { describe, expect, it } from "vitest";

import { HOST_PROBLEMS_PROBE, parseHostProblems } from "../src/lib/ssh/hostProblems";

// Build a combined probe stdout the way the bundled command lays it out: each
// section under its marker. Omitting a section's body simulates a probe that
// wasn't available on the host (e.g. systemctl/`/proc` missing on macOS).
function probeOutput(parts: { failed?: string; disk?: string; load?: string; mem?: string }): string {
  return [
    "@@PB_FAILED@@",
    parts.failed ?? "",
    "@@PB_DISK@@",
    parts.disk ?? "",
    "@@PB_LOAD@@",
    parts.load ?? "",
    "@@PB_MEM@@",
    parts.mem ?? "",
    "@@PB_END@@",
  ].join("\n");
}

describe("parseHostProblems", () => {
  it("reports failed systemd units as errors", () => {
    const out = probeOutput({
      failed: ["nginx.service loaded failed failed A high performance web server", "redis.service loaded failed failed Redis"].join("\n"),
    });
    const problems = parseHostProblems(out);
    expect(problems.map((p) => p.title)).toEqual(["nginx.service failed", "redis.service failed"]);
    expect(problems.every((p) => p.severity === "error" && p.source === "systemd")).toBe(true);
    expect(problems[0].hint).toContain("systemctl status nginx.service");
  });

  it("ignores systemctl footer lines that aren't units", () => {
    const out = probeOutput({ failed: "0 loaded units listed." });
    expect(parseHostProblems(out)).toEqual([]);
  });

  it("flags disk pressure by capacity threshold and skips pseudo filesystems", () => {
    const disk = [
      "Filesystem     1024-blocks      Used Available Capacity Mounted on",
      "/dev/sda1        103081248  98000000   5081248      96% /",
      "/dev/sdb1        103081248  90000000  13081248      88% /data",
      "/dev/sdc1        103081248  10000000  93081248      10% /spare",
      "tmpfs              8167840         0   8167840     100% /dev/shm",
    ].join("\n");
    const problems = parseHostProblems(probeOutput({ disk }));
    // Root (96% → error) and /data (88% → warning); /spare under threshold and
    // tmpfs (pseudo) are both excluded.
    expect(problems.map((p) => `${p.title} [${p.severity}]`)).toEqual([
      "/ is 96% full [error]",
      "/data is 88% full [warning]",
    ]);
    expect(problems[0].detail).toContain("on /dev/sda1");
  });

  it("flags load only when it exceeds the CPU count, scaled by nproc", () => {
    // load1 9.5 over 4 CPUs → ratio 2.375 → warning.
    const warn = parseHostProblems(probeOutput({ load: "9.50 7.20 5.10 3/512 9001\n4" }));
    expect(warn).toHaveLength(1);
    expect(warn[0]).toMatchObject({ source: "load", severity: "warning" });

    // load1 20 over 4 CPUs → ratio 5 → error.
    const err = parseHostProblems(probeOutput({ load: "20.00 18.00 12.00 9/512 9001\n4" }));
    expect(err[0].severity).toBe("error");

    // load1 1.0 over 4 CPUs → healthy → nothing.
    expect(parseHostProblems(probeOutput({ load: "1.00 0.80 0.50 1/512 9001\n4" }))).toEqual([]);
  });

  it("flags memory pressure from MemAvailable/MemTotal", () => {
    const mem = ["MemTotal:       16384000 kB", "MemFree:          200000 kB", "MemAvailable:     600000 kB"].join("\n");
    const problems = parseHostProblems(probeOutput({ mem }));
    expect(problems).toHaveLength(1);
    // 600000/16384000 ≈ 3.7% → error.
    expect(problems[0]).toMatchObject({ source: "memory", severity: "error" });
  });

  it("returns nothing for a healthy host with unavailable probes", () => {
    expect(parseHostProblems(probeOutput({}))).toEqual([]);
  });

  it("probe command bundles every section marker in order", () => {
    expect(HOST_PROBLEMS_PROBE.indexOf("@@PB_FAILED@@")).toBeLessThan(HOST_PROBLEMS_PROBE.indexOf("@@PB_DISK@@"));
    expect(HOST_PROBLEMS_PROBE).toContain("systemctl --failed");
    expect(HOST_PROBLEMS_PROBE).toContain("df -P -k");
  });
});
