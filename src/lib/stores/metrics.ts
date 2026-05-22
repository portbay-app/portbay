/**
 * Metrics store — subscribes to `portbay://metrics` events from the Rust
 * background poller (2s cadence). Also retains a rolling history of the
 * last N CPU samples for the sparkline.
 */
import { browser } from "$app/environment";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { safeInvoke } from "$lib/ipc";
import type { SystemMetrics } from "$lib/types/metrics";

const CHANNEL = "portbay://metrics";
const HISTORY_LENGTH = 60; // 60 × 2s = 2 minutes of history

function createMetricsStore() {
  let snapshot = $state<SystemMetrics | null>(null);
  let cpuHistory = $state<number[]>([]);
  let unlisten: UnlistenFn | null = null;

  function apply(m: SystemMetrics) {
    snapshot = m;
    const next = [...cpuHistory, m.cpu.total];
    cpuHistory = next.length > HISTORY_LENGTH
      ? next.slice(next.length - HISTORY_LENGTH)
      : next;
  }

  async function start() {
    if (!browser) return;
    if (unlisten) return;
    // Seed with a one-shot fetch so the cards render before the first
    // event tick. Failures fall through silently (the event will arrive).
    try {
      const initial = await safeInvoke<SystemMetrics>("system_metrics");
      apply(initial);
    } catch {
      /* event will catch up */
    }
    unlisten = await listen<SystemMetrics>(CHANNEL, (e) => apply(e.payload));
  }

  function stop() {
    if (unlisten) {
      unlisten();
      unlisten = null;
    }
  }

  return {
    get value() {
      return snapshot;
    },
    get cpuHistory() {
      return cpuHistory;
    },
    start,
    stop,
  };
}

export const metrics = createMetricsStore();
