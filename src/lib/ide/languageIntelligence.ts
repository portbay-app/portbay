/**
 * Curated CodeMirror intelligence for the file types PortBay edits directly.
 *
 * This is intentionally not a public extension marketplace. PortBay owns the
 * language profiles, keeps the feature set predictable, and can later swap a
 * profile's diagnostics/completions to a real LSP bridge without changing the
 * editor component contract.
 */
import {
  type Completion,
  type CompletionContext,
  type CompletionResult,
  type CompletionSource,
} from "@codemirror/autocomplete";
import type { Extension, Text } from "@codemirror/state";
import { linter, lintGutter, type Diagnostic } from "@codemirror/lint";
import { EditorView, hoverTooltip, type Tooltip } from "@codemirror/view";

type SmartLanguageId =
  | "javascript"
  | "typescript"
  | "json"
  | "yaml"
  | "toml"
  | "dotenv"
  | "sql"
  | "python"
  | "php"
  | "shell"
  | "nginx"
  | "systemd"
  | "dockerfile"
  | "sshconfig"
  | "apache";

interface SmartLanguage {
  id: SmartLanguageId;
  label: string;
  features: string[];
}

const LANGUAGE_PROFILES: Record<SmartLanguageId, SmartLanguage> = {
  javascript: {
    id: "javascript",
    label: "JavaScript",
    features: ["diagnostics", "completions", "hover"],
  },
  typescript: {
    id: "typescript",
    label: "TypeScript",
    features: ["diagnostics", "completions", "hover"],
  },
  json: {
    id: "json",
    label: "JSON / JSONC",
    features: ["diagnostics", "schema keys", "hover"],
  },
  yaml: {
    id: "yaml",
    label: "YAML",
    features: ["diagnostics", "schema keys", "hover"],
  },
  toml: {
    id: "toml",
    label: "TOML",
    features: ["diagnostics", "schema keys", "hover"],
  },
  dotenv: {
    id: "dotenv",
    label: "Environment",
    features: ["diagnostics", "common variables", "hover"],
  },
  sql: {
    id: "sql",
    label: "SQL",
    features: ["diagnostics", "completions", "hover"],
  },
  python: {
    id: "python",
    label: "Python",
    features: ["diagnostics", "completions", "hover"],
  },
  php: {
    id: "php",
    label: "PHP",
    features: ["diagnostics", "completions", "hover"],
  },
  shell: {
    id: "shell",
    label: "Shell",
    features: ["diagnostics", "completions", "hover"],
  },
  nginx: {
    id: "nginx",
    label: "Nginx",
    features: ["brace check", "directives", "hover"],
  },
  systemd: {
    id: "systemd",
    label: "systemd unit",
    features: ["diagnostics", "unit keys", "hover"],
  },
  dockerfile: {
    id: "dockerfile",
    label: "Dockerfile",
    features: ["diagnostics", "instructions", "hover"],
  },
  sshconfig: {
    id: "sshconfig",
    label: "SSH config",
    features: ["diagnostics", "keywords", "hover"],
  },
  apache: {
    id: "apache",
    label: "Apache",
    features: ["directives", "hover"],
  },
};

const WORD = /[A-Za-z_$][\w$-]*/;

export function smartLanguageForFile(name: string): SmartLanguage | null {
  // `name` may be a bare filename or a full remote path — path-aware checks
  // (`.ssh/config`, `/etc/nginx/sites-*`) only fire when the caller passes one.
  const lowerPath = name.toLowerCase();
  const lower = basename(name).toLowerCase();
  const ext = lower.split(".").pop() ?? "";

  if (lower === ".env" || lower.startsWith(".env.") || ext === "env") {
    return LANGUAGE_PROFILES.dotenv;
  }
  if (lower === "dockerfile" || lower.endsWith(".dockerfile")) {
    return LANGUAGE_PROFILES.dockerfile;
  }
  if (lower === "sshd_config" || lower === "ssh_config" || lowerPath.endsWith(".ssh/config")) {
    return LANGUAGE_PROFILES.sshconfig;
  }
  if (
    lower === "nginx.conf" ||
    (lower.includes("nginx") && ext === "conf") ||
    lowerPath.includes("/nginx/") // sites-available/* files have no extension
  ) {
    return LANGUAGE_PROFILES.nginx;
  }
  if (lower === ".htaccess" || lower === "httpd.conf" || lower === "apache2.conf") {
    return LANGUAGE_PROFILES.apache;
  }
  if (["service", "socket", "timer", "target", "mount"].includes(ext)) {
    return LANGUAGE_PROFILES.systemd;
  }
  if (["json", "jsonc", "webmanifest"].includes(ext)) return LANGUAGE_PROFILES.json;
  if (["yaml", "yml"].includes(ext)) return LANGUAGE_PROFILES.yaml;
  if (ext === "toml") return LANGUAGE_PROFILES.toml;
  if (["sql"].includes(ext)) return LANGUAGE_PROFILES.sql;
  if (["py", "pyw"].includes(ext)) return LANGUAGE_PROFILES.python;
  if (ext === "php") return LANGUAGE_PROFILES.php;
  if (["sh", "bash", "zsh", "fish", "ksh"].includes(ext)) return LANGUAGE_PROFILES.shell;
  if (["ts", "tsx", "mts", "cts"].includes(ext)) return LANGUAGE_PROFILES.typescript;
  if (["js", "jsx", "mjs", "cjs"].includes(ext)) return LANGUAGE_PROFILES.javascript;
  return null;
}

