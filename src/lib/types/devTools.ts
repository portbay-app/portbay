/**
 * Wire shape of `commands::integrations::DevToolInfo`.
 */
export type DevToolKind = "editor" | "agent" | "terminal" | "file-manager";

export interface DevToolInfo {
  id: string;
  label: string;
  kind: DevToolKind;
}
