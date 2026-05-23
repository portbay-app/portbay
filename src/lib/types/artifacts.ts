/**
 * Wire shape for `commands::artifacts`. Field names follow the Rust
 * `#[serde(rename_all = "camelCase")]` convention.
 */

/** One scanned build-output directory in a project. */
export interface ArtifactDir {
  /** Project-relative key (e.g. ".next", "public/build") — passed to clean. */
  rel: string;
  label: string;
  /** Absolute path, for display. */
  path: string;
  sizeBytes: number;
  fileCount: number;
  /** Newest file mtime as Unix seconds, or null for an empty dir. */
  lastModified: number | null;
}
