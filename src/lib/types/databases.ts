/**
 * Wire shapes for `commands::databases`. Field names follow the Rust
 * `#[serde(rename_all = "camelCase")]` convention.
 */

export type DatabaseEngineId =
  | "mysql"
  | "mariadb"
  | "postgres"
  | "redis"
  | "mongo";

/** An engine the user can create instances from (Add Database picker). */
export interface DatabaseEngineView {
  id: DatabaseEngineId;
  label: string;
  installed: boolean;
  version: string;
  defaultPort: number;
  clientAvailable: boolean;
  installHint: string;
}

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
