/**
 * DB-write approval store — single source of truth for pending agent-issued
 * database writes that require human sign-off before execution.
 *
 * Polls `list_pending_db_writes` every 1 500 ms so any in-app route sees the
 * queue in near-real-time. `approve` / `deny` record the verdict and
 * immediately re-fetch, keeping the modal in sync.
 */
import { browser } from "$app/environment";

import { safeInvoke } from "$lib/ipc";
import type { PendingDbWrite } from "$lib/types/databases";

const POLL_INTERVAL_MS = 1_500;

function createDbApprovalsStore() {
  let pending = $state<PendingDbWrite[]>([]);
  let timer: ReturnType<typeof setInterval> | null = null;

  async function refresh(): Promise<void> {
    if (!browser) return;
    try {
      pending = await safeInvoke<PendingDbWrite[]>("list_pending_db_writes");
    } catch {
      // safeInvoke already pushed a toast; leave the last-known list in
      // place so the modal doesn't flicker on transient errors.
    }
  }

  function start() {
    if (!browser || timer !== null) return;
    void refresh();
    timer = setInterval(() => void refresh(), POLL_INTERVAL_MS);
  }

  function stop() {
    if (timer !== null) {
      clearInterval(timer);
      timer = null;
    }
  }

  async function approve(id: string): Promise<void> {
    await safeInvoke("resolve_db_write", { id, approved: true, reason: null });
    await refresh();
  }

  async function deny(id: string, reason?: string): Promise<void> {
    await safeInvoke("resolve_db_write", {
      id,
      approved: false,
      reason: reason ?? null,
    });
    await refresh();
  }

  return {
    get pending() {
      return pending;
    },
    refresh,
    start,
    stop,
    approve,
    deny,
  };
}

export const dbApprovals = createDbApprovalsStore();
