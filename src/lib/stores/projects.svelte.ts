/**
 * Projects store — the canonical list of registered projects with live
 * status. One load on mount, then patches in place on every
 * `portbay://status` event so the table doesn't re-fetch on each tick.
 *
 * Also tracks `selectedId` for keyboard navigation in the table and
 * `lastErrors` for inline error envelopes beneath failed project rows
 * (card: P3 — Inline error rows).
 */
import { browser } from "$app/environment";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { safeInvoke } from "$lib/ipc";
import type { CommandError } from "$lib/types/error";
import type { ProjectStatusEvent, ProjectView } from "$lib/types/projects";
import type { DisplayStatus, PortbayStatus } from "$lib/types/status";
import {
  beginTransition as begin,
  clearTransition as clearT,
  onStatusEvent as foldEvent,
  optimisticDisplay,
  type TransitionKind,
  type TransitionMap,
} from "$lib/lifecycle/optimistic";

const STATUS_CHANNEL = "portbay://status";

/**
 * Safety net: if no resolving status event ever arrives (e.g. the user
 * declined a force-quit, or the backend went silent), drop the optimistic
 * overlay after this long so a row can never get wedged showing
 * "Starting…"/"Stopping…". The status poller ticks every 750 ms, so a real
 * transition resolves well inside this window.
 */
const OPTIMISTIC_TTL_MS = 12_000;

/** Build a minimal CommandError from a plain string (status-event errors). */
function syntheticEnvelope(message: string): CommandError {
  return {
    code: "PROCESS_ERROR",
    whatHappened: message,
    whyItMatters: "The project is not running as expected.",
    whoCausedIt: "system",
    actions: [],
  };
}

function createProjectsStore() {
  let items = $state<ProjectView[]>([]);
  let loading = $state<boolean>(false);
  let selectedId = $state<string | null>(null);
  /** Per-project inline error envelopes, keyed by project id. */
  let lastErrors = $state<Record<string, CommandError>>({});
  /**
   * Optimistic lifecycle overlays — what a row *displays* while a Play/Stop is
   * in flight, before the real status event lands. Logic lives in the pure
   * `lifecycle/optimistic` module; this is just the reactive holder.
   */
  let transitions = $state<TransitionMap>({});
  /** TTL timers per project (non-reactive — bookkeeping only). */
  const ttlTimers = new Map<string, ReturnType<typeof setTimeout>>();
  let unlisten: UnlistenFn | null = null;

  async function refresh(): Promise<void> {
    if (!browser) return;
    loading = true;
    try {
      items = await safeInvoke<ProjectView[]>("list_projects");
      // Selection is opt-in: only the user clicking a row opens the detail
      // rail. Drop a stale selection if the project disappeared, but never
      // auto-select on refresh.
      if (selectedId !== null && !items.some((p) => p.id === selectedId)) {
        selectedId = null;
      }
    } catch {
      // safeInvoke already pushed the toast.
    } finally {
      loading = false;
    }
  }

  function clearTtlTimer(id: string) {
    const t = ttlTimers.get(id);
    if (t !== undefined) {
      clearTimeout(t);
      ttlTimers.delete(id);
    }
  }

  /**
   * Optimistically flip a row to `starting`/`stopping` the instant a Play/Stop
   * is clicked — synchronous, so the UI responds before the IPC resolves. The
   * real status event reconciles it (see {@link applyStatusEvent}); a TTL net
   * drops it if nothing ever resolves.
   */
  function beginTransition(id: string, kind: TransitionKind) {
    clearTtlTimer(id);
    transitions = begin(transitions, id, kind);
    ttlTimers.set(
      id,
      setTimeout(() => {
        transitions = clearT(transitions, id);
        ttlTimers.delete(id);
      }, OPTIMISTIC_TTL_MS),
    );
  }

  /** Roll back an optimistic overlay (the action failed or was declined). */
  function failTransition(id: string) {
    clearTtlTimer(id);
    transitions = clearT(transitions, id);
  }

  /** The status a row should render: optimistic overlay if any, else real. */
  function displayStatusOf(p: { id: string; status: PortbayStatus }): DisplayStatus {
    return optimisticDisplay(transitions, p.id, p.status);
  }

  /** Patch a single row from a status event. Avoids re-fetching the list. */
  function applyStatusEvent(ev: ProjectStatusEvent) {
    items = items.map((p) =>
      p.id === ev.id
        ? { ...p, status: ev.status, runtime: ev.runtime ?? p.runtime }
        : p,
    );

    // Reconcile any optimistic overlay against the real event: a resolving
    // event drops it; a stale wrong-direction tick is ignored (no flicker).
    const before = transitions;
    transitions = foldEvent(transitions, ev.id, ev.status);
    if (transitions !== before) clearTtlTimer(ev.id);

    // Track inline errors from the status event.
    if (ev.lastError) {
      lastErrors = { ...lastErrors, [ev.id]: syntheticEnvelope(ev.lastError) };
    } else if (ev.status === "running") {
      // Auto-clear when the project recovers.
      if (ev.id in lastErrors) {
        const { [ev.id]: _, ...rest } = lastErrors;
        lastErrors = rest;
      }
    }
  }

  /** Store a command-level error envelope for a project (e.g. start failed). */
  function setError(id: string, error: CommandError) {
    lastErrors = { ...lastErrors, [id]: error };
  }

  /** Dismiss / clear the inline error for a project. */
  function clearError(id: string) {
    const { [id]: _, ...rest } = lastErrors;
    lastErrors = rest;
  }

  async function start() {
    if (!browser) return;
    await refresh();
    if (unlisten) return;
    unlisten = await listen<ProjectStatusEvent>(STATUS_CHANNEL, (e) => {
      applyStatusEvent(e.payload);
    });
  }

  function stop() {
    if (unlisten) {
      unlisten();
      unlisten = null;
    }
    for (const t of ttlTimers.values()) clearTimeout(t);
    ttlTimers.clear();
    transitions = {};
  }

  function select(id: string | null) {
    selectedId = id;
  }

  function selectRelative(delta: 1 | -1) {
    if (items.length === 0) return;
    const idx = items.findIndex((p) => p.id === selectedId);
    const next =
      idx === -1
        ? 0
        : Math.max(0, Math.min(items.length - 1, idx + delta));
    selectedId = items[next].id;
  }

  return {
    get value() {
      return items;
    },
    get loading() {
      return loading;
    },
    get selectedId() {
      return selectedId;
    },
    get lastErrors() {
      return lastErrors;
    },
    refresh,
    start,
    stop,
    select,
    selectRelative,
    setError,
    clearError,
    beginTransition,
    failTransition,
    displayStatusOf,
  };
}

export const projects = createProjectsStore();
