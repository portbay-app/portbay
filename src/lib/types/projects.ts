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
import type { CommandError } from "./error";

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

export type SandboxNetworkPolicy =
  | "loopback_only"
  | "outbound"
  | "full"
  | "blocked";

export interface SandboxConfig {
  enabled: boolean;
  network: SandboxNetworkPolicy;
  ephemeral: boolean;
}

/** How a domain's hostname is published to the local resolver. */
export type ResolverMode = "auto" | "hosts" | "dnsmasq";

/**
 * Per-project domain / routing settings edited on the Domains page. Mirrors
 * the Rust `registry::DomainConfig`. `null` on a project means every setting
 * takes its default (PortBay's behaviour before these knobs existed).
 */
export interface DomainConfig {
  /** Free-text note. No runtime effect. */
  notes?: string | null;
  /** URL path prefix stripped before proxying upstream. Empty / `/` = root. */
  pathPrefix?: string | null;
  resolverMode: ResolverMode;
  /** PortBay issues/renews this hostname's cert. Defaults true. */
  autoManageCert: boolean;
  /** Also route + certify `*.hostname`. */
  includeWildcardSubdomains: boolean;
  /** Only publish the Caddy route while the project's process is running. */
  exposeWhenRunning: boolean;
}

/** Defaults that match the Rust side — used when a project has no `domain`. */
export const defaultDomainConfig = (): DomainConfig => ({
  notes: null,
  pathPrefix: null,
  resolverMode: "auto",
  autoManageCert: true,
  includeWildcardSubdomains: false,
  exposeWhenRunning: false,
});

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
  sandboxed: boolean;
  sandbox?: SandboxConfig | null;
  domain?: DomainConfig | null;
  status: PortbayStatus;
  runtime?: RuntimeInfo;
  /** A reason this project's selected web server can't serve (e.g. nginx/apache
      not installed), or undefined when fine. Derived state from the backend,
      recomputed each list fetch. Rendered as an inline warning under the row. */
  webServerWarning?: string;
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

/**
 * Turn the backend's `webServerWarning` (one what-sentence + one why-sentence)
 * into an inline warning envelope, or null when there's nothing to surface.
 * Shared by ProjectRow (list) and ProjectCard (grid) so both views render the
 * same advisory: a PHP project whose nginx/apache binary is missing falls back
 * to PortBay's placeholder, and this explains why + how to fix it. Clears itself
 * on the next list fetch once the binary is installed or the project switches
 * to Caddy.
 */
export function webServerWarningEnvelope(
  msg: string | undefined | null,
): CommandError | null {
  if (!msg) return null;
  const split = msg.indexOf(". ");
  const what = split > 0 ? msg.slice(0, split + 1) : msg;
  const why =
    split > 0
      ? msg.slice(split + 2)
      : "Switch to Caddy, or install the web server.";
  return {
    code: "WEB_SERVER_MISSING",
    whatHappened: what,
    whyItMatters: why,
    whoCausedIt: "user",
    actions: [],
    severity: "warning",
  };
}