export function smartLanguageSummary(name: string): string | null {
  const lang = smartLanguageForFile(name);
  if (!lang) return null;
  return `${lang.label} smart help: ${lang.features.join(", ")}`;
}

export function smartLanguageExtensions(name: string): Extension[] {
  const lang = smartLanguageForFile(name);
  if (!lang) return [];
  return [
    lintGutter(),
    linter((view) => diagnosticsFor(lang.id, view.state.doc, name), { delay: 450 }),
    hoverTooltip((view, pos) => hoverFor(lang.id, view, pos)),
    smartTheme,
  ];
}

export function smartCompletionSourceFor(name: string): CompletionSource | null {
  const lang = smartLanguageForFile(name);
  if (!lang) return null;
  return (context) => completionsFor(lang.id, name, context);
}

function diagnosticsFor(lang: SmartLanguageId, doc: Text, name: string): Diagnostic[] {
  const text = doc.toString();
  switch (lang) {
    case "json":
      return jsonDiagnostics(doc, text, name);
    case "yaml":
      return yamlDiagnostics(doc);
    case "toml":
      return tomlDiagnostics(doc);
    case "dotenv":
      return dotenvDiagnostics(doc);
    case "python":
      return [...pythonDiagnostics(doc), ...delimiterDiagnostics(text, "python")];
    case "php":
      return [...phpDiagnostics(doc, text), ...delimiterDiagnostics(text, "php")];
    case "sql":
      return sqlDiagnostics(doc, text);
    case "shell":
      return shellDiagnostics(doc);
    case "javascript":
    case "typescript":
      return [...jsDiagnostics(doc), ...delimiterDiagnostics(text, "javascript")];
    case "nginx":
      return nginxDiagnostics(doc, text);
    case "systemd":
      return systemdDiagnostics(doc);
    case "dockerfile":
      return dockerfileDiagnostics(doc);
    case "sshconfig":
      return sshConfigDiagnostics(doc);
    case "apache":
      return []; // directive grammar is too free-form to lint shallowly
  }
}

/** Brace balance only — nginx directives (multi-line log_format, regex
    locations) are too free-form to lint line-by-line without false alarms. */
function nginxDiagnostics(doc: Text, text: string): Diagnostic[] {
  const out: Diagnostic[] = [];
  let depth = 0;
  for (let i = 0; i < text.length; i += 1) {
    const ch = text[i];
    if (ch === "#") {
      while (i < text.length && text[i] !== "\n") i += 1;
      continue;
    }
    if (ch === "{") depth += 1;
    else if (ch === "}") {
      depth -= 1;
      if (depth < 0) {
        out.push({
          from: i,
          to: i + 1,
          severity: "error",
          source: "PortBay nginx",
          message: "Unmatched '}'.",
        });
        depth = 0;
      }
    }
  }
  if (depth > 0) {
    out.push(fileDiagnostic(doc, "warning", `${depth} unclosed '{' block${depth === 1 ? "" : "s"}.`));
  }
  return out;
}

function systemdDiagnostics(doc: Text): Diagnostic[] {
  const out: Diagnostic[] = [];
  let inSection = false;
  let continued = false;
  forEachLine(doc, (line) => {
    const trimmed = line.text.trim();
    const wasContinued = continued;
    continued = /\\\s*$/.test(trimmed);
    if (wasContinued) return; // continuation of the previous value
    if (!trimmed || trimmed.startsWith("#") || trimmed.startsWith(";")) return;
    if (/^\[[^\]]+\]$/.test(trimmed)) {
      inSection = true;
      return;
    }
    if (!inSection) {
      out.push(lineDiagnostic(line, "warning", "Unit entries belong under a [Section] header."));
      return;
    }
    if (!/^[A-Za-z][A-Za-z0-9]*\s*=/.test(trimmed)) {
      out.push(lineDiagnostic(line, "warning", "Unit entries are `Key=value` pairs."));
    }
  });
  return out;
}

function dockerfileDiagnostics(doc: Text): Diagnostic[] {
  const KNOWN = new Set([
    "from", "run", "cmd", "entrypoint", "copy", "add", "workdir", "env", "arg",
    "expose", "volume", "user", "label", "healthcheck", "onbuild", "shell",
    "stopsignal", "maintainer",
  ]);
  const out: Diagnostic[] = [];
  let sawFrom = false;
  let continued = false;
  forEachLine(doc, (line) => {
    const trimmed = line.text.trim();
    const wasContinued = continued;
    continued = /\\\s*$/.test(trimmed);
    if (wasContinued) return; // continuation of a previous instruction
    if (!trimmed || trimmed.startsWith("#")) return;
    const word = trimmed.split(/\s+/)[0].toLowerCase();
    if (!KNOWN.has(word)) {
      out.push(lineDiagnostic(line, "warning", `Unknown Dockerfile instruction '${word.toUpperCase()}'.`));
      return;
    }
    if (!sawFrom && word !== "from" && word !== "arg") {
      out.push(lineDiagnostic(line, "warning", "Instructions before the first FROM are ignored (only ARG is allowed)."));
    }
    if (word === "from") sawFrom = true;
  });
  return out;
}

