/**
 * Filesystem-aware path completion for the terminal next-command ghost — the
 * "Warp-level" half of the suggestion. When the line being typed ends in a path
 * argument (`cd clients/ak`, `cat /etc/ho`, `./scr`), this lists the matching
 * remote directory over SFTP and offers the rest of the entry as ghost text.
 *
 * It's precise (no model, no hallucination — the suggestion is a real directory
 * entry), correctly spaced (the remainder attaches straight after the cursor,
 * never glued onto the command word), and works on *any* SSH host since it
 * leans on the already-open SFTP session rather than a host-resident model.
 *
 * Kept import-light (no `$lib` runtime imports) so it stays unit-testable: the
 * directory lister and the cwd/home getters are injected by the caller, which
 * owns the IPC and the live cwd tracking.
 */

/** One remote directory entry, pared down to what completion needs. */
export interface DirEntry {
  name: string;
  isDir: boolean;
}

/** Lists an absolute remote directory, or null when it can't be read (missing
    dir, no SFTP, aborted). Never throws. */
export type DirLister = (absDir: string, signal: AbortSignal) => Promise<DirEntry[] | null>;

export interface PathCompleterDeps {
  list: DirLister;
  /** The shell's current absolute working directory, or null until known. */
  getCwd: () => string | null;
  /** The connection's absolute home directory, or null until known. */
  getHome: () => string | null;
  /** Resolve cwd/home before the first listing (memoized by the caller). */
  ensureReady?: () => Promise<void>;
  /** Directory-listing cache lifetime, ms. Default 8000. */
  cacheTtlMs?: number;
  /** Clock seam for tests. Default `Date.now`. */
  now?: () => number;
}

/** Commands whose argument is (almost) always a filesystem path, so a bare
    relative token like `cd a` should still path-complete. Other commands only
    path-complete when the token is itself explicitly a path (see below). */
const PATH_COMMANDS = new Set([
  "cd", "pushd", "ls", "ll", "la", "cat", "bat", "less", "more", "head", "tail", "tac", "nl",
  "vi", "vim", "nvim", "nano", "emacs", "micro", "code", "subl", "open", "xdg-open",
  "rm", "rmdir", "cp", "mv", "mkdir", "touch", "stat", "file", "du", "tree", "wc",
  "source", ".", "chmod", "chown", "chgrp", "ln", "tar", "zip", "unzip", "gzip", "gunzip",
  "rsync", "scp", "diff", "cmp", "realpath", "readlink", "basename", "dirname",
]);

/** A token that is unambiguously a path regardless of the command. */
function isExplicitPath(t: string): boolean {
  return (
    t.startsWith("/") ||
    t.startsWith("~") ||
    t.startsWith("./") ||
    t.startsWith("../") ||
    t === ".." ||
    t.includes("/")
  );
}

/** Strip any leading `dir/` so `/usr/bin/vim` is recognised as `vim`. */
function baseCommand(word: string): string {
  const slash = word.lastIndexOf("/");
  return slash === -1 ? word : word.slice(slash + 1);
}

/** Index of the opening quote of an unclosed `'…`/`"…` run, or -1. Lets a
    quoted path that contains spaces (`cd "my dir/fi`) tokenise correctly. */
function unclosedQuoteStart(line: string): number {
  let quote = "";
  let start = -1;
  for (let i = 0; i < line.length; i++) {
    const ch = line[i];
    if (quote) {
      if (ch === quote) {
        quote = "";
        start = -1;
      }
    } else if (ch === '"' || ch === "'") {
      quote = ch;
      start = i;
    }
  }
  return quote ? start : -1;
}

/**
 * The path token under the cursor, or null when the line isn't a path context.
 * The ghost only models a cursor-at-end line with no tabs (a Tab keystroke
 * resets it), so the token is simply the text after the last space.
 */
export function pathToken(line: string): string | null {
  let token: string;
  let isArg: boolean;
  const qStart = unclosedQuoteStart(line);
  if (qStart >= 0) {
    // Inside an open quote — the token is everything after it (spaces and all).
    token = line.slice(qStart + 1);
    isArg = line.slice(0, qStart).trim().length > 0;
  } else {
    const lastSpace = line.lastIndexOf(" ");
    token = lastSpace === -1 ? line.trimStart() : line.slice(lastSpace + 1);
    isArg = lastSpace !== -1;
    if (token.startsWith('"') || token.startsWith("'")) token = token.slice(1);
  }

  if (!isArg) {
    // No argument yet — only complete the command token when it's itself a
    // path (running `./script`, `/usr/local/bin/th…`).
    return isExplicitPath(token) ? token : null;
  }
  const cmd = baseCommand(line.trimStart().split(/\s+/, 1)[0] ?? "");
  if (PATH_COMMANDS.has(cmd) || isExplicitPath(token)) return token;
  return null;
}

