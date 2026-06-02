/**
 * Shared CodeMirror 6 setup for the host workspace editor. Kept tiny and
 * dependency-light: only the packs we actually ship are imported
 * (`@codemirror/{state,view,commands,autocomplete,lang-json,lang-sql,theme-one-dark}`).
 * `oneDark` already bundles its syntax-highlighting extension, so we don't pull
 * in `@codemirror/language` directly (it isn't a top-level dependency).
 */
import { json } from "@codemirror/lang-json";
import { sql } from "@codemirror/lang-sql";
import type { Extension } from "@codemirror/state";

/** Pick a language extension from a file name, or `[]` for plaintext. */
export function languageFor(name: string): Extension[] {
  const ext = name.toLowerCase().split(".").pop() ?? "";
  switch (ext) {
    case "json":
    case "jsonc":
    case "webmanifest":
      return [json()];
    case "sql":
      return [sql()];
    default:
      return [];
  }
}

/** A short human label for the status bar's language indicator. */
export function languageLabel(name: string): string {
  const ext = name.toLowerCase().split(".").pop() ?? "";
  const MAP: Record<string, string> = {
    json: "JSON",
    jsonc: "JSON",
    webmanifest: "JSON",
    sql: "SQL",
    js: "JavaScript",
    mjs: "JavaScript",
    cjs: "JavaScript",
    ts: "TypeScript",
    tsx: "TypeScript",
    jsx: "JavaScript",
    md: "Markdown",
    yaml: "YAML",
    yml: "YAML",
    toml: "TOML",
    sh: "Shell",
    bash: "Shell",
    zsh: "Shell",
    rs: "Rust",
    py: "Python",
    rb: "Ruby",
    php: "PHP",
    go: "Go",
    html: "HTML",
    css: "CSS",
    svelte: "Svelte",
    vue: "Vue",
    conf: "Config",
    env: "Dotenv",
  };
  if (!name.includes(".")) return "Plain Text";
  return MAP[ext] ?? "Plain Text";
}
