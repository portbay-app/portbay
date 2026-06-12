/**
 * PID hygiene for remote `ps` output. PIDs parsed from a host's `ps aux` are
 * interpolated into follow-up shell commands (`kill <pid>`, a `for p in …`
 * liveness loop, a `grep` pattern), so they must be digits and nothing else
 * before they go anywhere near a command line. Input-validation hygiene, not
 * an escalation fix — the forged output and the injected command would run on
 * the same host — but validate anyway (2026-06-10 assessment, P3).
 */
export function isValidPid(pid: string): boolean {
  return /^\d+$/.test(pid);
}
