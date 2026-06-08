/**
 * Shared CodeMirror 6 setup for the host workspace editor.
 *
 * `languageFor` maps a filename to a language extension so remote files are
 * syntax-highlighted instead of rendered as flat text. The common languages use
 * their dedicated parser packs (`@codemirror/lang-*`); the long tail (shell,
 * toml, ini, ruby, …) uses `@codemirror/legacy-modes` via `StreamLanguage`, so a
 * wide spread of server files gets coloured without a pack each.
 *
 * Theming is split from the language: `editorChromeTheme` paints the editor
 * surface from the app's CSS tokens (`var(--color-*)`), so it follows the
 * light/dark theme automatically. `highlightFor` returns the token-colour style
 * for the resolved theme — oneDark's palette in dark, CodeMirror's default
 * (readable on a light surface) in light — and is swapped live via a compartment
 * by the editor component when the theme changes.
 */
import { json } from "@codemirror/lang-json";
import { sql } from "@codemirror/lang-sql";
import { javascript } from "@codemirror/lang-javascript";
import { python } from "@codemirror/lang-python";
import { yaml } from "@codemirror/lang-yaml";
import { markdown } from "@codemirror/lang-markdown";
import { html } from "@codemirror/lang-html";
import { css } from "@codemirror/lang-css";
import { rust } from "@codemirror/lang-rust";
import { go } from "@codemirror/lang-go";
import { php } from "@codemirror/lang-php";
import { xml } from "@codemirror/lang-xml";
import { cpp } from "@codemirror/lang-cpp";
import {
  StreamLanguage,
  syntaxHighlighting,
  defaultHighlightStyle,
} from "@codemirror/language";
import { shell } from "@codemirror/legacy-modes/mode/shell";
import { toml } from "@codemirror/legacy-modes/mode/toml";
import { properties } from "@codemirror/legacy-modes/mode/properties";
import { dockerFile } from "@codemirror/legacy-modes/mode/dockerfile";
import { ruby } from "@codemirror/legacy-modes/mode/ruby";
import { lua } from "@codemirror/legacy-modes/mode/lua";
import { diff } from "@codemirror/legacy-modes/mode/diff";
import {
  java,
  kotlin,
  scala,
  csharp,
  dart,
  objectiveC,
  objectiveCpp,
} from "@codemirror/legacy-modes/mode/clike";
import { clojure } from "@codemirror/legacy-modes/mode/clojure";
import { cmake } from "@codemirror/legacy-modes/mode/cmake";
import { coffeeScript } from "@codemirror/legacy-modes/mode/coffeescript";
import { commonLisp } from "@codemirror/legacy-modes/mode/commonlisp";
import { crystal } from "@codemirror/legacy-modes/mode/crystal";
import { d } from "@codemirror/legacy-modes/mode/d";
import { elm } from "@codemirror/legacy-modes/mode/elm";
import { erlang } from "@codemirror/legacy-modes/mode/erlang";
import { groovy } from "@codemirror/legacy-modes/mode/groovy";
import { haskell } from "@codemirror/legacy-modes/mode/haskell";
import { http } from "@codemirror/legacy-modes/mode/http";
import { jinja2 } from "@codemirror/legacy-modes/mode/jinja2";
import { julia } from "@codemirror/legacy-modes/mode/julia";
import { oCaml, fSharp } from "@codemirror/legacy-modes/mode/mllike";
import { nginx } from "@codemirror/legacy-modes/mode/nginx";
import { perl } from "@codemirror/legacy-modes/mode/perl";
import { powerShell } from "@codemirror/legacy-modes/mode/powershell";
import { protobuf } from "@codemirror/legacy-modes/mode/protobuf";
import { pug } from "@codemirror/legacy-modes/mode/pug";
import { puppet } from "@codemirror/legacy-modes/mode/puppet";
import { r } from "@codemirror/legacy-modes/mode/r";
import { sass } from "@codemirror/legacy-modes/mode/sass";
import { scheme } from "@codemirror/legacy-modes/mode/scheme";
import { stex } from "@codemirror/legacy-modes/mode/stex";
import { stylus } from "@codemirror/legacy-modes/mode/stylus";
import { swift } from "@codemirror/legacy-modes/mode/swift";
import { tcl } from "@codemirror/legacy-modes/mode/tcl";
import { vb } from "@codemirror/legacy-modes/mode/vb";
import { EditorView } from "@codemirror/view";
import { oneDarkHighlightStyle } from "@codemirror/theme-one-dark";
import type { Extension } from "@codemirror/state";
import type { ResolvedTheme } from "$lib/stores/theme.svelte";

