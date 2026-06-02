/**
 * sessionSecrets — an in-memory, idle-expiring cache of the one-shot secrets the
 * user typed at the credential prompt, so authenticating a host *once* covers
 * every subsystem opened during that working session (a shell, the SFTP file
 * tree, exec for Logs/Processes/Deploy each authenticate separately on the
 * backend). Without this, the first open of each subsystem would re-prompt.
 *
 * Deliberately **memory-only**: nothing is written to disk or the OS keychain —
 * persisting credentials stays a separate, explicit opt-in (see the credential
 * prompt rules). Entries expire after `TTL_MS` of inactivity, matching the
 * backend's idle-session reaper, so leaving a host idle eventually requires
 * re-authenticating; and they're cleared outright when the user disconnects.
 */
import type { SshCredentialKind } from "$lib/stores/credentialPrompt.svelte";

interface CachedSecrets {
  password?: string;
  passphrase?: string;
  /** Last time this entry was written or read — drives idle expiry. */
  at: number;
}

/** Match the backend exec/SFTP idle reaper (15 minutes). */
const TTL_MS = 15 * 60 * 1000;

const cache = new Map<string, CachedSecrets>();

function live(connectionId: string): CachedSecrets | undefined {
  const e = cache.get(connectionId);
  if (!e) return undefined;
  if (Date.now() - e.at > TTL_MS) {
    cache.delete(connectionId);
    return undefined;
  }
  return e;
}

/** Remember a secret the user just entered for this host (one kind at a time). */
export function rememberSecret(
  connectionId: string,
  kind: SshCredentialKind,
  secret: string,
): void {
  const cur = cache.get(connectionId) ?? { at: 0 };
  cache.set(connectionId, { ...cur, [kind]: secret, at: Date.now() });
}

/** Recall a previously-entered secret of `kind`, or `undefined` if none/expired. */
export function recallSecret(
  connectionId: string,
  kind: SshCredentialKind,
): string | undefined {
  const e = live(connectionId);
  if (!e) return undefined;
  e.at = Date.now(); // any auth use counts as activity
  return e[kind];
}

/** Forget a host's cached secrets (on disconnect, or an auth that finally failed). */
export function clearSecret(connectionId: string): void {
  cache.delete(connectionId);
}
