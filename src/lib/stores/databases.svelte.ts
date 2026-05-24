/**
 * Database instances + engine catalogue store.
 *
 * Mirrors the projects store pattern: `$state` for the instance list,
 * engines, loading, and selection; getters for readonly access; methods
 * that wrap `safeInvoke` and refresh in place. The Add-Database wizard's
 * open/close lives here too so the page CTA and the sidebar button share
 * one instance.
 */
import { browser } from "$app/environment";

import { safeInvoke } from "$lib/ipc";
import type {
  DatabaseEngineView,
  DatabaseInstanceView,
} from "$lib/types/databases";

function createDatabasesStore() {
  let instances = $state<DatabaseInstanceView[]>([]);
  let engines = $state<DatabaseEngineView[]>([]);
  let loading = $state<boolean>(false);
  let selectedId = $state<string | null>(null);
  let wizardOpen = $state<boolean>(false);

  /** Per-instance busy markers keyed `${id}:${action}`. */
  let busy = $state<Record<string, boolean>>({});

  function busyKey(id: string, action: string) {
    return `${id}:${action}`;
  }

  async function refresh(): Promise<void> {
    if (!browser) return;
    loading = true;
    try {
      const [list, engineList] = await Promise.all([
        safeInvoke<DatabaseInstanceView[]>("list_database_instances"),
        safeInvoke<DatabaseEngineView[]>("list_database_engines"),
      ]);
      instances = list;
      engines = engineList;
      // Keep selection valid; clear if the instance vanished.
      if (selectedId !== null && !instances.some((d) => d.id === selectedId)) {
        selectedId = instances[0]?.id ?? null;
      } else if (selectedId === null && instances.length > 0) {
        selectedId = instances[0].id;
      }
    } catch {
      // safeInvoke pushed the toast.
    } finally {
      loading = false;
    }
  }

  /** Reload only the engine catalogue (after an install). */
  async function refreshEngines(): Promise<void> {
    if (!browser) return;
    try {
      engines = await safeInvoke<DatabaseEngineView[]>("list_database_engines");
    } catch {
      /* toast already pushed */
    }
  }

  function select(id: string | null) {
    selectedId = id;
  }

  function isBusy(id: string, action: string): boolean {
    return busy[busyKey(id, action)] === true;
  }

  function anyBusy(id: string): boolean {
    return Object.entries(busy).some(([k, v]) => v && k.startsWith(`${id}:`));
  }

  function setBusy(id: string, action: string, v: boolean) {
    busy = { ...busy, [busyKey(id, action)]: v };
  }

  /** Run an instance lifecycle action, wrapping busy state + refresh. */
  async function action(
    id: string,
    name: "start" | "stop" | "restart",
  ): Promise<void> {
    if (isBusy(id, name)) return;
    setBusy(id, name, true);
    try {
      // Explicit per-action commands rather than a built command name, so
      // these stay greppable and survive a backend rename under typecheck.
      switch (name) {
        case "start":
          await safeInvoke("start_database_instance", { id });
          break;
        case "stop":
          await safeInvoke("stop_database_instance", { id });
          break;
        case "restart":
          await safeInvoke("restart_database_instance", { id });
          break;
      }
      await refresh();
    } catch {
      /* toast already pushed */
    } finally {
      setBusy(id, name, false);
    }
  }

  return {
    get value() {
      return instances;
    },
    get engines() {
      return engines;
    },
    get loading() {
      return loading;
    },
    get selectedId() {
      return selectedId;
    },
    get wizardOpen() {
      return wizardOpen;
    },
    get selected(): DatabaseInstanceView | null {
      return instances.find((d) => d.id === selectedId) ?? null;
    },
    isBusy,
    anyBusy,
    setBusy,
    refresh,
    refreshEngines,
    select,
    action,
    showWizard() {
      wizardOpen = true;
    },
    hideWizard() {
      wizardOpen = false;
    },
  };
}

export const databases = createDatabasesStore();
