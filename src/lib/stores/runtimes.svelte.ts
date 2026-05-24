/**
 * Runtimes store — detected language installs (PHP, Node, Python, Go,
 * Ruby) backed by the unified `list_runtimes` IPC that replaced the
 * PHP-only `list_php_installs`.
 *
 * Refreshed manually; the set rarely changes during a session (installs
 * happen out-of-band via Homebrew / asdf / nvm). Consumed by:
 *   - AdvancedFields' PHP version picker ("is this version installed?")
 *
 * The full per-version config lives on the /languages route, which reads
 * `list_runtimes` directly; this store only caches the lightweight
 * "which versions exist" view callers elsewhere need.
 */
import { safeInvoke } from "$lib/ipc";
import type { LanguageView } from "$lib/types/runtimes";

function createRuntimesStore() {
  let languages = $state<LanguageView[]>([]);
  let loading = $state<boolean>(false);
  let lastRefreshedAt = $state<number | null>(null);

  async function refresh(): Promise<void> {
    loading = true;
    try {
      languages = await safeInvoke<LanguageView[]>("list_runtimes");
      lastRefreshedAt = Date.now();
    } finally {
      loading = false;
    }
  }

  /** Detected versions for a language id ("php", "node", …). */
  function installedVersions(lang: string): string[] {
    const l = languages.find((x) => x.id === lang);
    return l ? l.versions.map((v) => v.install.version) : [];
  }

  function isInstalled(lang: string, version: string): boolean {
    return installedVersions(lang).includes(version);
  }

  return {
    get value() {
      return languages;
    },
    get loading() {
      return loading;
    },
    get lastRefreshedAt() {
      return lastRefreshedAt;
    },
    refresh,
    installedVersions,
    isInstalled,
  };
}

export const runtimes = createRuntimesStore();