function sshConfigDiagnostics(doc: Text): Diagnostic[] {
  const out: Diagnostic[] = [];
  forEachLine(doc, (line) => {
    const trimmed = line.text.trim();
    if (!trimmed || trimmed.startsWith("#")) return;
    if (!/^\S+[\s=]+\S/.test(trimmed)) {
      out.push(lineDiagnostic(line, "info", "SSH config entries are `Keyword value` pairs."));
    }
  });
  return out;
}

function jsonDiagnostics(doc: Text, text: string, name: string): Diagnostic[] {
  const prepared = stripJsonc(text);
  const diagnostics: Diagnostic[] = [];
  try {
    JSON.parse(prepared.text);
  } catch (err) {
    const message = err instanceof Error ? err.message : "Invalid JSON";
    const pos = jsonErrorPosition(message, prepared.text);
    diagnostics.push({
      from: pos,
      to: Math.min(doc.length, pos + 1),
      severity: "error",
      source: "PortBay JSON",
      message,
    });
  }

  if (basename(name).toLowerCase() === "package.json") {
    const raw = text.toLowerCase();
    if (!raw.includes('"scripts"')) {
      diagnostics.push(fileDiagnostic(doc, "info", "package.json has no scripts block."));
    } else if (!raw.includes('"dev"')) {
      diagnostics.push(fileDiagnostic(doc, "info", "No dev script detected for PortBay's default Play command."));
    }
  }
  return diagnostics;
}

function yamlDiagnostics(doc: Text): Diagnostic[] {
  const out: Diagnostic[] = [];
  forEachLine(doc, (line) => {
    const raw = line.text;
    if (/^\t+/.test(raw)) {
      out.push(lineDiagnostic(line, "warning", "YAML indentation should use spaces, not tabs."));
    }
    if (/^\s*[^#\s][^:]*:[^\s"'\[{>|-]/.test(raw)) {
      out.push(lineDiagnostic(line, "info", "Add a space after ':' for portable YAML parsers."));
    }
    if (/^\s*-\S/.test(raw)) {
      out.push(lineDiagnostic(line, "info", "Add a space after '-' in YAML list items."));
    }
  });
  return out;
}

function tomlDiagnostics(doc: Text): Diagnostic[] {
  const out: Diagnostic[] = [];
  const seenTables = new Map<string, number>();
  forEachLine(doc, (line) => {
    const trimmed = stripLineComment(line.text, "#").trim();
    if (!trimmed) return;
    const table = trimmed.match(/^\[([^\]]+)\]$/);
    if (table) {
      const prev = seenTables.get(table[1]);
      if (prev != null) {
        out.push(lineDiagnostic(line, "warning", `Duplicate TOML table [${table[1]}] also appears on line ${prev}.`));
      } else {
        seenTables.set(table[1], line.number);
      }
      return;
    }
    if (!/^[A-Za-z0-9_.-]+\s*=/.test(trimmed)) {
      out.push(lineDiagnostic(line, "warning", "TOML entries should be `key = value` pairs or `[table]` headers."));
    }
  });
  return out;
}

function dotenvDiagnostics(doc: Text): Diagnostic[] {
  const out: Diagnostic[] = [];
  forEachLine(doc, (line) => {
    const trimmed = line.text.trim();
    if (!trimmed || trimmed.startsWith("#")) return;
    if (!/^(export\s+)?[A-Za-z_][A-Za-z0-9_]*\s*=/.test(trimmed)) {
      out.push(lineDiagnostic(line, "warning", "Environment entries should be `NAME=value` pairs."));
    }
    if (/^\s+/.test(line.text)) {
      out.push(lineDiagnostic(line, "info", "Leading whitespace becomes part of some dotenv parsers' input."));
    }
  });
  return out;
}

function pythonDiagnostics(doc: Text): Diagnostic[] {
  const out: Diagnostic[] = [];
  let sawTabs = false;
  let sawSpaces = false;
  forEachLine(doc, (line) => {
    const raw = line.text;
    const indent = raw.match(/^[\t ]*/)?.[0] ?? "";
    if (indent.includes("\t")) sawTabs = true;
    if (indent.includes(" ")) sawSpaces = true;
    if (/^ +\t|\t+ /.test(indent)) {
      out.push(lineDiagnostic(line, "warning", "Mixed tabs and spaces in one indentation block."));
    }
    if (/^\s*(if|elif|else|for|while|def|class|try|except|finally|with|match|case)\b.*[^:]$/.test(raw.trim())) {
      out.push(lineDiagnostic(line, "warning", "Python block statements normally end with ':'."));
    }
  });
  if (sawTabs && sawSpaces) {
    out.push(fileDiagnostic(doc, "info", "This file mixes tabs and spaces for indentation."));
  }
  return out;
}

