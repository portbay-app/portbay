/**
 * Migration-import wire types. Mirror
 * `src-tauri/src/import/mod.rs` + `commands/import.rs`.
 */

import type { ProjectType } from "$lib/types/projects";

export type ImportSource = "herd" | "servBay" | "mamp";

export interface DetectedSource {
  source: ImportSource;
  label: string;
  present: boolean;
  siteCount: number;
  note?: string | null;
}

export interface ImportedSite {
  source: ImportSource;
  path: string;
  hostname: string;
  phpVersion?: string | null;
  https: boolean;
  suggestedId: string;
  suggestedName: string;
}

export interface ImportPreviewRow {
  site: ImportedSite;
  idCollision: boolean;
  pathCollision: boolean;
}

export interface SkippedRow {
  site: ImportedSite;
  reason: string;
}

export interface ImportResult {
  imported: string[];
  skipped: SkippedRow[];
}
