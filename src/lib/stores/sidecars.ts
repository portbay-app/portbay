/**
 * Sidecars store — polls `sidecar_status` every 3 s and exposes the
 * latest snapshot to all consumers (the dashboard row, the sidebar
 * footer pill, the settings page once it grows a sidecar section).
 *
 * Real-time push events for sidecar transitions are out of scope for
 * Phase 2 — the existing `portbay://status` channel is project-scoped.
 * A sidecar-scoped channel arrives with the reconcile loop expansion.
 */
import { browser } from "$app/environment";

import { safeInvoke } from "$lib/ipc";
import type { SidecarHealth, SidecarStatus } from "$lib/types/sidecars";

const POLL_INTERVAL_MS = 3_000;

const PLACEHOLDER: SidecarStatus = {
  name: "—",
  status: "stopped",
  detail: "loading…",
};

const INITIAL: SidecarHealth = {
  processCompose: { ...PLACEHOLDER, name: "process-compose" },
  caddy: { ...PLACEHOLDER, name: "caddy" },
  mkcertCa: { ...PLACEHOLDER, name: "mkcert" },
  hostsHelper: { ...PLACEHOLDER, name: "hosts" },
};

function createSidecarsStore() {
  let snapshot = $state<SidecarHealth>(INITIAL);
  let loading = $state<boolean>(false);
  let lastErrorAt = $state<number | null>(null);
  let timer: ReturnType<typeof setInterval> | null = null;

  async function refresh(): Promise<void> {
    if (!browser) return;
    loading = true;
    try {
      snapshot = await safeInvoke<SidecarHealth>("sidecar_status");
      lastErrorAt = null;
    } catch {
      // safeInvoke already pushed the toast; record so the UI can dim the
      // row if we want a "stale" affordance later.
      lastErrorAt = Date.now();
    } finally {
      loading = false;
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
      return snapshot;
    },
    get loading() {
      return loading;
    },
    get lastErrorAt() {
      return lastErrorAt;
    },
    refresh,
    start,
    stop,
  };
}

export const sidecars = createSidecarsStore();