const stream = (mode: Parameters<typeof StreamLanguage.define>[0]): Extension[] => [
  StreamLanguage.define(mode),
];

/** Fixed-name prose files (no extension) that read well with markdown styling:
    headings/lists/links get coloured, plain prose stays plain. */
const PROSE_NAMES = new Set([
  "readme", "license", "copying", "notice", "authors",
  "changelog", "changes", "news", "todo", "contributing",
]);

/** Well-known `Field: value` text files (RFC 9116 security.txt and friends) —
    the properties mode colours the field names like a config file. */
const FIELD_TXT_NAMES = new Set([
  "robots.txt", "security.txt", "humans.txt", "ads.txt", "app-ads.txt",
]);

/** Pick a language extension from a file name, or `[]` for plaintext. */
export function languageFor(name: string): Extension[] {
  const lower = name.toLowerCase();
  const ext = lower.split(".").pop() ?? "";

  // Extensionless / fixed-name files the switch below would miss.
  if (lower === "dockerfile" || lower.endsWith(".dockerfile")) return stream(dockerFile);
  if (lower === ".env" || lower.startsWith(".env.")) return stream(properties);
  if (lower === ".gitignore" || lower === ".dockerignore") return stream(properties);
  // Make's recipes are shell — closest fit, colours comments/vars/strings.
  if (lower === "makefile" || lower === "gnumakefile" || ext === "mk") return stream(shell);
  if (lower === "cmakelists.txt") return stream(cmake);
  if (lower === "jenkinsfile") return stream(groovy);
  if (
    ["gemfile", "rakefile", "vagrantfile", "brewfile", "podfile", "fastfile", "capfile"]
      .includes(lower)
  ) {
    return stream(ruby);
  }
  if (lower === "procfile") return stream(properties);
  if (lower === "cargo.lock") return stream(toml);
  if (lower === "yarn.lock") return [yaml()];
  if (lower === "composer.lock") return [json()];
  if (lower === "nginx.conf" || (lower.includes("nginx") && ext === "conf")) return stream(nginx);
  // llms.txt is markdown by spec (llmstxt.org); llm.txt is a common variant.
  if (lower === "llm.txt" || lower === "llms.txt" || lower === "llms-full.txt") {
    return [markdown()];
  }
  if (FIELD_TXT_NAMES.has(lower)) return stream(properties);
  if (!lower.includes(".") && PROSE_NAMES.has(lower)) return [markdown()];

  switch (ext) {
    case "json":
    case "jsonc":
    case "json5":
    case "jsonl":
    case "ndjson":
    case "webmanifest":
      return [json()];
    case "sql":
      return [sql()];
    case "js":
    case "mjs":
    case "cjs":
      return [javascript()];
    case "jsx":
      return [javascript({ jsx: true })];
    case "ts":
    case "mts":
    case "cts":
      return [javascript({ typescript: true })];
    case "tsx":
      return [javascript({ typescript: true, jsx: true })];
    case "py":
    case "pyw":
      return [python()];
    case "yaml":
    case "yml":
      return [yaml()];
    case "md":
    case "markdown":
    // Prose: markdown styles headings/lists/links when they're there and
    // leaves plain paragraphs plain — the neutral "neat text" default.
    case "txt":
    case "text":
      return [markdown()];
    case "html":
    case "htm":
    case "xhtml":
    case "svelte":
    case "vue":
    case "astro":
    case "ejs":
    case "erb":
    case "hbs":
    case "mustache":
      return [html()];
    case "css":
    case "scss":
    case "less":
      return [css()];
    case "rs":
      return [rust()];
    case "go":
      return [go()];
    case "php":
      return [php()];
    case "xml":
    case "svg":
    case "plist":
    case "xsl":
    case "xslt":
    case "xsd":
      return [xml()];
    case "c":
    case "h":
    case "cc":
    case "cpp":
    case "cxx":
    case "hpp":
    case "hh":
      return [cpp()];
    case "sh":
    case "bash":
    case "zsh":
    case "fish":
    case "ksh":
      return stream(shell);
    case "toml":
      return stream(toml);
    case "ini":
    case "conf":
    case "cfg":
    case "env":
    case "properties":
    case "htaccess":
    // systemd units / .desktop entries are INI-shaped.
    case "service":
    case "socket":
    case "timer":
    case "target":
    case "desktop":
    // git/tool dotfiles (`.gitconfig` splits to ext "gitconfig").
    case "gitattributes":
    case "gitmodules":
    case "gitconfig":
    case "npmrc":
    case "yarnrc":
    case "editorconfig":
      return stream(properties);
    case "rb":
    // No Elixir/HCL modes exist — Ruby's grammar (do/end blocks, symbols,
    // string interpolation) is the standard close-enough stand-in for both.
    case "ex":
    case "exs":
    case "hcl":
    case "tf":
    case "tfvars":
      return stream(ruby);
    case "lua":
      return stream(lua);
    case "diff":
    case "patch":
      return stream(diff);
    case "java":
      return stream(java);
    case "kt":
    case "kts":
      return stream(kotlin);
    case "scala":
    case "sbt":
      return stream(scala);
    case "gradle":
    case "groovy":
      return stream(groovy);
    case "cs":
    case "csx":
      return stream(csharp);
    case "dart":
      return stream(dart);
    case "m":
      return stream(objectiveC);
    case "mm":
      return stream(objectiveCpp);
    case "swift":
      return stream(swift);
    case "pl":
    case "pm":
      return stream(perl);
    case "r":
      return stream(r);
    case "jl":
      return stream(julia);
    case "hs":
      return stream(haskell);
    case "erl":
    case "hrl":
      return stream(erlang);
    case "clj":
    case "cljs":
    case "cljc":
    case "edn":
      return stream(clojure);
    case "lisp":
    case "cl":
    case "el":
      return stream(commonLisp);
    case "scm":
    case "ss":
    case "rkt":
      return stream(scheme);
    case "ml":
    case "mli":
      return stream(oCaml);
    case "fs":
    case "fsi":
    case "fsx":
      return stream(fSharp);
    case "ps1":
    case "psm1":
    case "psd1":
      return stream(powerShell);
    case "proto":
      return stream(protobuf);
    case "cmake":
      return stream(cmake);
    case "coffee":
      return stream(coffeeScript);
    case "elm":
      return stream(elm);
    case "sass":
      return stream(sass);
    case "styl":
      return stream(stylus);
    case "pug":
    case "jade":
      return stream(pug);
    case "j2":
    case "jinja":
    case "jinja2":
    case "njk":
    case "twig":
      return stream(jinja2);
    case "tex":
    case "sty":
    case "bib":
      return stream(stex);
    case "cr":
      return stream(crystal);
    case "d":
      return stream(d);
    case "tcl":
      return stream(tcl);
    case "pp":
      return stream(puppet);
    // VS Code REST-client style request files.
    case "http":
    case "rest":
      return stream(http);
    case "vb":
      return stream(vb);
    default:
      return [];
  }
}

