/**
 * Typed wrappers over the remote-exec / deploy IPC commands.
 */
import { safeInvoke } from "$lib/ipc";
import type { StepResult } from "$lib/types/sshTunnels";

export interface ExecResult {
  stdout: string;
  stderr: string;
  exitCode: number;
}

/** Run a single command on the remote host. */
export function sshExecRun(
  connectionId: string,
  command: string,
  cwd?: string,
): Promise<ExecResult> {
  return safeInvoke<ExecResult>("ssh_exec_run", {
    input: { connectionId, command, cwd: cwd || null },
  });
}

/** Run an ordered deploy sequence; the backend stops at the first failing step. */
export function sshDeployRun(
  connectionId: string,
  steps: string[],
  cwd?: string,
): Promise<StepResult[]> {
  return safeInvoke<StepResult[]>("ssh_deploy_run", {
    input: { connectionId, steps, cwd: cwd || null },
  });
}
