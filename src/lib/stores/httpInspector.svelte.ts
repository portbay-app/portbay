/**
 * HTTP inspector store — subscribes to `portbay://request` events emitted by
 * the Rust tailer (Caddy JSON access log) and keeps a capped rolling buffer.
 * Backfills on open via `recent_requests`. Filtering lives in the page so the
 * controls bind to plain local state; this store just owns the data + stream.
 *
 * Mirrors the metrics store's lifecycle (start on mount, stop on unmount).
 */
import { browser } from "$app/environment";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { safeInvoke } from "$lib/ipc";
import type { RequestEntry } from "$lib/types/inspector";

const CHANNEL = "portbay://request";
/** Cap so heavy traffic never grows the buffer unbounded (DoD). */
const MAX_ENTRIES = 1000;
const BACKFILL = 200;

function createHttpInspectorStore() {
  let entries = $state<RequestEntry[]>([]);
  let unlisten: UnlistenFn | null = null;

  function push(e: RequestEntry) {
    const next = [...entries, e];
    entries =
      next.length > MAX_ENTRIES ? next.slice(next.length - MAX_ENTRIES) : next;
  }

  async function start() {
    if (!browser) return;
    if (unlisten) return;
    // Backfill from the log tail so the table isn't empty on open. Failures
    // fall through — the live event stream will fill in.
    try {
      entries = await safeInvoke<RequestEntry[]>("recent_requests", {
        limit: BACKFILL,
      });
    } catch {
      /* event stream will catch up */
    }
    unlisten = await listen<RequestEntry>(CHANNEL, (ev) => push(ev.payload));
  }

  function stop() {
    if (unlisten) {
      unlisten();
      unlisten = null;
    }
  }

  /** Truncate the on-disk access log and empty the buffer. */
  async function clear() {
    try {
      await safeInvoke("clear_requests");
    } catch {
      /* toast already shown */
    }
    entries = [];
  }

  return {
    get entries() {
      return entries;
    },
    start,
    stop,
    clear,
  };
}

export const httpInspector = createHttpInspectorStore();