/** A short human label for the status bar's language indicator. */
export function languageLabel(name: string): string {
  const lower = name.toLowerCase();
  const ext = lower.split(".").pop() ?? "";

  // Fixed-name files first, mirroring languageFor's special cases.
  const FIXED: Record<string, string> = {
    dockerfile: "Dockerfile",
    makefile: "Makefile",
    gnumakefile: "Makefile",
    "cmakelists.txt": "CMake",
    jenkinsfile: "Groovy",
    gemfile: "Ruby",
    rakefile: "Ruby",
    vagrantfile: "Ruby",
    brewfile: "Ruby",
    podfile: "Ruby",
    fastfile: "Ruby",
    capfile: "Ruby",
    procfile: "Config",
    "cargo.lock": "TOML",
    "yarn.lock": "YAML",
    "composer.lock": "JSON",
    "nginx.conf": "Nginx",
    "llm.txt": "Markdown",
    "llms.txt": "Markdown",
    "llms-full.txt": "Markdown",
    ".env": "Dotenv",
    ".gitignore": "Config",
    ".dockerignore": "Config",
  };
  if (FIXED[lower]) return FIXED[lower];
  if (lower.startsWith(".env.")) return "Dotenv";
  if (FIELD_TXT_NAMES.has(lower)) return "Config";
  if (!lower.includes(".") && PROSE_NAMES.has(lower)) return "Markdown";

  const MAP: Record<string, string> = {
    json: "JSON",
    jsonc: "JSON",
    webmanifest: "JSON",
    sql: "SQL",
    js: "JavaScript",
    mjs: "JavaScript",
    cjs: "JavaScript",
    jsx: "JavaScript",
    ts: "TypeScript",
    tsx: "TypeScript",
    mts: "TypeScript",
    cts: "TypeScript",
    md: "Markdown",
    markdown: "Markdown",
    yaml: "YAML",
    yml: "YAML",
    toml: "TOML",
    sh: "Shell",
    bash: "Shell",
    zsh: "Shell",
    fish: "Shell",
    rs: "Rust",
    py: "Python",
    pyw: "Python",
    rb: "Ruby",
    php: "PHP",
    go: "Go",
    lua: "Lua",
    html: "HTML",
    htm: "HTML",
    xml: "XML",
    svg: "SVG",
    css: "CSS",
    scss: "SCSS",
    less: "Less",
    svelte: "Svelte",
    vue: "Vue",
    c: "C",
    h: "C",
    cpp: "C++",
    cc: "C++",
    cxx: "C++",
    hpp: "C++",
    conf: "Config",
    cfg: "Config",
    ini: "INI",
    env: "Dotenv",
    properties: "Config",
    htaccess: "Config",
    service: "Config",
    socket: "Config",
    timer: "Config",
    target: "Config",
    desktop: "Config",
    gitattributes: "Config",
    gitmodules: "Config",
    gitconfig: "Config",
    npmrc: "Config",
    yarnrc: "Config",
    editorconfig: "Config",
    diff: "Diff",
    patch: "Diff",
    json5: "JSON",
    jsonl: "JSON",
    ndjson: "JSON",
    plist: "XML",
    xsl: "XML",
    xslt: "XML",
    xsd: "XML",
    astro: "Astro",
    ejs: "HTML",
    erb: "HTML",
    hbs: "HTML",
    mustache: "HTML",
    ksh: "Shell",
    mk: "Makefile",
    dockerfile: "Dockerfile",
    java: "Java",
    kt: "Kotlin",
    kts: "Kotlin",
    scala: "Scala",
    sbt: "Scala",
    gradle: "Gradle",
    groovy: "Groovy",
    cs: "C#",
    csx: "C#",
    dart: "Dart",
    m: "Objective-C",
    mm: "Objective-C++",
    swift: "Swift",
    pl: "Perl",
    pm: "Perl",
    r: "R",
    jl: "Julia",
    hs: "Haskell",
    erl: "Erlang",
    hrl: "Erlang",
    ex: "Elixir",
    exs: "Elixir",
    hcl: "HCL",
    tf: "Terraform",
    tfvars: "Terraform",
    clj: "Clojure",
    cljs: "Clojure",
    cljc: "Clojure",
    edn: "Clojure",
    lisp: "Lisp",
    cl: "Lisp",
    el: "Lisp",
    scm: "Scheme",
    ss: "Scheme",
    rkt: "Scheme",
    ml: "OCaml",
    mli: "OCaml",
    fs: "F#",
    fsi: "F#",
    fsx: "F#",
    ps1: "PowerShell",
    psm1: "PowerShell",
    psd1: "PowerShell",
    proto: "Protobuf",
    cmake: "CMake",
    coffee: "CoffeeScript",
    elm: "Elm",
    sass: "Sass",
    styl: "Stylus",
    pug: "Pug",
    jade: "Pug",
    j2: "Jinja",
    jinja: "Jinja",
    jinja2: "Jinja",
    njk: "Jinja",
    twig: "Twig",
    tex: "LaTeX",
    sty: "LaTeX",
    bib: "BibTeX",
    cr: "Crystal",
    d: "D",
    tcl: "Tcl",
    pp: "Puppet",
    http: "HTTP",
    rest: "HTTP",
    vb: "Visual Basic",
    txt: "Plain Text",
    text: "Plain Text",
  };
  if (!name.includes(".")) return "Plain Text";
  return MAP[ext] ?? "Plain Text";
}

