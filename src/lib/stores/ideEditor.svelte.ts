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

function createIdeEditor() {
  let files = $state<OpenFile[]>([]);
  // `null` = the pinned Welcome tab is active.
  let active = $state<string | null>(null);

  return {
    get files() {
      return files;
    },
    get activeFile() {
      return active;
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

    /** Focus the Welcome tab. */
    showWelcome() {
      active = null;
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
        active = next ? next.path : null;
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
    },
  };
}

export const ideEditor = createIdeEditor();