/** True when the line ends in a filesystem path the user is typing. Such lines
    must be completed *only* from real directory listings — never a model or
    history, which could invent a path that doesn't exist. */
export function isPathContext(line: string): boolean {
  return pathToken(line) !== null;
}

/** Split a path token into its directory part (with trailing slash) and the
    basename fragment still being typed. */
export function splitToken(token: string): { dir: string; base: string } {
  const slash = token.lastIndexOf("/");
  if (slash === -1) return { dir: "", base: token };
  return { dir: token.slice(0, slash + 1), base: token.slice(slash + 1) };
}

/**
 * Resolve a (possibly relative, `~`, `.` / `..`-laden) directory reference to a
 * normalised absolute path, given the current cwd and home. Returns null when a
 * relative/`~` reference can't be resolved because cwd/home isn't known yet.
 */
export function resolvePath(input: string, cwd: string | null, home: string | null): string | null {
  let p = input;
  if (p === "~") {
    if (!home) return null;
    p = home;
  } else if (p.startsWith("~/")) {
    if (!home) return null;
    p = `${home}/${p.slice(2)}`;
  }
  if (!p.startsWith("/")) {
    if (!cwd) return null;
    p = `${cwd}/${p}`;
  }
  const stack: string[] = [];
  for (const seg of p.split("/")) {
    if (seg === "" || seg === ".") continue;
    if (seg === "..") {
      stack.pop();
      continue;
    }
    stack.push(seg);
  }
  return `/${stack.join("/")}`;
}

/** Longest common prefix of a non-empty list of strings. */
function commonPrefix(strings: string[]): string {
  let p = strings[0] ?? "";
  for (let i = 1; i < strings.length && p; i++) {
    const s = strings[i];
    let n = 0;
    while (n < p.length && n < s.length && p[n] === s[n]) n++;
    p = p.slice(0, n);
  }
  return p;
}

/**
 * Extract an absolute path from an OSC 7 payload (`file://host/abs/path`), which
 * many shells emit on every directory change. Returns null when the payload
 * isn't a usable absolute path.
 */
export function parseOsc7Cwd(data: string): string | null {
  if (!data) return null;
  let s = data.trim();
  const m = s.match(/^file:\/\/[^/]*(\/.*)$/);
  if (m) s = m[1];
  if (!s.startsWith("/")) return null;
  try {
    s = decodeURIComponent(s);
  } catch {
    /* leave as-is if it isn't valid percent-encoding */
  }
  if (s.length > 1 && s.endsWith("/")) s = s.slice(0, -1);
  return s;
}

export interface PathCompleter {
  /** The ghost remainder for `line`, or null when there's nothing precise to
      suggest. Only lists a directory when the line is actually a path context. */
  complete: (line: string, signal: AbortSignal) => Promise<string | null>;
  /** Drop cached directory listings (call after a command that may have changed
      the filesystem, or on a cwd change). */
  invalidate: () => void;
}

export function createPathCompleter(deps: PathCompleterDeps): PathCompleter {
  const ttl = deps.cacheTtlMs ?? 8000;
  const now = deps.now ?? (() => Date.now());
  const cache = new Map<string, { t: number; entries: DirEntry[] | null }>();

  async function listCached(absDir: string, signal: AbortSignal): Promise<DirEntry[] | null> {
    const hit = cache.get(absDir);
    if (hit && now() - hit.t < ttl) return hit.entries;
    const entries = await deps.list(absDir, signal);
    if (signal.aborted) return null;
    cache.set(absDir, { t: now(), entries });
    return entries;
  }

  async function complete(line: string, signal: AbortSignal): Promise<string | null> {
    const token = pathToken(line);
    if (token === null) return null;

    if (deps.ensureReady) await deps.ensureReady();
    if (signal.aborted) return null;

    const { dir, base } = splitToken(token);
    const absDir = resolvePath(dir, deps.getCwd(), deps.getHome());
    if (!absDir) return null;

    const entries = await listCached(absDir, signal);
    if (!entries || entries.length === 0) return null;

    // Hidden entries only surface once the user types the leading dot.
    const wantHidden = base.startsWith(".");
    const matches = entries.filter(
      (e) => e.name.startsWith(base) && (wantHidden || !e.name.startsWith(".")),
    );
    if (matches.length === 0) return null;

    let remainder: string;
    if (matches.length === 1) {
      const m = matches[0];
      // A directory gets a trailing slash so the next keystroke descends into it.
      remainder = m.name.slice(base.length) + (m.isDir ? "/" : "");
    } else {
      // Ambiguous: only offer the unambiguous shared prefix (shell-style), never
      // a trailing slash (we don't know which one wins).
      remainder = commonPrefix(matches.map((m) => m.name)).slice(base.length);
    }
    return remainder.length > 0 ? remainder : null;
  }

  return {
    complete,
    invalidate: () => cache.clear(),
  };
}
