/**
 * Wire shapes for `commands::databases`. Field names follow the Rust
 * `#[serde(rename_all = "camelCase")]` convention.
 */

export type DatabaseEngineId =
  | "mysql"
  | "mariadb"
  | "postgres"
  | "redis"
  | "mongo"
  | "memcached";

/** An engine the user can create instances from (Add Database picker). */
export interface DatabaseEngineView {
  id: DatabaseEngineId;
  label: string;
  installed: boolean;
  version: string;
  defaultPort: number;
  clientAvailable: boolean;
  installHint: string;
  /** True when a PortBay-managed build of this engine is installed. */
  managed: boolean;
  /** Version of the managed build, when `managed` is true. */
  managedVersion: string;
}

/** Progress events streamed from `install_database_engine` (mirrors runtimes). */
export type EngineInstallEvent =
  | { kind: "log"; line: string }
  | { kind: "progress"; downloaded: number; total: number | null }
  | { kind: "done"; success: boolean };

export type InstanceStatus = "running" | "stopped" | "starting" | "errored";

/** A provisioned, PortBay-supervised database instance. */
export interface DatabaseInstanceView {
  id: string;
  name: string;
  engine: DatabaseEngineId;
  engineLabel: string;
  version: string;
  port: number;
  status: InstanceStatus;
  autoStart: boolean;
  dataDir: string;
  configPath?: string | null;
  socketPath?: string | null;
  connectionUrl: string;
  account: string;
  linkedProjects: string[];
  binaryAvailable: boolean;
  provisioned: boolean;
}

export interface CreateDatabaseInput {
  engine: DatabaseEngineId;
  name: string;
  port?: number | null;
  autoStart?: boolean;
}

export const statusLabel: Record<InstanceStatus, string> = {
  running: "Running",
  stopped: "Stopped",
  starting: "Starting",
  errored: "Error",
};

/**
 * A database connection parsed from a project's on-disk `.env` (returned by
 * `project_db_connections`). Distinct from {@link DatabaseInstanceView},
 * which is a PortBay-provisioned server — this is just the connection a
 * project's own config points at.
 */
export interface ProjectDbConnection {
  /** "Default" for the primary `DB_*` set, else the prefix (e.g. "READ"). */
  name: string;
  /** Driver from `DB_CONNECTION` (e.g. "mysql", "pgsql"); empty if unset. */
  driver: string;
  host: string;
  port: string;
  database: string;
  username: string;
  password: string;
  /** Scheme URL a DB client can open, or null when hostless (e.g. sqlite). */
  url: string | null;
}
