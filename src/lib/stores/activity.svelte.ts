/**
 * Agent-activity notifications — the "what did my agents do while I wasn't
 * looking?" feed behind the topbar bell.
 *
 * A backend scanner (`src-tauri/src/notifications.rs`) watches every project's
 * audit log and emits a `portbay://notifications` event per new notable agent
 * action (comment, blocked, warning). This store hydrates the persisted history
 * on launch via `notifications_list`, then keeps it live off that channel — so
 * the bell shows activity even when the terminal is closed or the user is on a
 * different project. Read/clear state round-trips to disk through the backend.
 */
import { browser } from "$app/environment";
import { goto } from "$app/navigation";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { safeInvoke } from "$lib/ipc";
import { shouldDeliver, type AgentSoundEvent } from "$lib/notifications/prefs";
import { playAgentEventCue } from "$lib/sound/play";
import { notificationPrefs } from "$lib/stores/notificationPrefs.svelte";

const CHANNEL = "portbay://notifications";
/** Mirror of the tasks page's board-picker memory, so click-through lands on
 *  the right board even on a cold `/tasks` mount. */
const LAST_BOARD_KEY = "portbay.board.lastProject";
const MAX = 200;

export type ActivityKind = "comment" | "blocked" | "warning" | "done" | "learning";

export interface ActivityNotification {
  id: string;
  projectId: string;
  projectName: string;
  cardId: string;
  cardTitle: string;
  /** Stable id of the originating audit entry. For a comment this is the
   *  comment's id, used to deep-link/scroll to it in the card thread. */
  entryId?: string | null;
  kind: ActivityKind;
  agent?: string | null;
  body?: string | null;
  /** ISO-8601 of the originating audit entry. */
  at: string;
  /** Unix millis PortBay recorded it (drives ordering + relative time). */
  createdMs: number;
  read: boolean;
  /** The card's "Notify on agent activity" automation was on when this fired.
   *  Forces the event's sound even if Settings has it muted. */
  subscribed?: boolean;
}

function createActivityStore() {
  let items = $state<ActivityNotification[]>([]);
  let started = false;
  let unlisten: UnlistenFn | null = null;

  /** Hydrate from disk and start listening. Idempotent; safe to call from any
   *  component that mounts (topbar bell does). */
  async function start(): Promise<void> {
    if (started || !browser) return;
    started = true;
    items = await safeInvoke<ActivityNotification[]>("notifications_list").catch(() => []);
    unlisten = await listen<ActivityNotification>(CHANNEL, (e) => {
      const n = e.payload;
      if (items.some((x) => x.id === n.id)) return; // de-dupe replays
      playActivitySound(n);
      const next = [n, ...items];
      items = next.length > MAX ? next.slice(0, MAX) : next;
    });
  }

  function stop(): void {
    unlisten?.();
    unlisten = null;
    started = false;
  }

  async function markAllRead(): Promise<void> {
    if (items.every((n) => n.read)) return;
    items = items.map((n) => (n.read ? n : { ...n, read: true }));
    await safeInvoke("notifications_mark_all_read").catch(() => {});
  }

  async function markRead(id: string): Promise<void> {
    if (!items.some((n) => n.id === id && !n.read)) return;
    items = items.map((n) => (n.id === id ? { ...n, read: true } : n));
    await safeInvoke("notifications_mark_read", { id }).catch(() => {});
  }

  async function clear(): Promise<void> {
    if (items.length === 0) return;
    items = [];
    await safeInvoke("notifications_clear").catch(() => {});
  }

  /** Click-through: land on the board the notification is about. Seeds the
   *  board picker's last-project memory (cold mount) and navigates. Agent
   *  activity only exists in builds with the task board compiled in, so on
   *  OSS builds this is never reached. */
  async function open(n: ActivityNotification): Promise<void> {
    void markRead(n.id);
    try {
      localStorage.setItem(LAST_BOARD_KEY, n.projectId);
    } catch {
      /* private mode / quota — navigation still works */
    }
    await goto("/tasks");
  }

  return {
    get value() {
      return items.filter(visibleInBell);
    },
    get unreadCount() {
      return items.filter(visibleInBell).reduce((c, n) => c + (n.read ? 0 : 1), 0);
    },
    start,
    stop,
    markAllRead,
    markRead,
    clear,
    open,
  };
}

export const activity = createActivityStore();

function visibleInBell(n: ActivityNotification): boolean {
  return shouldDeliver(
    notificationPrefs.value,
    "agent-board",
    severityForKind(n.kind),
    "bell",
    new Date(n.createdMs),
  );
}

function playActivitySound(n: ActivityNotification): void {
  // No audible nudge for activity you're already watching: if the app is the
  // focused window AND you're on that project's board, you can see the change
  // land — a sound would just be noise. The item still goes to the bell. (When
  // the app is unfocused or you're elsewhere in the app, the sound plays.)
  if (isActivelyViewing(n.projectId)) return;
  // Each agent event has its own sound toggle + cue (still subject to quiet
  // hours, severity floor, and manual pause). A muted event plays nothing —
  // unless the card opted into "Notify on agent activity", which overrides the
  // mute for that card so the user always hears its activity.
  playAgentEventCue(agentSoundEventForKind(n.kind), severityForKind(n.kind), n.subscribed === true);
}

/** True when the user is plainly already looking at `projectId`'s board: the app
 *  window is focused and the user is on the board route. Used to silence
 *  redundant activity sounds for the board you're staring at. (The OSS build
 *  has no board route, so this is always false there.) */
function isActivelyViewing(projectId: string): boolean {
  void projectId;
  if (!browser) return false;
  if (typeof document !== "undefined" && !document.hasFocus()) return false;
  return window.location?.pathname?.startsWith("/tasks") ?? false;
}

function agentSoundEventForKind(kind: ActivityKind): AgentSoundEvent {
  if (kind === "done") return "done";
  if (kind === "comment") return "comment";
  if (kind === "learning") return "learning";
  // blocked | warning — an agent ran into trouble while executing.
  return "error";
}

function severityForKind(kind: ActivityKind): "success" | "info" | "warning" | "error" {
  if (kind === "done") return "success";
  if (kind === "blocked") return "error";
  if (kind === "warning") return "warning";
  // comment | learning — informational.
  return "info";
}
