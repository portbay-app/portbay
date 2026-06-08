/**
 * ideEditor — open-file tab state for the VS Code–style host workspace's editor
 * area. Tracks which remote files are open, which is active, and each file's
 * dirty flag. A pinned "Welcome" tab is represented by `activeFile === null`.
 *
 * Unlike `ideLayout`, this is **not** persisted: open files are tied to a live
 * SFTP session for one host, so they reset when the workspace switches hosts
 * (the workspace calls `reset()` on host change). Memory-only, like the session
 * secret cache.
 */
import { posixBasename } from "$lib/sftp";

export interface OpenFile {
  /** Absolute POSIX path on the remote host (the stable tab key). */
  path: string;
  /** Display name (basename of `path`). */
  name: string;
  /** Unsaved edits pending. */
  dirty: boolean;
}

/** Sentinel tab key for the Files (remote file-manager) tab. Doesn't start
 *  with "/" so it can never collide with a real remote path. */
export const FILES_TAB = "tab:files";

function createIdeEditor() {
  let files = $state<OpenFile[]>([]);
  // `null` = no file tab is active (the Welcome tab, if open, shows).
  let active = $state<string | null>(null);
  // Whether the Welcome tab is present. It's the landing view, but it's
  // dismissable (via the Home toggle or its tab ×) so it isn't perpetually
  // occupying the editor area once you're working in a host.
  let welcomeOpen = $state(true);
  // The Files tab — a Finder-style remote file manager in the editor area,
  // opened by clicking a folder in the Explorer tree or SFTP sidebar. Like
  // Welcome it's a singleton tab; `filesRequest` carries "navigate here"
  // into the mounted pane (the nonce makes the same path re-applyable).
  let filesOpen = $state(false);
  let filesRequest = $state<{ path: string; nonce: number } | null>(null);
  let filesNonce = 0;

  return {
    get files() {
      return files;
    },
    get activeFile() {
      return active;
    },
    /** Whether the Welcome tab is currently present. */
    get welcomeOpen() {
      return welcomeOpen;
    },
    /** Whether the Welcome view is the one on screen (no file active + open). */
    get welcomeActive() {
      return active === null && welcomeOpen;
    },
    /** Whether the Files (file-manager) tab is present. */
    get filesOpen() {
      return filesOpen;
    },
    /** Whether the Files tab is the one on screen. */
    get filesActive() {
      return active === FILES_TAB;
    },
    /** The latest "navigate here" request for the Files pane. */
    get filesRequest() {
      return filesRequest;
    },
    /** Is there an open file with unsaved edits? (For close-guard / status.) */
    get anyDirty() {
      return files.some((f) => f.dirty);
    },

    /** Open `path` (focusing it if already open) and make it the active tab. */
    open(path: string) {
      if (!files.some((f) => f.path === path)) {
        files = [...files, { path, name: posixBasename(path), dirty: false }];
      }
      active = path;
    },

    /** Open + focus the Files tab, optionally navigating it to `path`. */
    openFiles(path?: string) {
      filesOpen = true;
      active = FILES_TAB;
      if (path) filesRequest = { path, nonce: ++filesNonce };
    },

    /** Close the Files tab; if active, fall back like a file close would. */
    closeFiles() {
      filesOpen = false;
      if (active === FILES_TAB) active = files.at(-1)?.path ?? null;
    },

    /** Show + focus the Welcome tab. */
    showWelcome() {
      welcomeOpen = true;
      active = null;
    },

    /** Dismiss the Welcome tab; fall back to the last open file (or Files). */
    closeWelcome() {
      welcomeOpen = false;
      if (active === null) active = files.at(-1)?.path ?? (filesOpen ? FILES_TAB : null);
    },

    /** Home button: show Welcome, or dismiss it if it's already on screen. */
    toggleWelcome() {
      if (active === null && welcomeOpen) this.closeWelcome();
      else this.showWelcome();
    },

    /** Focus an already-open file (no-op if not open). */
    focus(path: string) {
      if (files.some((f) => f.path === path)) active = path;
    },

    /** Close a tab; if it was active, fall back to a neighbour or Welcome. */
    close(path: string) {
      const idx = files.findIndex((f) => f.path === path);
      if (idx === -1) return;
      const wasActive = active === path;
      files = files.filter((f) => f.path !== path);
      if (wasActive) {
        const next = files[idx] ?? files[idx - 1];
        active = next ? next.path : filesOpen ? FILES_TAB : null;
      }
    },

    /** Mark a file's dirty state (called by its editor on change / save). */
    setDirty(path: string, dirty: boolean) {
      const f = files.find((x) => x.path === path);
      if (f) f.dirty = dirty;
    },

    /** A renamed/saved-as path: update the tab key + name in place. */
    rename(from: string, to: string) {
      const f = files.find((x) => x.path === from);
      if (!f) return;
      f.path = to;
      f.name = posixBasename(to);
      if (active === from) active = to;
    },

    /** Drop all open files (host switch / workspace teardown). */
    reset() {
      files = [];
      active = null;
      welcomeOpen = true;
      filesOpen = false;
      filesRequest = null;
    },
  };
}

export const ideEditor = createIdeEditor();
