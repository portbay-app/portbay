/**
 * TypeScript shapes mirroring `runtimes::*` on the Rust side. Field
 * names follow the `#[serde(rename_all = "camelCase")]` convention.
 *
 * Source of truth: src-tauri/src/runtimes/mod.rs.
 */

export type InstallSource =
  | "homebrew"
  | "asdf"
  | "mise"
  | "nvm"
  | "pyenv"
  | "system";

export interface RuntimeInstall {
  version: string;
  binary: string;
  source: InstallSource;
  configDir?: string | null;
}

export interface KvRow {
  label: string;
  value: string;
  hint?: string;
  isPath?: boolean;
}

export interface ConfigTab {
  id: string;
  label: string;
  rows: KvRow[];
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
}

/** Human label for the install-source pill. */
export const sourceLabel: Record<InstallSource, string> = {
  homebrew: "Homebrew",
  asdf: "asdf",
  mise: "mise",
  nvm: "nvm",
  pyenv: "pyenv",
  system: "System",
};
