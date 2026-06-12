/**
 * Mobile run-phase store — live sub-state for mobile projects, keyed by
 * project id. Hydrates once from `get_mobile_phases` (so a UI reload mid-run
 * doesn't lose the phase), then patches in place on every
 * `portbay://mobile-phase` event.
 *
 * Also keeps a small per-project ring of recent transitions, which the rail's
 * Recent Activity section renders for mobile kinds (the projects store keeps
 * no history).
 */
import { browser } from "$app/environment";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { safeInvoke } from "$lib/ipc";
import type {
  MobilePhase,
  MobilePhaseEvent,
  MobilePhaseInfo,
} from "$lib/types/mobile";

const PHASE_CHANNEL = "portbay://mobile-phase";

/** Transitions kept per project for the rail's activity feed. */
const HISTORY_LIMIT = 6;

export interface PhaseEntry {
  phase: MobilePhase;
  detail?: string;
  /** Epoch ms when the phase was entered (drives the elapsed counter). */
  since: number;
}

export interface PhaseTransition {
  phase: MobilePhase;
  detail?: string;
  ts: number;
}

function createMobilePhaseStore() {
  let items = $state<Record<string, PhaseEntry>>({});
  let history = $state<Record<string, PhaseTransition[]>>({});
  let unlisten: UnlistenFn | null = null;

  function apply(ev: MobilePhaseEvent) {
    if (ev.phase === null) {
      if (ev.id in items) {
        const { [ev.id]: _, ...rest } = items;
        items = rest;
      }
      return;
    }
    items = {
      ...items,
      [ev.id]: { phase: ev.phase, detail: ev.detail, since: ev.ts },
    };
    const ring = history[ev.id] ?? [];
    history = {
      ...history,
      [ev.id]: [
        { phase: ev.phase, detail: ev.detail, ts: ev.ts },
        ...ring,
      ].slice(0, HISTORY_LIMIT),
    };
  }

  async function start(): Promise<void> {
    if (!browser) return;
    if (unlisten) return;
    unlisten = await listen<MobilePhaseEvent>(PHASE_CHANNEL, (e) => {
      apply(e.payload);
    });
    // Hydrate after subscribing so no transition can fall in the gap.
    try {
      const snapshot = await safeInvoke<Record<string, MobilePhaseInfo>>(
        "get_mobile_phases",
      );
      const now = Date.now();
      for (const [id, info] of Object.entries(snapshot)) {
        if (!(id in items)) {
          items = {
            ...items,
            [id]: { phase: info.phase, detail: info.detail, since: now },
          };
        }
      }
    } catch {
      // safeInvoke already surfaced the failure; live events still work.
    }
  }

  function stop() {
    if (unlisten) {
      unlisten();
      unlisten = null;
    }
  }

  return {
    get value() {
      return items;
    },
    /** Current phase entry for a project, or null. */
    get(id: string): PhaseEntry | null {
      return items[id] ?? null;
    },
    /** Recent transitions, newest first. */
    historyOf(id: string): PhaseTransition[] {
      return history[id] ?? [];
    },
    start,
    stop,
  };
}

export const mobilePhase = createMobilePhaseStore();
