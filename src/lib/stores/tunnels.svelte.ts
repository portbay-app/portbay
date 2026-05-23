/**
 * Active-tunnels store — the single source of truth for Cloudflare
 * tunnel state and lifecycle. Polls `list_tunnels` every 5 s so the
 * sidebar/TopBar stay in sync with backend reality, and exposes
 * `share`/`stopSharing` so the /tunnels page (and anything else) starts
 * and stops tunnels through one place. Every action refreshes the
 * snapshot synchronously, so the UI never shows stale lifecycle state.
 */
import { browser } from "$app/environment";

import { safeInvoke } from "$lib/ipc";
import { errorBus } from "$lib/stores/errors.svelte";
import type { TunnelStatus } from "$lib/types/tunnel";

const POLL_INTERVAL_MS = 5_000;

function createTunnelsStore() {
  let entries = $state<TunnelStatus[]>([]);
  let timer: ReturnType<typeof setInterval> | null = null;
  /** Per-project busy markers while a start/stop request is in flight. */
  let busy = $state<Record<string, boolean>>({});

  async function refresh(): Promise<void> {
    if (!browser) return;
    try {
      entries = await safeInvoke<TunnelStatus[]>("list_tunnels");
    } catch {
      // safeInvoke toasts on its own; leave the last-known snapshot
      // in place so the UI doesn't flicker on transient errors.
    }
  }

  function start() {
    if (!browser || timer !== null) return;
    void refresh();
    timer = setInterval(() => void refresh(), POLL_INTERVAL_MS);
  }

  function stop() {
    if (timer !== null) {
      clearInterval(timer);
      timer = null;
    }
  }

  /** The active tunnel for a project, or null when it isn't shared. */
  function statusFor(projectId: string): TunnelStatus | null {
    return entries.find((t) => t.projectId === projectId) ?? null;
  }

  function isBusy(projectId: string): boolean {
    return busy[projectId] === true;
  }

  function setBusy(projectId: string, value: boolean) {
    busy = { ...busy, [projectId]: value };
  }

  /**
   * Start sharing a project publicly. The backend resolves the project's
   * URL itself, spawns cloudflared, and blocks until the public
   * trycloudflare.com URL is announced; we then refresh the snapshot.
   */
  async function share(projectId: string): Promise<void> {
    if (isBusy(projectId)) return;
    setBusy(projectId, true);
    try {
      await safeInvoke("start_tunnel", { id: projectId });
      await refresh();
      const url = statusFor(projectId)?.publicUrl;
      errorBus.push({
        code: "TUNNEL_STARTED",
        whatHappened: "Public tunnel is live.",
        whyItMatters: url
          ? `Anyone with the URL can reach this project at ${url}.`
          : "Cloudflare is assigning a public URL — it'll appear in a moment.",
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });
    } catch {
      /* safeInvoke pushed the error toast */
    } finally {
      setBusy(projectId, false);
    }
  }

  /** Stop sharing a project — kills its cloudflared child. */
  async function stopSharing(projectId: string): Promise<void> {
    if (isBusy(projectId)) return;
    setBusy(projectId, true);
    try {
      await safeInvoke("stop_tunnel", { id: projectId });
      await refresh();
    } catch {
      /* toast already pushed */
    } finally {
      setBusy(projectId, false);
    }
  }

  return {
    get value() {
      return entries;
    },
    get count() {
      return entries.length;
    },
    statusFor,
    isBusy,
    refresh,
    start,
    stop,
    share,
    stopSharing,
  };
}

export const tunnels = createTunnelsStore();
