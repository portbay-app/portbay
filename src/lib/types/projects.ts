/**
 * TypeScript shape of `commands::dto::ProjectView` and friends from the
 * Rust side. Field names follow the `#[serde(rename_all = "camelCase")]`
 * convention everywhere except `kind`, which the Rust side explicitly
 * renames to `type` for backward compatibility with the documented
 * registry JSON shape.
 *
 * Source of truth: `src-tauri/src/commands/dto.rs`.
 */
import type { PortbayStatus } from "./status";

export type ProjectType =
  | "next"
  | "vite"
  | "php"
  | "static"
  | "node"
  | "custom";

export interface Readiness {
  type: "http" | "tcp" | "process";
  path?: string;
  timeout_seconds?: number;
}

export interface RuntimeInfo {
  pid: number;
  restarts: number;
  isReady: string;
  hasReadyProbe: boolean;
  exitCode: number;
  /** Process age in nanoseconds (PC's native unit). */
  age: number;
  memBytes: number;
  cpuPercent: number;
}

export interface ProjectView {
  id: string;
  name: string;
  path: string;
  type: ProjectType;
  startCommand?: string;
  port?: number;
  extraPorts: number[];
  hostname: string;
  url: string;
  https: boolean;
  services: string[];
  env: Record<string, string>;
  readiness?: Readiness;
  autoStart: boolean;
  tags: string[];
  documentRoot?: string;
  phpVersion?: string;
  status: PortbayStatus;
  runtime?: RuntimeInfo;
}

/** Emitted on `portbay://status` for every project that transitions. */
export interface ProjectStatusEvent {
  id: string;
  status: PortbayStatus;
  runtime?: RuntimeInfo;
  lastError?: string;
  ts: number;
}

/** Short labels for each `ProjectType` — used in the table's type column. */
export const typeLabel: Record<ProjectType, string> = {
  next: "Next",
  vite: "Vite",
  php: "PHP",
  static: "Static",
  node: "Node",
  custom: "Custom",
};
