/**
 * Installed developer-tool cache. The Rust side returns supported editor,
 * agent, and terminal launch targets in preferred order.
 */
import { browser } from "$app/environment";

import { safeInvoke } from "$lib/ipc";
import type { DevToolInfo } from "$lib/types/devTools";

function createDevToolStore() {
  let items = $state<DevToolInfo[]>([]);
  let loading = $state<boolean>(false);
  let loaded = $state<boolean>(false);

  async function refresh(): Promise<void> {
    if (!browser) return;
    loading = true;
    try {
      items = await safeInvoke<DevToolInfo[]>("installed_dev_tools");
      loaded = true;
    } catch {
      // safeInvoke already pushed the toast.
    } finally {
      loading = false;
    }
  }

  async function start(): Promise<void> {
    if (loaded || loading) return;
    await refresh();
  }

  return {
    get value() {
      return items;
    },
    get loading() {
      return loading;
    },
    get defaultTool() {
      return items[0] ?? null;
    },
    refresh,
    start,
  };
}

export const devTools = createDevToolStore();