function phpDiagnostics(doc: Text, text: string): Diagnostic[] {
  const out: Diagnostic[] = [];
  if (!text.includes("<?php") && !text.includes("<?=")) {
    out.push(fileDiagnostic(doc, "info", "PHP files usually start with `<?php` or `<?=`."));
  }
  forEachLine(doc, (line) => {
    if (/\b(var_dump|print_r)\s*\(/.test(line.text)) {
      out.push(lineDiagnostic(line, "info", "Debug output left in PHP response path."));
    }
  });
  return out;
}

function jsDiagnostics(doc: Text): Diagnostic[] {
  const out: Diagnostic[] = [];
  forEachLine(doc, (line) => {
    if (/\bconsole\.(log|debug)\s*\(/.test(line.text)) {
      out.push(lineDiagnostic(line, "info", "Console logging left in code."));
    }
  });
  return out;
}

function sqlDiagnostics(doc: Text, text: string): Diagnostic[] {
  const out: Diagnostic[] = [];
  if (hasUnclosedQuote(text, "'")) {
    out.push(fileDiagnostic(doc, "error", "Unclosed SQL string literal."));
  }
  forEachLine(doc, (line) => {
    if (/^\s*(drop|truncate|delete|update)\b/i.test(line.text) && !/\bwhere\b/i.test(line.text)) {
      out.push(lineDiagnostic(line, "warning", "Destructive SQL statement without a WHERE clause."));
    }
  });
  return out;
}

function shellDiagnostics(doc: Text): Diagnostic[] {
  const out: Diagnostic[] = [];
  forEachLine(doc, (line) => {
    if (/\brm\s+-rf\s+(\$[A-Za-z_][A-Za-z0-9_]*|\/|\*)/.test(line.text)) {
      out.push(lineDiagnostic(line, "warning", "Review this destructive remove command before running it."));
    }
    if (/^\s*export\s+[A-Za-z_][A-Za-z0-9_]*\s+\S/.test(line.text)) {
      out.push(lineDiagnostic(line, "warning", "Shell exports use `export NAME=value`."));
    }
  });
  return out;
}

function delimiterDiagnostics(text: string, style: "javascript" | "python" | "php"): Diagnostic[] {
  const out: Diagnostic[] = [];
  const stack: Array<{ ch: string; pos: number }> = [];
  const closeToOpen: Record<string, string> = { ")": "(", "]": "[", "}": "{" };
  let quote: string | null = null;
  let escaped = false;
  let lineComment = false;
  let blockComment = false;

  for (let i = 0; i < text.length; i += 1) {
    const ch = text[i];
    const next = text[i + 1];
    if (lineComment) {
      if (ch === "\n") lineComment = false;
      continue;
    }
    if (blockComment) {
      if (ch === "*" && next === "/") {
        blockComment = false;
        i += 1;
      }
      continue;
    }
    if (quote) {
      if (escaped) escaped = false;
      else if (ch === "\\") escaped = true;
      else if (ch === quote) quote = null;
      continue;
    }
    if ((style === "javascript" || style === "php") && ch === "/" && next === "/") {
      lineComment = true;
      i += 1;
      continue;
    }
    if ((style === "javascript" || style === "php") && ch === "/" && next === "*") {
      blockComment = true;
      i += 1;
      continue;
    }
    if (style === "python" && ch === "#") {
      lineComment = true;
      continue;
    }
    if (ch === "'" || ch === "\"" || (style === "javascript" && ch === "`")) {
      quote = ch;
      continue;
    }
    if (ch === "(" || ch === "[" || ch === "{") {
      stack.push({ ch, pos: i });
    } else if (ch === ")" || ch === "]" || ch === "}") {
      const want = closeToOpen[ch];
      const top = stack.pop();
      if (!top || top.ch !== want) {
        out.push({
          from: i,
          to: i + 1,
          severity: "error",
          source: "PortBay syntax",
          message: `Unmatched '${ch}'.`,
        });
      }
    }
  }
  for (const item of stack.slice(-5)) {
    out.push({
      from: item.pos,
      to: item.pos + 1,
      severity: "warning",
      source: "PortBay syntax",
      message: `Unclosed '${item.ch}'.`,
    });
  }
  return out;
}

function completionsFor(lang: SmartLanguageId, name: string, context: CompletionContext): CompletionResult | null {
  const word = context.matchBefore(/[A-Za-z_$][\w$.-]*/);
  if (!word || (word.from === word.to && !context.explicit)) return null;
  const options = completionOptions(lang, name);
  if (options.length === 0) return null;
  return {
    from: word.from,
    options,
    validFor: /^[\w$.-]*$/,
  };
}

function completionOptions(lang: SmartLanguageId, name: string): Completion[] {
  const lowerPath = name.toLowerCase();
  const lower = basename(name).toLowerCase();
  if (lang === "json" && lower === "package.json") {
    return keywords([
      ["scripts", "property", "npm script block"],
      ["dependencies", "property", "runtime dependencies"],
      ["devDependencies", "property", "development dependencies"],
      ["packageManager", "property", "declares npm/yarn/pnpm/bun"],
      ["type", "property", "module format"],
      ["engines", "property", "runtime constraints"],
    ]);
  }
  if (lang === "json" && lower === "tsconfig.json") {
    return keywords([
      ["compilerOptions", "property", "TypeScript compiler settings"],
      ["extends", "property", "inherit another config"],
      ["include", "property", "included globs"],
      ["exclude", "property", "excluded globs"],
      ["strict", "property", "strict type checking"],
      ["paths", "property", "import path aliases"],
    ]);
  }
  if (lang === "dotenv") {
    return keywords([
      ["PORT=", "variable", "dev server port"],
      ["HOST=", "variable", "bind host"],
      ["DATABASE_URL=", "variable", "database connection URL"],
      ["DB_HOST=", "variable", "database host"],
      ["DB_PORT=", "variable", "database port"],
      ["DB_DATABASE=", "variable", "database name"],
      ["DB_USERNAME=", "variable", "database user"],
      ["DB_PASSWORD=", "variable", "database password"],
      ["REDIS_URL=", "variable", "Redis connection URL"],
    ]);
  }
  if (lang === "sql") {
    return keywords([
      ["SELECT", "keyword", "read rows"],
      ["FROM", "keyword", "source table"],
      ["WHERE", "keyword", "filter rows"],
      ["JOIN", "keyword", "join tables"],
      ["GROUP BY", "keyword", "group rows"],
      ["ORDER BY", "keyword", "sort rows"],
      ["LIMIT", "keyword", "cap row count"],
      ["EXPLAIN", "keyword", "show query plan"],
    ]);
  }
  if (lang === "python") {
    return keywords([
      ["def", "keyword", "function declaration"],
      ["class", "keyword", "class declaration"],
      ["from", "keyword", "module import"],
      ["import", "keyword", "module import"],
      ["async", "keyword", "async function or context"],
      ["await", "keyword", "wait for awaitable"],
      ["with", "keyword", "context manager"],
      ["try", "keyword", "exception handling"],
    ]);
  }
  if (lang === "php") {
    return keywords([
      ["function", "keyword", "function declaration"],
      ["class", "keyword", "class declaration"],
      ["namespace", "keyword", "namespace declaration"],
      ["use", "keyword", "import class/trait/function"],
      ["public", "keyword", "public member"],
      ["private", "keyword", "private member"],
      ["protected", "keyword", "protected member"],
      ["return", "keyword", "return value"],
    ]);
  }
  if (lang === "shell") {
    return keywords([
      ["set -euo pipefail", "keyword", "strict shell mode"],
      ["export", "keyword", "export environment variable"],
      ["if", "keyword", "conditional"],
      ["then", "keyword", "conditional body"],
      ["fi", "keyword", "end conditional"],
      ["for", "keyword", "loop"],
      ["done", "keyword", "end loop"],
    ]);
  }
  if (lang === "yaml") {
    // File-aware YAML: compose files and GitHub workflows have well-known keys.
    if (lower.startsWith("docker-compose") || lower.startsWith("compose.")) {
      return keywords([
        ["services", "property", "container definitions"],
        ["image", "property", "image to run"],
        ["build", "property", "build context / Dockerfile"],
        ["ports", "property", "host:container port maps"],
        ["volumes", "property", "mounts and named volumes"],
        ["environment", "property", "environment variables"],
        ["env_file", "property", "load env from file"],
        ["restart", "property", "restart policy"],
        ["depends_on", "property", "start-order dependencies"],
        ["networks", "property", "attached networks"],
        ["command", "property", "override container command"],
        ["entrypoint", "property", "override entrypoint"],
        ["healthcheck", "property", "container health probe"],
        ["container_name", "property", "fixed container name"],
        ["labels", "property", "metadata labels"],
      ]);
    }
    if (lowerPath.includes(".github/workflows")) {
      return keywords([
        ["name", "property", "workflow / step name"],
        ["on", "property", "trigger events"],
        ["jobs", "property", "job definitions"],
        ["runs-on", "property", "runner image"],
        ["steps", "property", "job steps"],
        ["uses", "property", "action reference"],
        ["with", "property", "action inputs"],
        ["run", "property", "shell command"],
        ["env", "property", "environment variables"],
        ["if", "property", "conditional execution"],
        ["needs", "property", "job dependencies"],
        ["strategy", "property", "matrix strategy"],
        ["permissions", "property", "GITHUB_TOKEN scopes"],
      ]);
    }
    return [];
  }
  if (lang === "nginx") {
    return keywords([
      ["server", "keyword", "virtual server block"],
      ["location", "keyword", "URI match block"],
      ["upstream", "keyword", "backend pool block"],
      ["listen", "property", "port / address to bind"],
      ["server_name", "property", "hostnames served"],
      ["root", "property", "document root"],
      ["index", "property", "default index files"],
      ["try_files", "property", "fallback file resolution"],
      ["proxy_pass", "property", "forward to backend"],
      ["proxy_set_header", "property", "header sent upstream"],
      ["proxy_http_version", "property", "upstream HTTP version"],
      ["fastcgi_pass", "property", "forward to PHP-FPM"],
      ["fastcgi_param", "property", "FastCGI parameter"],
      ["include", "property", "include another config"],
      ["ssl_certificate", "property", "TLS certificate path"],
      ["ssl_certificate_key", "property", "TLS key path"],
      ["return", "property", "respond with status/URL"],
      ["rewrite", "property", "regex URL rewrite"],
      ["error_page", "property", "custom error pages"],
      ["access_log", "property", "access log path"],
      ["error_log", "property", "error log path"],
      ["add_header", "property", "response header"],
      ["client_max_body_size", "property", "upload size limit"],
      ["keepalive_timeout", "property", "keep-alive timeout"],
      ["gzip", "property", "response compression"],
      ["expires", "property", "cache expiry headers"],
      ["allow", "property", "permit address range"],
      ["deny", "property", "block address range"],
    ]);
  }
  if (lang === "systemd") {
    return keywords([
      ["Description=", "property", "[Unit] human-readable name"],
      ["After=", "property", "[Unit] start ordering"],
      ["Requires=", "property", "[Unit] hard dependency"],
      ["Wants=", "property", "[Unit] soft dependency"],
      ["Type=", "property", "[Service] startup type"],
      ["ExecStart=", "property", "[Service] start command"],
      ["ExecStop=", "property", "[Service] stop command"],
      ["ExecReload=", "property", "[Service] reload command"],
      ["Restart=", "property", "[Service] restart policy"],
      ["RestartSec=", "property", "[Service] restart delay"],
      ["User=", "property", "[Service] run as user"],
      ["Group=", "property", "[Service] run as group"],
      ["WorkingDirectory=", "property", "[Service] working dir"],
      ["Environment=", "property", "[Service] env variables"],
      ["EnvironmentFile=", "property", "[Service] env file"],
      ["StandardOutput=", "property", "[Service] stdout target"],
      ["LimitNOFILE=", "property", "[Service] fd limit"],
      ["NoNewPrivileges=", "property", "[Service] hardening"],
      ["ProtectSystem=", "property", "[Service] fs hardening"],
      ["PrivateTmp=", "property", "[Service] isolated /tmp"],
      ["WantedBy=", "property", "[Install] enable target"],
      ["OnCalendar=", "property", "[Timer] calendar schedule"],
      ["OnBootSec=", "property", "[Timer] delay after boot"],
      ["ListenStream=", "property", "[Socket] TCP/unix socket"],
    ]);
  }
  if (lang === "dockerfile") {
    return keywords([
      ["FROM", "keyword", "base image"],
      ["RUN", "keyword", "build-time command"],
      ["CMD", "keyword", "default command"],
      ["ENTRYPOINT", "keyword", "fixed entry command"],
      ["COPY", "keyword", "copy files into image"],
      ["ADD", "keyword", "copy + fetch/extract"],
      ["WORKDIR", "keyword", "working directory"],
      ["ENV", "keyword", "environment variable"],
      ["ARG", "keyword", "build argument"],
      ["EXPOSE", "keyword", "documented port"],
      ["VOLUME", "keyword", "mount point"],
      ["USER", "keyword", "run as user"],
      ["LABEL", "keyword", "image metadata"],
      ["HEALTHCHECK", "keyword", "container health probe"],
      ["SHELL", "keyword", "shell for RUN"],
      ["STOPSIGNAL", "keyword", "stop signal"],
    ]);
  }
  if (lang === "sshconfig") {
    if (lower === "sshd_config") {
      return keywords([
        ["Port", "property", "daemon listen port"],
        ["ListenAddress", "property", "bind address"],
        ["PermitRootLogin", "property", "allow root login"],
        ["PasswordAuthentication", "property", "allow passwords"],
        ["PubkeyAuthentication", "property", "allow public keys"],
        ["KbdInteractiveAuthentication", "property", "keyboard-interactive auth"],
        ["AuthorizedKeysFile", "property", "authorized_keys path"],
        ["AllowUsers", "property", "user allowlist"],
        ["AllowGroups", "property", "group allowlist"],
        ["MaxAuthTries", "property", "auth attempt cap"],
        ["ClientAliveInterval", "property", "keepalive interval"],
        ["ClientAliveCountMax", "property", "keepalive limit"],
        ["AllowTcpForwarding", "property", "permit forwarding"],
        ["GatewayPorts", "property", "remote-forward binding"],
        ["X11Forwarding", "property", "permit X11"],
        ["UsePAM", "property", "PAM integration"],
        ["Subsystem", "property", "e.g. sftp server"],
        ["Banner", "property", "pre-auth banner file"],
      ]);
    }
    return keywords([
      ["Host", "keyword", "host pattern block"],
      ["Match", "keyword", "conditional block"],
      ["HostName", "property", "real hostname / IP"],
      ["User", "property", "login user"],
      ["Port", "property", "remote port"],
      ["IdentityFile", "property", "private key path"],
      ["IdentitiesOnly", "property", "only listed keys"],
      ["ProxyJump", "property", "jump host chain"],
      ["ProxyCommand", "property", "custom transport"],
      ["ForwardAgent", "property", "agent forwarding"],
      ["LocalForward", "property", "local port forward"],
      ["RemoteForward", "property", "remote port forward"],
      ["DynamicForward", "property", "SOCKS proxy port"],
      ["ServerAliveInterval", "property", "keepalive interval"],
      ["ServerAliveCountMax", "property", "keepalive limit"],
      ["StrictHostKeyChecking", "property", "host key policy"],
      ["UserKnownHostsFile", "property", "known_hosts path"],
      ["ControlMaster", "property", "connection sharing"],
      ["ControlPath", "property", "control socket path"],
      ["ControlPersist", "property", "keep master alive"],
      ["ConnectTimeout", "property", "connect timeout"],
      ["Compression", "property", "transport compression"],
      ["AddKeysToAgent", "property", "auto-add to agent"],
      ["PreferredAuthentications", "property", "auth order"],
      ["SetEnv", "property", "send env variable"],
    ]);
  }
  if (lang === "apache") {
    return keywords([
      ["RewriteEngine", "property", "enable mod_rewrite"],
      ["RewriteRule", "property", "regex rewrite"],
      ["RewriteCond", "property", "rewrite condition"],
      ["RewriteBase", "property", "rewrite base path"],
      ["Redirect", "property", "simple redirect"],
      ["RedirectMatch", "property", "regex redirect"],
      ["Options", "property", "directory options"],
      ["AllowOverride", "property", ".htaccess scope"],
      ["Require", "property", "access control"],
      ["Header", "property", "response header"],
      ["ErrorDocument", "property", "custom error page"],
      ["DirectoryIndex", "property", "default index files"],
      ["FilesMatch", "keyword", "file-pattern block"],
      ["ExpiresActive", "property", "enable mod_expires"],
      ["ExpiresByType", "property", "cache expiry by MIME"],
      ["AddType", "property", "MIME type mapping"],
      ["SetEnv", "property", "environment variable"],
    ]);
  }
  if (lang === "javascript" || lang === "typescript") {
    return keywords([
      ["import", "keyword", "module import"],
      ["export", "keyword", "module export"],
      ["const", "keyword", "constant binding"],
      ["let", "keyword", "block binding"],
      ["async", "keyword", "async function"],
      ["await", "keyword", "wait for promise"],
      ["function", "keyword", "function declaration"],
      ["return", "keyword", "return value"],
      ["try", "keyword", "exception handling"],
    ]);
  }
  return [];
}

function hoverFor(lang: SmartLanguageId, view: EditorView, pos: number): Tooltip | null {
  const word = wordAt(view, pos);
  if (!word) return null;
  const text = HOVER_DOCS[lang]?.[word.text] ?? HOVER_DOCS.common[word.text];
  if (!text) return null;
  return {
    pos: word.from,
    end: word.to,
    above: true,
    create() {
      const dom = document.createElement("div");
      dom.className = "pb-smart-hover";
      dom.textContent = text;
      return { dom };
    },
  };
}

const HOVER_DOCS: Record<string, Record<string, string>> = {
  common: {
    PORT: "PortBay routes the public hostname to this local port.",
    DATABASE_URL: "Single connection string used by many Node, Python, and PHP frameworks.",
  },
  javascript: {
    async: "Creates a function that returns a Promise.",
    await: "Pauses an async function until a Promise settles.",
    import: "Loads bindings from another module.",
    export: "Exports bindings from this module.",
  },
  typescript: {
    type: "Declares a TypeScript type alias.",
    interface: "Declares an object shape.",
    satisfies: "Checks a value against a type without changing its inferred type.",
  },
  json: {
    scripts: "Commands available through npm, pnpm, yarn, or bun.",
    packageManager: "Pins the package manager PortBay should prefer.",
    compilerOptions: "TypeScript compiler settings.",
  },
  dotenv: {
    DB_HOST: "Database hostname.",
    DB_PORT: "Database port.",
    DB_DATABASE: "Database name.",
    REDIS_URL: "Redis connection URL.",
  },
  sql: {
    SELECT: "Read rows from one or more tables.",
    WHERE: "Filter rows before grouping or ordering.",
    EXPLAIN: "Show the database query plan.",
  },
  python: {
    def: "Declares a function.",
    class: "Declares a class.",
    async: "Declares async behavior.",
    await: "Waits for an awaitable in async code.",
  },
  php: {
    namespace: "Declares the namespace for PHP symbols in this file.",
    use: "Imports a class, trait, function, or constant.",
  },
  shell: {
    export: "Adds a variable to the environment of child processes.",
    "pipefail": "Makes a pipeline fail when any command in it fails.",
  },
  nginx: {
    proxy_pass: "Forwards matched requests to a backend (URL or upstream block).",
    try_files: "Checks each path in order; the last entry is the fallback (often a controller or =404).",
    server_name: "Hostnames this server block answers for; supports wildcards.",
    listen: "Port (and optionally address) to accept connections on; add `ssl` for TLS.",
    upstream: "Named pool of backend servers for proxy_pass load balancing.",
    fastcgi_pass: "Hands the request to a FastCGI backend such as PHP-FPM.",
    client_max_body_size: "Largest allowed request body — raise it for big uploads.",
  },
  systemd: {
    ExecStart: "Command run when the unit starts. Use an absolute binary path.",
    Restart: "When to restart the service: no, on-failure, always, …",
    WantedBy: "Target that pulls this unit in when enabled (usually multi-user.target).",
    OnCalendar: "Calendar schedule for timer units, e.g. `daily` or `Mon..Fri 02:00`.",
    EnvironmentFile: "File of NAME=value lines loaded into the service environment.",
    PrivateTmp: "Gives the service an isolated /tmp — cheap, effective hardening.",
  },
  dockerfile: {
    FROM: "Base image for the build; starts a new build stage.",
    COPY: "Copies files from the build context. Prefer COPY over ADD unless extracting archives.",
    ADD: "Like COPY but also fetches URLs and extracts archives — use COPY when in doubt.",
    ENTRYPOINT: "Fixed command; CMD then supplies its default arguments.",
    CMD: "Default command (or ENTRYPOINT arguments) — overridable at `docker run`.",
    HEALTHCHECK: "Command Docker runs to decide whether the container is healthy.",
  },
  sshconfig: {
    ProxyJump: "Connects through one or more jump hosts (comma-separated).",
    IdentityFile: "Private key to offer for this host.",
    ServerAliveInterval: "Seconds between keepalive probes — keeps idle sessions open.",
    StrictHostKeyChecking: "Whether unknown host keys are accepted: yes, no, or accept-new.",
    ControlMaster: "Shares one connection across sessions — much faster repeat logins.",
    PermitRootLogin: "Whether root may log in: yes, no, or prohibit-password.",
    LocalForward: "Forwards a local port to an address reachable from the server.",
  },
  apache: {
    RewriteRule: "mod_rewrite regex rule: pattern, substitution, [flags].",
    AllowOverride: "Which directives .htaccess files may set in this directory.",
    Require: "Access control: `all granted`, `all denied`, ip ranges, users.",
  },
};

function keywords(items: Array<[string, Completion["type"], string]>): Completion[] {
  // Boosted so curated entries sort above same-match buffer words from the
  // editor's completeAnyWord source.
  return items.map(([label, type, detail]) => ({ label, type, detail, boost: 2 }));
}

function wordAt(view: EditorView, pos: number): { from: number; to: number; text: string } | null {
  const line = view.state.doc.lineAt(pos);
  let from = pos;
  let to = pos;
  while (from > line.from && /[\w$-]/.test(view.state.doc.sliceString(from - 1, from))) from -= 1;
  while (to < line.to && /[\w$-]/.test(view.state.doc.sliceString(to, to + 1))) to += 1;
  if (from === to) return null;
  const text = view.state.doc.sliceString(from, to);
  return WORD.test(text) ? { from, to, text } : null;
}

function forEachLine(doc: Text, fn: (line: ReturnType<Text["line"]>) => void): void {
  for (let n = 1; n <= doc.lines; n += 1) fn(doc.line(n));
}

function lineDiagnostic(
  line: ReturnType<Text["line"]>,
  severity: Diagnostic["severity"],
  message: string,
): Diagnostic {
  return {
    from: line.from,
    to: Math.max(line.from, line.to),
    severity,
    source: "PortBay smart editor",
    message,
  };
}

function fileDiagnostic(doc: Text, severity: Diagnostic["severity"], message: string): Diagnostic {
  return {
    from: 0,
    to: Math.min(doc.length, Math.max(1, doc.length)),
    severity,
    source: "PortBay smart editor",
    message,
  };
}

function stripJsonc(text: string): { text: string } {
  let out = "";
  let quote: string | null = null;
  let escaped = false;
  let lineComment = false;
  let blockComment = false;
  for (let i = 0; i < text.length; i += 1) {
    const ch = text[i];
    const next = text[i + 1];
    if (lineComment) {
      if (ch === "\n") {
        lineComment = false;
        out += ch;
      } else {
        out += " ";
      }
      continue;
    }
    if (blockComment) {
      if (ch === "*" && next === "/") {
        out += "  ";
        i += 1;
        blockComment = false;
      } else {
        out += ch === "\n" ? "\n" : " ";
      }
      continue;
    }
    if (quote) {
      out += ch;
      if (escaped) escaped = false;
      else if (ch === "\\") escaped = true;
      else if (ch === quote) quote = null;
      continue;
    }
    if (ch === "\"" || ch === "'") {
      quote = ch;
      out += ch;
      continue;
    }
    if (ch === "/" && next === "/") {
      out += "  ";
      i += 1;
      lineComment = true;
      continue;
    }
    if (ch === "/" && next === "*") {
      out += "  ";
      i += 1;
      blockComment = true;
      continue;
    }
    if (ch === "," && /[\s\r\n]*[\]}]/.test(text.slice(i + 1))) {
      out += " ";
    } else {
      out += ch;
    }
  }
  return { text: out };
}

