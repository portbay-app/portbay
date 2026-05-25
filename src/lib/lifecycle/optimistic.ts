/**
 * Optimistic lifecycle core (card: P3 — Speed as a feature).
 *
 * Pure, dependency-free state machine for the optimistic Play/Stop overlay.
 * The projects store wraps this in `$state`; keeping the logic here (no Svelte
 * runes, no SvelteKit/Tauri imports) makes the subtle stale-event resolution
 * rules unit-testable in CI without mocking the framework.
 *
 * The model is an *overlay*, not a mutation: the canonical `status` on a
 * project row always reflects the last real `portbay://status` event. A
 * transition only changes what the row *displays* until the backend catches
 * up — so rollback is just dropping the overlay, never restoring a snapshot.
 */
import type { PortbayStatus } from "$lib/types/status";

export type TransitionKind = "start" | "stop";

/** The display state a freshly-issued action shows immediately on click. */
export type OptimisticDisplay = "starting" | "stopping";

export interface Transition {
  kind: TransitionKind;
  display: OptimisticDisplay;
  /** Wall-clock ms the transition began — used only by the store's TTL net. */
  startedAt: number;
}

/** Per-project optimistic overlays, keyed by project id. Immutable. */
export type TransitionMap = Readonly<Record<string, Transition>>;

/** The optimistic display an action shows the instant it's clicked. */
export function displayFor(kind: TransitionKind): OptimisticDisplay {
  return kind === "start" ? "starting" : "stopping";
}

/**
 * Whether a real status event RESOLVES an in-flight optimistic transition —
 * i.e. the backend has moved far enough in the intended direction that we drop
 * the overlay and show the canonical status again.
 *
 * Events in the *wrong* direction are treated as stale poll readings and
 * ignored (returns `false`), so the row doesn't flicker back during the ~1s
 * the backend takes to catch up. The status poller ticks every 750 ms, so a
 * "stopped" reading can easily arrive *after* a Play click but *before* the
 * process actually boots; suppressing it is the whole point.
 */
export function resolvesTransition(
  kind: TransitionKind,
  status: PortbayStatus,
): boolean {
  if (kind === "start") {
    // A start is satisfied once the project is anything but fully stopped
    // (starting / running / unhealthy / crashed / port_conflict all count —
    // the backend has acted on the intent).
    return status !== "stopped";
  }
  // A stop is satisfied once the project has come to rest — i.e. it's no
  // longer up or booting. A lingering "running"/"starting" tick is stale.
  return status !== "running" && status !== "starting";
}

/** Record a new optimistic transition for a project (overwrites any prior). */
export function beginTransition(
  map: TransitionMap,
  id: string,
  kind: TransitionKind,
  now: number = Date.now(),
): TransitionMap {
  return { ...map, [id]: { kind, display: displayFor(kind), startedAt: now } };
}

/** Drop the overlay for a project (rollback on failure, or after resolution). */
export function clearTransition(map: TransitionMap, id: string): TransitionMap {
  if (!(id in map)) return map;
  const { [id]: _dropped, ...rest } = map;
  return rest;
}

/**
 * Fold a real status event into the overlay map: clears the overlay iff the
 * event resolves the transition, otherwise leaves it untouched (stale event).
 */
export function onStatusEvent(
  map: TransitionMap,
  id: string,
  status: PortbayStatus,
): TransitionMap {
  const t = map[id];
  if (!t) return map;
  return resolvesTransition(t.kind, status) ? clearTransition(map, id) : map;
}

/** The status to render: the optimistic overlay if present, else canonical. */
export function optimisticDisplay(
  map: TransitionMap,
  id: string,
  canonical: PortbayStatus,
): PortbayStatus | OptimisticDisplay {
  return map[id]?.display ?? canonical;
}
