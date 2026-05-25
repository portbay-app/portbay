/**
 * TypeScript shapes mirroring `runtimes::*` on the Rust side. Field
 * names follow the `#[serde(rename_all = "camelCase")]` convention.
 *
 * Source of truth: src-tauri/src/runtimes/mod.rs.
 */

export type InstallSource =
  | "homebrew"
  | "serv_bay"
  | "fly_env"
  | "asdf"
  | "mise"
  | "nvm"
  | "pyenv"
  | "system"
  | "manual";

export interface RuntimeInstall {
  version: string;
  binary: string;
  source: InstallSource;
  configDir?: string | null;
}

/**
 * How a row renders + whether it's editable. Mirrors `runtimes::FieldKind`
 * (internally tagged on `kind`). `readonly` rows show value + copy/reveal;
 * the rest render the matching input and are posted back on save.
 */
export type FieldKind =
  | { kind: "readonly" }
  | { kind: "text" }
  | { kind: "number"; min?: number; max?: number }
  | { kind: "select"; options: string[] }
  | { kind: "bool" };

export interface KvRow {
  /** Stable key edits are posted under (ignored for readonly rows). */
  key: string;
  label: string;
  value: string;
  hint?: string;
  isPath?: boolean;
  field: FieldKind;
}

export interface ConfigTab {
  id: string;
  label: string;
  rows: KvRow[];
  /** When true, the tab has editable rows and shows a Save button. */
  editable?: boolean;
}

export interface VersionView {
  install: RuntimeInstall;
  tabs: ConfigTab[];
}

export interface LanguageView {
  id: string;
  displayName: string;
  versions: VersionView[];
  installHint: string;
  /** Version marked as this language's default, or null if none set. */
  defaultVersion?: string | null;
}

/** Human label for the install-source pill. */
export const sourceLabel: Record<InstallSource, string> = {
  homebrew: "Homebrew",
  serv_bay: "ServBay",
  fly_env: "FlyEnv",
  asdf: "asdf",
  mise: "mise",
  nvm: "nvm",
  pyenv: "pyenv",
  system: "System",
  manual: "Manual",
};
