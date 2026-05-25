/**
 * HTTP request inspector types — mirror the Rust `RequestEntry` DTO
 * (`commands/http_inspector.rs`, `#[serde(rename_all = "camelCase")]`).
 */
export interface RequestEntry {
  /** Unix milliseconds when Caddy handled the request. */
  ts: number;
  method: string;
  host: string;
  uri: string;
  status: number;
  durationMs: number;
  /** Response size in bytes. */
  size: number;
  /** The PortBay project this host maps to, when known. */
  projectId?: string;
  /** Request headers Caddy logged, for the row-detail view. */
  reqHeaders?: Record<string, string[]>;
}
