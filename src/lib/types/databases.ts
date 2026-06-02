/**
 * Wire shapes for `commands::databases`. Field names follow the Rust
 * `#[serde(rename_all = "camelCase")]` convention.
 */

export type DatabaseEngineId =
  | "mysql"
  | "mariadb"
  | "postgres"
  | "sqlite"
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

/** A backup snapshot on disk (from `list_database_backups`). */
export interface BackupSnapshot {
  /** Snapshot id — the unix-millis timestamp it was taken at. */
  id: string;
  createdAt: number;
  sizeBytes: number;
}

/** Result of `provision_project_database`. */
export interface ProjectDbProvision {
  database: string;
  username: string;
  connectionUrl: string;
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
  /** Absolute path to the database file, for file-based engines (SQLite). */
  filePath?: string | null;
  /** True for file-based engines (SQLite) — no daemon, port, or lifecycle. */
  fileBased: boolean;
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
  /** For file-based engines (SQLite): adopt an existing file at this path. */
  filePath?: string | null;
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

export interface DbClientColumn {
  name: string;
  dataType: string;
  nullable: boolean;
  primaryKey: boolean;
}

export interface DbClientForeignKey {
  table: string;
  column: string;
  refTable: string;
  refColumn: string;
}

export interface DbClientTable {
  schema?: string | null;
  name: string;
  columns: DbClientColumn[];
  foreignKeys: DbClientForeignKey[];
}

export interface DbClientSchema {
  engine: DatabaseEngineId;
  schemas: string[];
  tables: DbClientTable[];
}

export interface DbClientRows {
  columns: DbClientColumn[];
  rows: unknown[][];
  affectedRows: number;
  truncated: boolean;
}

/**
 * A node in a parsed query-execution plan. Mirrors the Rust `DbExplainNode`
 * (`#[serde(rename_all = "camelCase")]`). This is a recursive tree — `children`
 * holds the node's inputs — not a flat list. Numeric fields are `null` when the
 * engine/plan does not provide them (e.g. cost is absent for SQLite, and the
 * `actual*` fields are only populated when the plan was run with ANALYZE).
 */
export interface DbExplainNode {
  id: string;
  nodeType: string;
  relation: string | null;
  startupCost: number | null;
  totalCost: number | null;
  planRows: number | null;
  actualRows: number | null;
  actualTimeMs: number | null;
  actualLoops: number | null;
  buffersHit: number | null;
  buffersRead: number | null;
  filter: string | null;
  indexCondition: string | null;
  joinType: string | null;
  hashCondition: string | null;
  /** Engine-specific fields that don't map to a typed slot above. */
  extra: Record<string, unknown>;
  children: DbExplainNode[];
}

/** A parsed query plan returned by `database_client_explain`. */
export interface DbExplainPlan {
  root: DbExplainNode;
  planningTimeMs: number | null;
  executionTimeMs: number | null;
  originalQuery: string;
  /** Engine id the plan came from ("sqlite" | "mysql" | "postgres"). */
  driver: string;
  /** True when the plan carries real ANALYZE measurements (actual rows/time). */
  hasAnalyzeData: boolean;
  /** Raw EXPLAIN text/JSON, for the "raw" view. */
  rawOutput: string | null;
}

/**
 * A database write proposed by an AI agent (via MCP) that is blocked until
 * a human approves or denies it. Mirrors the Rust `PendingWrite` serde shape
 * (`#[serde(rename_all = "camelCase")]`).
 */
export interface PendingDbWrite {
  id: string;
  instanceId: string;
  engine: string;
  schema: string | null;
  sql: string;
  origin: string;
  createdAtMs: number;
}