/**
 * Editor chrome (surface, gutters, cursor, selection) painted from the app's CSS
 * tokens so it tracks the active light/dark theme with no per-theme branch. Only
 * the syntax-highlight palette (see `highlightFor`) needs swapping on theme change.
 */
export const editorChromeTheme: Extension = EditorView.theme({
  "&": {
    height: "100%",
    color: "var(--color-fg)",
    backgroundColor: "transparent",
  },
  ".cm-scroller": {
    fontFamily: "var(--font-mono, ui-monospace, monospace)",
  },
  ".cm-content": { caretColor: "var(--color-accent)" },
  ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--color-accent)" },
  "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection":
    { backgroundColor: "color-mix(in oklab, var(--color-accent) 28%, transparent)" },
  ".cm-gutters": {
    backgroundColor: "transparent",
    color: "var(--color-fg-subtle)",
    border: "none",
  },
  ".cm-activeLine": {
    backgroundColor: "color-mix(in oklab, var(--color-fg) 6%, transparent)",
  },
  ".cm-activeLineGutter": {
    backgroundColor: "color-mix(in oklab, var(--color-fg) 8%, transparent)",
    color: "var(--color-fg-muted)",
  },
  ".cm-lineNumbers .cm-gutterElement": { padding: "0 6px 0 12px" },
  "&.cm-focused": { outline: "none" },

  // --- Tooltips (autocomplete, lint, hover) ---
  // CodeMirror's defaults are a light panel with light-grey text — unreadable
  // on a dark surface. Paint them like the app's popovers from theme tokens so
  // they track light/dark automatically.
  ".cm-tooltip": {
    border: "1px solid var(--color-border)",
    borderRadius: "8px",
    backgroundColor: "var(--color-surface)",
    color: "var(--color-fg)",
    boxShadow: "0 8px 28px color-mix(in oklab, black 25%, transparent)",
    overflow: "hidden",
  },
  ".cm-tooltip.cm-tooltip-autocomplete > ul": {
    fontFamily: "var(--font-mono, ui-monospace, monospace)",
    fontSize: "12px",
    maxHeight: "240px",
    minWidth: "220px",
    maxWidth: "420px",
    padding: "3px",
  },
  ".cm-tooltip.cm-tooltip-autocomplete > ul > li": {
    display: "flex",
    alignItems: "center",
    padding: "3px 8px",
    borderRadius: "5px",
    color: "var(--color-fg-muted)",
    lineHeight: "1.5",
  },
  ".cm-tooltip.cm-tooltip-autocomplete > ul > li[aria-selected]": {
    backgroundColor: "color-mix(in oklab, var(--color-accent) 18%, transparent)",
    color: "var(--color-fg)",
  },
  ".cm-completionIcon": {
    width: "1.1em",
    marginRight: "6px",
    color: "var(--color-fg-subtle)",
    opacity: "1",
    fontSize: "11px",
  },
  ".cm-completionLabel": {
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
  },
  ".cm-completionMatchedText": {
    textDecoration: "none",
    fontWeight: "600",
    color: "var(--color-accent)",
  },
  ".cm-completionDetail": {
    marginLeft: "auto",
    paddingLeft: "12px",
    fontStyle: "normal",
    fontSize: "10.5px",
    color: "var(--color-fg-subtle)",
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
  },
  ".cm-tooltip.cm-completionInfo": {
    padding: "8px 10px",
    maxWidth: "320px",
    fontSize: "12px",
    lineHeight: "1.45",
  },
  ".cm-tooltip-lint": { padding: "2px" },
  ".cm-diagnostic": {
    padding: "4px 8px",
    fontSize: "12px",
  },
});

/** Token-colour highlight style for the resolved theme. */
export function highlightFor(resolved: ResolvedTheme): Extension {
  return resolved === "dark"
    ? syntaxHighlighting(oneDarkHighlightStyle)
    : syntaxHighlighting(defaultHighlightStyle, { fallback: true });
}
