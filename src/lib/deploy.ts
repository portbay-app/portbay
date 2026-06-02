/**
 * Typed wrappers over the project-deploy + local-filesystem IPC commands.
 *
 * `projectDeployRun` goes through `connectWithPrompt` so a host needing a
 * password/passphrase is asked once (the secret is cached for the session and
 * passed inline, never stored), exactly like the ad-hoc deploy path.
 */
import { invokeQuiet, safeInvoke } from "$lib/ipc";
import { connectWithPrompt } from "$lib/ssh/connectWithPrompt";
import type { DeployRunResult, ProjectDeploy, ProjectView } from "$lib/types/projects";

/** One local file/dir entry from `local_list_dir` / `local_stat`. */
export interface LocalEntry {
  name: string;
  path: string;
  isDir: boolean;
  size: number;
}

/** The project's saved deploy config, or `null` when none is set. */
export function projectGetDeploy(id: string): Promise<ProjectDeploy | null> {
  return safeInvoke<ProjectDeploy | null>("project_get_deploy", { id });
}

/** Persist (or clear, with `null`) a project's deploy config. */
export function projectSetDeploy(
  id: string,
  deploy: ProjectDeploy | null,
): Promise<ProjectView> {
  return safeInvoke<ProjectView>("project_set_deploy", { id, deploy });
}

/**
 * Run a project's configured deploy: sync files to the host, then run the
 * steps. Prompts once for a credential if the deploy host needs one.
 */
export function projectDeployRun(
  projectId: string,
  connectionId: string,
  hostLabel: string,
): Promise<DeployRunResult> {
  return connectWithPrompt(connectionId, hostLabel, (cred) =>
    invokeQuiet<DeployRunResult>("project_deploy_run", {
      input: {
        projectId,
        password: cred?.kind === "password" ? cred.secret : undefined,
        passphrase: cred?.kind === "passphrase" ? cred.secret : undefined,
      },
    }),
  );
}

/** List a local directory (dirs first, then name). */
export function localListDir(path: string): Promise<LocalEntry[]> {
  return safeInvoke<LocalEntry[]>("local_list_dir", { path });
}

export function localStat(path: string): Promise<LocalEntry> {
  return safeInvoke<LocalEntry>("local_stat", { path });
}
