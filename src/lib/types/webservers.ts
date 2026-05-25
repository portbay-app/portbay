/**
 * TypeScript shape of `commands::webservers::{WebServerInfo,
 * WebServerProjectRef}`. Backs the `/web-servers` page. Keep field names in
 * sync with the Rust `#[serde(rename_all = "camelCase")]` structs.
 */
import type { WebServer } from "./projects";

/** A project that currently resolves to a given web server. */
export interface WebServerProjectRef {
  id: string;
  name: string;
}

/** One server's status as shown on the Web Server page. */
export interface WebServerInfo {
  /** Stable id — matches the `WebServer` union. */
  id: WebServer;
  /** Display name ("Caddy" | "Nginx" | "Apache"). */
  name: string;
  /** One-line description of the role this server plays in PortBay. */
  role: string;
  /** True for Caddy — PortBay's public edge. The others are loopback-only. */
  edge: boolean;
  /** True when PortBay ships the binary (Caddy); others are detected. */
  bundled: boolean;
  /** Whether the binary is available (bundled, or detected on disk). */
  installed: boolean;
  /** Resolved binary path, when found. null for the bundled Caddy sidecar. */
  binaryPath: string | null;
  /** Best-effort version string parsed from the binary. null when unknown. */
  version: string | null;
  /** PHP projects that currently resolve to this server. */
  projects: WebServerProjectRef[];
  /** True when this server is the default for newly-added PHP projects. */
  isDefault: boolean;
}
