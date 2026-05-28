/**
 * Crash-report and telemetry shapes — the TypeScript mirror of
 * `src-tauri/src/telemetry.rs`. Shared by the Settings "Crash reporting"
 * panel and the proactive crash card so the two never drift.
 */

export type CrashKind = "rust_panic" | "js_error" | "js_unhandled_rejection";

/** Lightweight row returned by `list_crash_reports`. */
export interface CrashReportSummary {
  id: string;
  kind: CrashKind;
  message: string;
  createdAt: number;
}

/** Full report returned by `read_crash_report` (adds the scrubbed backtrace). */
export interface CrashReport {
  id: string;
  kind: CrashKind;
  message: string;
  backtrace: string | null;
  os: string;
  arch: string;
  appVersion: string;
  createdAt: number;
}

/** Snapshot returned by `telemetry_settings`. */
export interface TelemetrySettings {
  enabled: boolean;
  crashReportCount: number;
  endpointConfigured: boolean;
}
