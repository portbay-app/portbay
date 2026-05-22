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
 */
export function statusDotClass(status: PortbayStatus): string {
  switch (status) {
    case "stopped":
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

/** All variants in canonical order. Useful for demo pages and tests. */
export const ALL_STATUSES: PortbayStatus[] = [
  "stopped",
  "starting",
  "running",
  "unhealthy",
  "crashed",
  "port_conflict",
];
