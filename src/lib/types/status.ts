/**
 * PortBay status taxonomy — exactly mirrors the Rust enum
 * `process_compose::ProjectStatus` in `src-tauri/src/process_compose/types.rs`.
 * Serde renames each variant to snake_case across the IPC boundary.
 *
 * Source of truth: docs/UX_DESIGN.md §5.3.
 *
 * Keep these in sync with the Rust side — if a new state is added there,
 * add it here and update every consumer.
 */
export type PortbayStatus =
  | "stopped"
  | "starting"
  | "running"
  | "unhealthy"
  | "crashed"
  | "port_conflict";

/**
 * Display-only superset of {@link PortbayStatus}. `stopping` has no backend
 * counterpart — it's the optimistic state a row shows the instant Stop is
 * clicked, before the real `stopped` event lands (card: P3 — Speed as a
 * feature). Only the presentation layer (StatusDot / StatusPill / rows) sees
 * it; the canonical `status` on a project is always a real `PortbayStatus`.
 */
export type DisplayStatus = PortbayStatus | "stopping";

/** Human-readable label for each status (the word users see). */
export const statusLabel: Record<PortbayStatus, string> = {
  stopped: "Stopped",
  starting: "Starting…",
  running: "Running",
  unhealthy: "Needs attention",
  crashed: "Crashed",
  port_conflict: "Port in use",
};

/**
 * Tailwind utility for the dot color. Maps to the `--color-status-*` tokens
 * declared in `src/app.css`. Using a function (not a const map) so the
 * Tailwind JIT picks up the class strings statically.
 *
 * Accepts {@link DisplayStatus}: the optimistic `stopping` reuses the neutral
 * stopped colour (it pulses, via {@link isTransitional}, so it reads as
 * "settling down" rather than at rest).
 */
export function statusDotClass(status: DisplayStatus): string {
  switch (status) {
    case "stopped":
      return "bg-status-stopped";
    case "stopping":
      return "bg-status-stopped";
    case "starting":
      return "bg-status-starting";
    case "running":
      return "bg-status-running";
    case "unhealthy":
      return "bg-status-unhealthy";
    case "crashed":
      return "bg-status-crashed";
    case "port_conflict":
      return "bg-status-port-conflict";
  }
}

/** Human-readable label for a {@link DisplayStatus} (incl. optimistic states). */
export function displayStatusLabel(status: DisplayStatus): string {
  return status === "stopping" ? "Stopping…" : statusLabel[status];
}

/** Transitional states pulse their dot to signal in-flight work. */
export function isTransitional(status: DisplayStatus): boolean {
  return status === "starting" || status === "stopping";
}

/** All variants in canonical order. Useful for demo pages and tests. */
export const ALL_STATUSES: PortbayStatus[] = [
  "stopped",
  "starting",
  "running",
  "unhealthy",
  "crashed",
  "port_conflict",
];
