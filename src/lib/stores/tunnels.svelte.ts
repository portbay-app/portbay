/**
 * Active-tunnels store. Polls `list_tunnels` every 5 s so the TopBar
 * pill stays in sync with backend reality. Updates are also pushed
 * synchronously after every start/stop the user triggers.
 */
import { browser } from "$app/environment";

import { safeInvoke } from "$lib/ipc";
import type { TunnelStatus } from "$lib/types/tunnel";

const POLL_INTERVAL_MS = 5_000;

function createTunnelsStore() {
  let entries = $state<TunnelStatus[]>([]);
  let timer: ReturnType<typeof setInterval> | null = null;

  async function refresh(): Promise<void> {
    if (!browser) return;
    try {
      entries = await safeInvoke<TunnelStatus[]>("list_tunnels");
    } catch {
      // safeInvoke toasts on its own; leave the last-known snapshot
      // in place so the pill doesn't flicker on transient errors.
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

  return {
    get value() {
      return entries;
    },
    get count() {
      return entries.length;
    },
    refresh,
    start,
    stop,
  };
}

export const tunnels = createTunnelsStore();
