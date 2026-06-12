/**
 * Deep (recursive) server-side file search over the cached SFTP session — the
 * "find files" feature every desktop SFTP client ships. The backend walks the
 * tree breadth-first and streams hits in batches on `portbay://sftp-search`,
 * so results fill in live on big or slow trees; searches are cancellable
 * mid-walk.
 *
 * Query syntax: plain text matches as a case-insensitive substring; `*` / `?`
 * switch to a glob over the whole name (`*.zip`, `config.?ml`, `IMG_2024*`) —
 * which is also how you search by file type.
 */
import { invokeQuiet } from "$lib/ipc";
import type { SftpEntry } from "$lib/types/sshTunnels";

export interface SftpSearchUpdate {
  /** All hits so far (accumulated across batches). */
  hits: SftpEntry[];
  /** Directory entries examined so far. */
  scanned: number;
  done: boolean;
  /** True when the walk stopped at a result/scan/depth cap. */
  truncated: boolean;
}

export interface RunningSearch {
  id: string;
  /** Stop the walk; the final `done` update still arrives with partial hits. */
  cancel: () => void;
}

interface Batch {
  id: string;
  hits: SftpEntry[];
  scanned: number;
  done: boolean;
  truncated: boolean;
}

/** A finished search's payload, as kept by {@link SftpSearchCache}. */
export interface CachedSearch {
  hits: SftpEntry[];
  scanned: number;
  truncated: boolean;
}

const isGlob = (q: string) => q.includes("*") || q.includes("?");

/**
 * Session cache over completed deep searches, so retyping (or extending) a
 * query doesn't re-walk the server tree from zero.
 *
 * Two ways a query answers instantly:
 *  - an identical completed search under the same root, or
 *  - for plain substring queries, a completed search for any *substring* of
 *    it — `clients` can only match a subset of what `client` matched, so the
 *    cached superset is narrowed client-side and is authoritative.
 *
 * Only complete, untruncated, uncancelled walks are stored: a partial walk
 * may have missed matches, so it can't seed narrowing. Call `invalidate()`
 * after anything that changes the remote tree (delete/upload/refresh).
 */
export class SftpSearchCache {
  private byRoot = new Map<string, Map<string, CachedSearch>>();
  private static readonly MAX_PER_ROOT = 24;

  resolve(root: string, query: string): CachedSearch | null {
    const q = query.trim().toLowerCase();
    if (!q) return null;
    const m = this.byRoot.get(root);
    if (!m) return null;
    const exact = m.get(q);
    if (exact) return exact;
    if (isGlob(q)) return null;
    // Longest cached substring of the query → smallest superset to narrow.
    let best: string | null = null;
    for (const p of m.keys()) {
      if (!isGlob(p) && q.includes(p) && (best === null || p.length > best.length)) best = p;
    }
    if (best === null) return null;
    const base = m.get(best);
    if (!base) return null;
    const derived: CachedSearch = {
      hits: base.hits.filter((e) => e.name.toLowerCase().includes(q)),
      scanned: base.scanned,
      truncated: false,
    };
    this.store(root, q, derived);
    return derived;
  }

  store(root: string, query: string, result: CachedSearch): void {
    const q = query.trim().toLowerCase();
    if (!q || result.truncated) return;
    let m = this.byRoot.get(root);
    if (!m) {
      m = new Map();
      this.byRoot.set(root, m);
    }
    m.delete(q); // re-insert as most recent
    m.set(q, result);
    while (m.size > SftpSearchCache.MAX_PER_ROOT) {
      const oldest = m.keys().next().value;
      if (oldest === undefined) break;
      m.delete(oldest);
    }
  }

  /** Drop everything — the remote tree (or local mirror of it) changed. */
  invalidate(): void {
    this.byRoot.clear();
  }
}

/** Start a recursive search under `root`. `onUpdate` fires per batch with the
    accumulated hit list, and a final time with `done: true`. */
export async function startSftpSearch(
  connectionId: string,
  root: string,
  query: string,
  onUpdate: (u: SftpSearchUpdate) => void,
): Promise<RunningSearch> {
  const id = crypto.randomUUID();
  const { listen } = await import("@tauri-apps/api/event");
  const hits: SftpEntry[] = [];
  let finished = false;
  const unlisten = await listen<Batch>(
    "portbay://sftp-search",
    (ev) => {
      const p = ev.payload;
      if (p.id !== id || finished) return;
      hits.push(...p.hits);
      onUpdate({ hits, scanned: p.scanned, done: p.done, truncated: p.truncated });
      if (p.done) {
        finished = true;
        unlisten();
      }
    },
    // The backend emits search batches point-to-point to the main window
    // (remote paths); a targeted emit skips untargeted listeners.
    { target: "main" },
  );
  invokeQuiet<void>("sftp_search", { input: { connectionId, id, root, query } }).catch(() => {
    // Transport/session failure — close out the search so the UI stops spinning.
    if (!finished) {
      finished = true;
      unlisten();
      onUpdate({ hits, scanned: 0, done: true, truncated: false });
    }
  });
  return {
    id,
    cancel: () => {
      if (!finished) void invokeQuiet("sftp_search_cancel", { id }).catch(() => {});
    },
  };
}
