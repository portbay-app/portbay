/**
 * Auto-update store — thin frontend over the Rust `check_for_update` /
 * `install_update` commands (which wrap tauri-plugin-updater).
 *
 * Boot flow: the root layout calls `check({ silent: true })` once on mount.
 * When a newer signed release is published, `notifyAvailable` pushes a
 * non-blocking info toast with "Update now" (→ `install_update`) and
 * "Release notes" (→ the GitHub releases page). The Settings → Updates
 * section drives the same `check()` / `install()` manually.
 *
 * No-op in the hosted web demo (`PUBLIC_SIMULATOR === "true"`) — there's no
 * Tauri runtime there, so invoking the commands would only throw.
 */
import { safeInvoke, invokeQuiet } from "$lib/ipc";
import { errorBus } from "./errors.svelte";

/** Mirrors `commands::updater::UpdateInfo` (serde camelCase). */
export interface UpdateInfo {
  version: string;
  currentVersion: string;
  notes: string | null;
  pubDate: string | null;
}

export type UpdaterStatus =
  | "idle"
  | "checking"
  | "available"
  | "uptodate"
  | "installing"
  | "error";

const RELEASES_URL = "https://github.com/portbay-app/portbay/releases/latest";

function isSimulator(): boolean {
  return import.meta.env.PUBLIC_SIMULATOR === "true";
}

function createUpdaterStore() {
  let status = $state<UpdaterStatus>("idle");
  let available = $state<UpdateInfo | null>(null);
  let lastChecked = $state<number | null>(null);

  function notifyAvailable(info: UpdateInfo): void {
    errorBus.push({
      code: "UPDATE_AVAILABLE",
      category: "updates",
      whatHappened: `PortBay ${info.version} is ready.`,
      whyItMatters: "Update to get the latest fixes and features.",
      whoCausedIt: "system",
      severity: "info",
      // The toast has actions, so it stays until the user acts (errorBus
      // only auto-dismisses action-less toasts). "Update now" routes through
      // the same safeInvoke path as every other command button.
      actions: [
        { label: "Update now", command: "install_update" },
        { label: "Release notes", url: RELEASES_URL },
      ],
    });
  }

  /**
   * Check the configured endpoint for a newer release.
   * `silent` suppresses the failure toast (used by the boot check, where a
   * transient network blip shouldn't nag the user) and surfaces the
   * "update available" toast on success.
   */
  async function check({ silent = false }: { silent?: boolean } = {}): Promise<void> {
    if (isSimulator()) return;
    status = "checking";
    try {
      const info = silent
        ? await invokeQuiet<UpdateInfo | null>("check_for_update")
        : await safeInvoke<UpdateInfo | null>("check_for_update");
      lastChecked = Date.now();
      if (info) {
        available = info;
        status = "available";
        if (silent) notifyAvailable(info);
      } else {
        available = null;
        status = "uptodate";
      }
    } catch {
      // Non-silent path already toasted via safeInvoke.
      status = "error";
    }
  }

  /**
   * Download, verify, install, and relaunch. On success the process is
   * replaced by `app.restart()`, so this never resolves; on failure
   * safeInvoke surfaces the envelope and we drop back to `error`.
   */
  async function install(): Promise<void> {
    if (isSimulator() || !available) return;
    status = "installing";
    try {
      await safeInvoke("install_update");
    } catch {
      status = "error";
    }
  }

  return {
    get status() {
      return status;
    },
    get available() {
      return available;
    },
    get lastChecked() {
      return lastChecked;
    },
    get releasesUrl() {
      return RELEASES_URL;
    },
    check,
    install,
  };
}

export const updater = createUpdaterStore();
