/**
 * Notifications history — mirrors every envelope pushed onto the error
 * bus into a rolling 50-entry log so the topbar bell can show "what
 * happened recently?" without depending on the user catching toasts.
 *
 * The toast bus (`errorBus`) is fire-and-forget; this store is the
 * append-only history layer. Unread count drives the bell badge; the
 * NotificationsPanel pops it and lists entries newest-first.
 */
import { browser } from "$app/environment";

import type { CommandError } from "$lib/types/error";

const MAX_HISTORY = 50;
/** History survives app restarts (the agent-activity half of the bell is
 *  backend-persisted; this keeps the system half in step). */
const STORAGE_KEY = "portbay.notifications.history";

export interface NotificationEntry {
  id: string;
  envelope: CommandError;
  /** Unix millis. */
  receivedAt: number;
  read: boolean;
}

function loadPersisted(): NotificationEntry[] {
  if (!browser) return [];
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    const parsed = raw ? (JSON.parse(raw) as NotificationEntry[]) : [];
    return Array.isArray(parsed) ? parsed.slice(0, MAX_HISTORY) : [];
  } catch {
    return [];
  }
}

function createNotificationsStore() {
  let entries = $state<NotificationEntry[]>(loadPersisted());

  function persist(): void {
    if (!browser) return;
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(entries));
    } catch {
      /* quota / private mode — history degrades to session-only */
    }
  }

  function push(envelope: CommandError): void {
    const entry: NotificationEntry = {
      id: crypto.randomUUID(),
      envelope,
      receivedAt: Date.now(),
      read: false,
    };
    // Newest-first; trim once we exceed the cap so we never grow the
    // array unboundedly during a long session.
    const next = [entry, ...entries];
    entries =
      next.length > MAX_HISTORY ? next.slice(0, MAX_HISTORY) : next;
    persist();
  }

  function markAllRead(): void {
    entries = entries.map((e) => (e.read ? e : { ...e, read: true }));
    persist();
  }

  function clear(): void {
    entries = [];
    persist();
  }

  return {
    get value() {
      return entries;
    },
    get unreadCount() {
      return entries.reduce((n, e) => n + (e.read ? 0 : 1), 0);
    },
    push,
    markAllRead,
    clear,
  };
}

export const notifications = createNotificationsStore();
