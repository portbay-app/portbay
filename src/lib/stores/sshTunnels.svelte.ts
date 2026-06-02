import { browser } from "$app/environment";

import { invokeQuiet, safeInvoke } from "$lib/ipc";
import { connectWithPrompt } from "$lib/ssh/connectWithPrompt";
import { errorBus } from "$lib/stores/errors.svelte";
import type {
  OpenSshTunnelDatabaseInput,
  SaveSshTunnelInput,
  SshTunnelRuntimeStatus,
} from "$lib/types/sshTunnels";

const POLL_INTERVAL_MS = 5_000;

function createSshTunnelsStore() {
  let entries = $state<SshTunnelRuntimeStatus[]>([]);
  let busy = $state<Record<string, boolean>>({});
  let timer: ReturnType<typeof setInterval> | null = null;

  async function refresh(): Promise<void> {
    if (!browser) return;
    try {
      entries = await safeInvoke<SshTunnelRuntimeStatus[]>("ssh_tunnel_list");
    } catch {
      /* safeInvoke pushed the toast */
    }
  }

  function startPolling() {
    if (!browser || timer !== null) return;
    void refresh();
    timer = setInterval(() => void refresh(), POLL_INTERVAL_MS);
  }

  function stopPolling() {
    if (timer !== null) {
      clearInterval(timer);
      timer = null;
    }
  }

  function setBusy(id: string, value: boolean) {
    busy = { ...busy, [id]: value };
  }

  function isBusy(id: string): boolean {
    return busy[id] === true;
  }

  async function save(input: SaveSshTunnelInput): Promise<SshTunnelRuntimeStatus | null> {
    const key = input.id ?? "__new";
    if (isBusy(key)) return null;
    setBusy(key, true);
    try {
      const saved = await safeInvoke<SshTunnelRuntimeStatus>("ssh_tunnel_save", {
        input,
      });
      await refresh();
      return saved;
    } catch {
      return null;
    } finally {
      setBusy(key, false);
    }
  }

  async function start(id: string): Promise<void> {
    if (isBusy(id)) return;
    setBusy(id, true);
    try {
      // Credentials are keyed by the *connection*, not the tunnel id.
      const t = entries.find((x) => x.id === id);
      const label = t
        ? t.sshUser
          ? `${t.sshUser}@${t.sshHost}`
          : t.sshHost
        : id;
      await connectWithPrompt(t?.connectionId ?? id, label, (cred) =>
        invokeQuiet("ssh_tunnel_start", {
          id,
          password: cred?.kind === "password" ? cred.secret : undefined,
        }),
      );
      await refresh();
      const tunnel = entries.find((t) => t.id === id);
      errorBus.push({
        code: "SSH_TUNNEL_STARTED",
        category: "infrastructure",
        whatHappened: "SSH tunnel is live.",
        whyItMatters: tunnel
          ? `Remote traffic is available on ${tunnel.localHost}:${tunnel.localPort}.`
          : "Remote traffic is available on localhost.",
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });
    } catch {
      /* toast already pushed */
    } finally {
      setBusy(id, false);
    }
  }

  async function stop(id: string): Promise<void> {
    if (isBusy(id)) return;
    setBusy(id, true);
    try {
      await safeInvoke("ssh_tunnel_stop", { id });
      await refresh();
    } catch {
      /* toast already pushed */
    } finally {
      setBusy(id, false);
    }
  }

  async function remove(id: string): Promise<void> {
    if (isBusy(id)) return;
    setBusy(id, true);
    try {
      await safeInvoke("ssh_tunnel_delete", { id });
      await refresh();
    } catch {
      /* toast already pushed */
    } finally {
      setBusy(id, false);
    }
  }

  async function test(id: string): Promise<void> {
    if (isBusy(id)) return;
    setBusy(id, true);
    try {
      await safeInvoke("ssh_tunnel_test", { id });
      errorBus.push({
        code: "SSH_TUNNEL_TESTED",
        category: "infrastructure",
        whatHappened: "SSH connection test passed.",
        whyItMatters: "The host accepted non-interactive key authentication.",
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });
    } catch {
      /* toast already pushed */
    } finally {
      setBusy(id, false);
    }
  }

  async function openDatabase(input: OpenSshTunnelDatabaseInput): Promise<void> {
    if (isBusy(`${input.id}:db`)) return;
    setBusy(`${input.id}:db`, true);
    try {
      await safeInvoke("ssh_tunnel_open_database", { input });
    } catch {
      /* toast already pushed */
    } finally {
      setBusy(`${input.id}:db`, false);
    }
  }

  return {
    get value() {
      return entries;
    },
    get liveCount() {
      return entries.filter((e) => e.running).length;
    },
    isBusy,
    refresh,
    startPolling,
    stopPolling,
    save,
    start,
    stop,
    remove,
    test,
    openDatabase,
  };
}

export const sshTunnels = createSshTunnelsStore();
