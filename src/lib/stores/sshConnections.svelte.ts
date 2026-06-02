import { browser } from "$app/environment";

import { invokeQuiet, safeInvoke } from "$lib/ipc";
import { connectWithPrompt } from "$lib/ssh/connectWithPrompt";
import { errorBus } from "$lib/stores/errors.svelte";
import type {
  SaveSshConnectionInput,
  SshConfigCandidate,
  SshConnectionView,
} from "$lib/types/sshConnections";

/**
 * The saved SSH hosts behind the connections dashboard. Unlike the tunnels
 * store this isn't polled — connections only change on explicit add/edit/delete,
 * so callers `refresh()` once on mount and after each mutation.
 */
function createSshConnectionsStore() {
  let entries = $state<SshConnectionView[]>([]);
  let loaded = $state(false);
  let busy = $state<Record<string, boolean>>({});

  /**
   * Guarantee the array fields the UI treats as always-present. The backend
   * omits `tags` from the JSON when it's empty (`skip_serializing_if`), so a
   * tagless host (e.g. one imported from ~/.ssh/config) would otherwise arrive
   * with `tags === undefined` and crash `host.tags.length` in the detail view.
   */
  function normalize(c: SshConnectionView): SshConnectionView {
    // The backend omits absent optional metadata (`skip_serializing_if`), so
    // coerce the fields the UI reads as nullable to explicit `null`.
    return {
      ...c,
      tags: c.tags ?? [],
      stage: c.stage ?? null,
      region: c.region ?? null,
      provider: c.provider ?? null,
      createdAt: c.createdAt ?? null,
    };
  }

  async function refresh(): Promise<void> {
    if (!browser) return;
    try {
      const list = await safeInvoke<SshConnectionView[]>("ssh_connections_list");
      entries = list.map(normalize);
      loaded = true;
    } catch {
      /* safeInvoke pushed the toast */
    }
  }

  function setBusy(id: string, value: boolean) {
    busy = { ...busy, [id]: value };
  }

  function isBusy(id: string): boolean {
    return busy[id] === true;
  }

  async function save(
    input: SaveSshConnectionInput,
  ): Promise<SshConnectionView | null> {
    const key = input.id ?? "__new";
    if (isBusy(key)) return null;
    setBusy(key, true);
    try {
      const saved = await safeInvoke<SshConnectionView>("ssh_connection_save", {
        input,
      });
      await refresh();
      return normalize(saved);
    } catch {
      return null;
    } finally {
      setBusy(key, false);
    }
  }

  async function remove(id: string): Promise<boolean> {
    if (isBusy(id)) return false;
    setBusy(id, true);
    try {
      await safeInvoke("ssh_connection_delete", { id });
      await refresh();
      return true;
    } catch {
      return false;
    } finally {
      setBusy(id, false);
    }
  }

  async function detectOs(id: string): Promise<void> {
    const key = `${id}:os`;
    if (isBusy(key)) return;
    setBusy(key, true);
    try {
      const host = entries.find((c) => c.id === id);
      const label = host
        ? host.sshUser
          ? `${host.sshUser}@${host.sshHost}`
          : host.sshHost
        : id;
      const os = await connectWithPrompt(id, label, (cred) =>
        invokeQuiet<string>("ssh_connection_detect_os", {
          id,
          password: cred?.kind === "password" ? cred.secret : undefined,
          passphrase: cred?.kind === "passphrase" ? cred.secret : undefined,
        }),
      );
      await refresh();
      errorBus.push({
        code: "SSH_OS_DETECTED",
        category: "infrastructure",
        whatHappened: `Detected ${os}.`,
        whyItMatters: "The host's OS is cached on its dashboard card.",
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });
    } catch {
      /* toast already pushed */
    } finally {
      setBusy(key, false);
    }
  }

  /**
   * Parse `~/.ssh/config` and return the importable host candidates for a
   * preview. Read-only — nothing is saved until the user picks and `save()`s.
   */
  async function importConfig(): Promise<SshConfigCandidate[]> {
    if (!browser) return [];
    try {
      return await safeInvoke<SshConfigCandidate[]>("ssh_config_import");
    } catch {
      return [];
    }
  }

  /** Stamp a host as just-used (fire-and-forget; ordering only). */
  async function touch(id: string): Promise<void> {
    if (!browser) return;
    try {
      await safeInvoke("ssh_connection_touch", { id });
    } catch {
      /* non-critical */
    }
  }

  return {
    get value() {
      return entries;
    },
    get loaded() {
      return loaded;
    },
    get count() {
      return entries.length;
    },
    find(id: string): SshConnectionView | undefined {
      return entries.find((c) => c.id === id);
    },
    isBusy,
    refresh,
    save,
    remove,
    detectOs,
    importConfig,
    touch,
  };
}

export const sshConnections = createSshConnectionsStore();
