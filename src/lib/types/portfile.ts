/**
 * `.portbay.json` schema as transmitted across the IPC boundary.
 * Mirrors `portbay_lib::portfile::schema::PortbayFile`.
 */

import type { ProjectType } from "$lib/types/projects";

export interface PortbayFile {
  version: number;
  name: string;
  type: ProjectType;
  hostname: string;
  port?: number | null;
  phpVersion?: string | null;
  https: boolean;
  autoStart: boolean;
  startCommand?: string | null;
  documentRoot?: string | null;
  envTemplate?: Record<string, string>;
  secrets?: string[];
  postInstall?: string[];
  tags?: string[];
}

/** Returned by `import_portfile_preview`. */
export interface PortfilePreview {
  file: PortbayFile;
  projectPath: string;
  requiredSecrets: string[];
  idCollision: boolean;
}
