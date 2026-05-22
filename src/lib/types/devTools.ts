/**
 * Wire shape of `commands::integrations::DevToolInfo`.
 */
export type DevToolKind = "editor" | "agent" | "terminal";

export interface DevToolInfo {
  id: string;
  label: string;
  kind: DevToolKind;
}
