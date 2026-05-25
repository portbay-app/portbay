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
  | "flutter"
  | "xcode"
  | "android"
  | "custom";

export type WebServer = "caddy" | "nginx" | "apache";

export interface MobileRunConfig {
  /** Flutter flavor or Android variant, e.g. staging/debug. */
  flavor?: string | null;
  /** Xcode scheme or Android module, e.g. App/app. */
  target?: string | null;
  /** Flutter device id, Android serial, or xcodebuild destination. */
  device?: string | null;
}

export interface Readiness {
  type: "http" | "tcp" | "process";
  path?: string;
  timeout_seconds?: number;
}

/** Package manager / task runner used to scope a single-app run in a monorepo. */
export type WorkspaceTool = "pnpm" | "npm" | "yarn" | "bun" | "turbo";

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
  suggestedPort: number | null;
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

/**
 * Per-project CORS policy (Pro). Empty `allowedOrigins` = no policy (the free
 * default — PortBay adds no CORS headers). Editing this is gated on the
 * `custom_port_cors` entitlement; the basic listen port is never gated.
 */
export interface CorsConfig {
  allowedOrigins: string[];
  allowCredentials: boolean;
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
  webServer?: WebServer;
  mobileRun?: MobileRunConfig | null;
  workspace?: Workspace;
  cors?: CorsConfig | null;
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
  flutter: "Flutter",
  xcode: "Xcode",
  android: "Android",
  custom: "Custom",
};

/** Display labels for each web server option. */
export const webServerLabel: Record<WebServer, string> = {
  caddy: "Caddy",
  nginx: "Nginx",
  apache: "Apache",
};

/**
 * The web server a project actually runs behind, or `null` when the choice
 * doesn't apply. Mirrors the Rust rule (`registry::types` + `caddy::config`):
 * the per-project web server is only honored for PHP document-root projects
 * (no custom start command). Everything else — JS/Node dev servers, or PHP
 * projects with their own start command — is reverse-proxied through Caddy and
 * has no user-selectable server, so we don't label one.
 */
export function effectiveWebServer(p: ProjectView): WebServer | null {
  if (p.type !== "php") return null;
  if (p.startCommand && p.startCommand.trim() !== "") return null;
  return p.webServer ?? "caddy";
}
