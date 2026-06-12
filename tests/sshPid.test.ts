/**
 * PID hygiene (2026-06-10 SSH assessment, P3): PIDs parsed from remote `ps`
 * output are interpolated into kill / for-loop / grep commands, so anything
 * but pure digits must be rejected before it reaches a command line.
 */
import { describe, expect, it } from "vitest";
import { isValidPid } from "$lib/ssh/pid";

describe("isValidPid", () => {
  it("accepts pure-digit PIDs", () => {
    for (const pid of ["1", "42", "31337", "4194304"]) {
      expect(isValidPid(pid), pid).toBe(true);
    }
  });

  it("rejects anything that could escape a shell interpolation", () => {
    const hostile = [
      "",
      " 42",
      "42 ",
      "-9",
      "42; rm -rf /",
      "$(reboot)",
      "`id`",
      "42|cat",
      "4 2",
      "42\n43",
      "1e3",
      "0x1f",
      "PID",
      "４２", // full-width digits — \d in JS is ASCII-only, but pin it
    ];
    for (const pid of hostile) {
      expect(isValidPid(pid), JSON.stringify(pid)).toBe(false);
    }
  });
});
