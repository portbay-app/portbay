/**
 * PHP installs store — list of detected versions on the user's machine.
 *
 * Refreshed manually; the set rarely changes during a session (Homebrew
 * installs happen out-of-band). Consumed by:
 *   - /php route
 *   - ProjectDetailPanel's AdvancedFields (PHP version picker)
 *   - ProjectRow's Xdebug toggle (to know if xdebug is actually loaded)
 */
import { safeInvoke } from "$lib/ipc";
import type { PhpInstall } from "$lib/types/php";

function createPhpStore() {
  let items = $state<PhpInstall[]>([]);
  let loading = $state<boolean>(false);
  let lastRefreshedAt = $state<number | null>(null);

  async function refresh(): Promise<void> {
    loading = true;
    try {
      items = await safeInvoke<PhpInstall[]>("list_php_installs");
      lastRefreshedAt = Date.now();
    } finally {
      loading = false;
    }
  }

  function get(version: string): PhpInstall | null {
    return items.find((i) => i.version === version) ?? null;
  }

  function isInstalled(version: string): boolean {
    return items.some((i) => i.version === version);
  }

  return {
    get value() {
      return items;
    },
    get loading() {
      return loading;
    },
    get lastRefreshedAt() {
      return lastRefreshedAt;
    },
    refresh,
    get,
    isInstalled,
  };
}

export const php = createPhpStore();
