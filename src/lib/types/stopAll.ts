/**
 * Wire shape of `commands::dto::StopAllReport` from the Rust side.
 * Mirrors the snake_case-renamed-to-camelCase fields per the global
 * `#[serde(rename_all = "camelCase")]` convention.
 */

export interface StopAllResultEntry {
  id: string;
  ok: boolean;
  error?: string;
}

export interface StopAllReport {
  stopped: number;
  failed: number;
  results: StopAllResultEntry[];
}
