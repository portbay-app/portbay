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
import type { DeployEvent } from "$lib/types/sshTunnels";

/**
 * Subscribe to live progress for one deploy run (`runId` is caller-generated;
 * events for other runs on the shared channel are filtered out). Returns the
 * unlisten function — call it once the run's promise settles.
 */
export async function listenDeploy(
  runId: string,
  onEvent: (ev: DeployEvent) => void,
): Promise<() => void> {
  const { listen } = await import("@tauri-apps/api/event");
  return listen<DeployEvent>(
    "portbay://deploy",
    (ev) => {
      if (ev.payload.runId === runId) onEvent(ev.payload);
    },
    // The backend emits deploy progress point-to-point to the main window
    // (chunks are raw remote output); a targeted emit skips untargeted
    // listeners.
    { target: "main" },
  );
}

/** Flag an in-flight deploy run for cancellation (skips queued steps and
    best-effort kills the running command). */
export function cancelDeploy(runId: string): Promise<void> {
  return safeInvoke<void>("ssh_deploy_cancel", { runId });
}

/** One local file/dir entry from `local_list_dir` / `local_stat`. */
export interface LocalEntry {
  name: string;
  path: string;
  isDir: boolean;
  size: number;
}

/** One file from `local_walk_files`: absolute path + POSIX path relative to
    the walked root (so it maps cleanly onto a remote destination). */
export interface WalkedLocalFile {
  path: string;
  rel: string;
  size: number;
}

/** Recursively enumerate every file under a local folder (for folder upload). */
export function localWalkFiles(root: string): Promise<WalkedLocalFile[]> {
  return safeInvoke<WalkedLocalFile[]>("local_walk_files", { root });
}

/** Result of a recursive local name search (`local_search`). */
export interface LocalSearchResult {
  entries: LocalEntry[];
  scanned: number;
  /** True when the walk stopped at a result/scan/depth cap. */
  truncated: boolean;
}

/** Recursive name search under a local folder — plain text is a substring
    match, `*`/`?` switch to a glob over the whole name (`*.zip`). */
export function localSearch(root: string, query: string): Promise<LocalSearchResult> {
  return safeInvoke<LocalSearchResult>("local_search", { root, query });
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
 * steps. Prompts once for a credential if the deploy host needs one. With a
 * `runId`, live sync/step progress streams on `portbay://deploy` (subscribe
 * via `listenDeploy`) and the run is cancellable through `cancelDeploy`.
 */
export function projectDeployRun(
  projectId: string,
  connectionId: string,
  hostLabel: string,
  runId?: string,
): Promise<DeployRunResult> {
  return connectWithPrompt(connectionId, hostLabel, (cred) =>
    invokeQuiet<DeployRunResult>("project_deploy_run", {
      input: {
        projectId,
        runId,
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
