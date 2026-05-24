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

/** Package manager / task runner used to scope a single-app run in a monorepo. */
export type WorkspaceTool = "pnpm" | "npm" | "yarn" | "turbo";

/**
 * Set on a project that runs ONE app of a monorepo via a workspace filter.
 * The project's `path` is the monorepo root. Mirrors `registry::types::Workspace`.
 */
export interface Workspace {
  /** Package name — the workspace filter token (e.g. `@bookslash/web`). */
  package: string;
  /** App directory relative to the monorepo root (e.g. `apps/web`). */
  relDir: string;
  tool: WorkspaceTool;
}

/** Result of `detect_workspace_apps` — `null` when the folder isn't a monorepo. */
export interface WorkspaceScan {
  tool: WorkspaceTool;
  apps: WorkspaceApp[];
}

/** One runnable monorepo app, pre-filled with standalone-project defaults. */
export interface WorkspaceApp {
  package: string;
  relDir: string;
  /** Absolute path to the app's directory (root + relDir). */
  path: string;
  kind: ProjectType;
  suggestedId: string;
  suggestedName: string;
  suggestedHostname: string;
  suggestedPort: number;
  suggestedStartCommand?: string;
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
  workspace?: Workspace;
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
