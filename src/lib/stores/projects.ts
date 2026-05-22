/**
 * Projects store — the canonical list of registered projects with live
 * status. One load on mount, then patches in place on every
 * `portbay://status` event so the table doesn't re-fetch on each tick.
 *
 * Also tracks `selectedId` for keyboard navigation in the table.
 */
import { browser } from "$app/environment";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { safeInvoke } from "$lib/ipc";
import type { ProjectStatusEvent, ProjectView } from "$lib/types/projects";

const STATUS_CHANNEL = "portbay://status";

function createProjectsStore() {
  let items = $state<ProjectView[]>([]);
  let loading = $state<boolean>(false);
  let selectedId = $state<string | null>(null);
  let unlisten: UnlistenFn | null = null;

  async function refresh(): Promise<void> {
    if (!browser) return;
    loading = true;
    try {
      items = await safeInvoke<ProjectView[]>("list_projects");
      // Preserve selection if the project still exists; clear otherwise.
      if (selectedId !== null && !items.some((p) => p.id === selectedId)) {
        selectedId = items[0]?.id ?? null;
      } else if (selectedId === null && items.length > 0) {
        selectedId = items[0].id;
      }
    } catch {
      // safeInvoke already pushed the toast.
    } finally {
      loading = false;
    }
  }

  /** Patch a single row from a status event. Avoids re-fetching the list. */
  function applyStatusEvent(ev: ProjectStatusEvent) {
    items = items.map((p) =>
      p.id === ev.id
        ? { ...p, status: ev.status, runtime: ev.runtime ?? p.runtime }
        : p,
    );
  }

  async function start() {
    if (!browser) return;
    await refresh();
    if (unlisten) return;
    unlisten = await listen<ProjectStatusEvent>(STATUS_CHANNEL, (e) => {
      applyStatusEvent(e.payload);
    });
  }

  function stop() {
    if (unlisten) {
      unlisten();
      unlisten = null;
    }
  }

  function select(id: string | null) {
    selectedId = id;
  }

  function selectRelative(delta: 1 | -1) {
    if (items.length === 0) return;
    const idx = items.findIndex((p) => p.id === selectedId);
    const next =
      idx === -1
        ? 0
        : Math.max(0, Math.min(items.length - 1, idx + delta));
    selectedId = items[next].id;
  }

  return {
    get value() {
      return items;
    },
    get loading() {
      return loading;
    },
    get selectedId() {
      return selectedId;
    },
    refresh,
    start,
    stop,
    select,
    selectRelative,
  };
}

export const projects = createProjectsStore();
