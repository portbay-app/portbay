/**
 * Per-project CPU history — feeds the tray-panel sparklines.
 *
 * The status poller emits `portbay://status` events at 1.5 s cadence
 * with `runtime.cpuPercent` for every project that's transitioned or
 * is still in a non-stopped state. This store accumulates the last
 * `HISTORY_LENGTH` samples per project id so a 36-px wide SVG
 * sparkline always has data to render.
 *
 * Separate from the main `projects` store on purpose:
 *   - the projects store keeps the *current* runtime only;
 *   - this store keeps a rolling buffer;
 *   - the popover panel needs both but they have different lifetimes
 *     (the projects store lives on the main window; this one is
 *     popover-scoped).
 */
import { browser } from "$app/environment";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type { ProjectStatusEvent } from "$lib/types/projects";

const CHANNEL = "portbay://status";
/** 30 samples × 1.5 s poller interval = ~45 s of history per spark. */
const HISTORY_LENGTH = 30;

function createProjectCpuStore() {
  let buffers = $state<Record<string, number[]>>({});
  let unlisten: UnlistenFn | null = null;

  function append(id: string, value: number) {
    const prev = buffers[id] ?? [];
    const next =
      prev.length >= HISTORY_LENGTH
        ? [...prev.slice(prev.length - HISTORY_LENGTH + 1), value]
        : [...prev, value];
    buffers = { ...buffers, [id]: next };
  }

  function apply(ev: ProjectStatusEvent) {
    // Only meaningful when the process is alive; a stopped event with
    // no runtime drops the trailing values (next start will rebuild).
    if (ev.runtime) {
      append(ev.id, ev.runtime.cpuPercent);
    } else {
      // Process gone — clear so a future sparkline doesn't render
      // stale data when the project comes back.
      if (ev.id in buffers) {
        const { [ev.id]: _, ...rest } = buffers;
        buffers = rest;
      }
    }
  }

  async function start() {
    if (!browser) return;
    if (unlisten) return;
    unlisten = await listen<ProjectStatusEvent>(CHANNEL, (e) => apply(e.payload));
  }

  function stop() {
    unlisten?.();
    unlisten = null;
  }

  function historyFor(id: string): number[] {
    return buffers[id] ?? [];
  }

  return {
    start,
    stop,
    historyFor,
  };
}

export const projectCpu = createProjectCpuStore();