function jsonErrorPosition(message: string, text: string): number {
  const match = message.match(/position\s+(\d+)/i);
  if (match) return clamp(Number(match[1]), 0, text.length);
  const line = message.match(/line\s+(\d+)\s+column\s+(\d+)/i);
  if (line) {
    const targetLine = Number(line[1]);
    const targetCol = Number(line[2]);
    let offset = 0;
    const lines = text.split("\n");
    for (let i = 0; i < Math.max(0, targetLine - 1); i += 1) offset += lines[i].length + 1;
    return clamp(offset + Math.max(0, targetCol - 1), 0, text.length);
  }
  return 0;
}

function stripLineComment(line: string, marker: string): string {
  let quote: string | null = null;
  for (let i = 0; i < line.length; i += 1) {
    const ch = line[i];
    if (quote) {
      if (ch === quote && line[i - 1] !== "\\") quote = null;
    } else if (ch === "\"" || ch === "'") {
      quote = ch;
    } else if (ch === marker) {
      return line.slice(0, i);
    }
  }
  return line;
}

function hasUnclosedQuote(text: string, quote: string): boolean {
  let open = false;
  let escaped = false;
  for (const ch of text) {
    if (escaped) {
      escaped = false;
      continue;
    }
    if (ch === "\\") {
      escaped = true;
      continue;
    }
    if (ch === quote) open = !open;
  }
  return open;
}

function basename(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}

function clamp(n: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, Number.isFinite(n) ? n : min));
}

const smartTheme = EditorView.theme({
  ".cm-lintRange-info": {
    backgroundImage:
      "linear-gradient(45deg, transparent 65%, color-mix(in oklab, var(--color-accent) 70%, white) 80%, transparent 90%)",
  },
  ".pb-smart-hover": {
    maxWidth: "280px",
    padding: "6px 8px",
    border: "1px solid var(--color-border)",
    borderRadius: "6px",
    background: "var(--color-surface)",
    color: "var(--color-fg)",
    fontSize: "12px",
    lineHeight: "1.45",
    boxShadow: "0 8px 28px color-mix(in oklab, black 22%, transparent)",
  },
});
