import type { Severity } from "$lib/types/error";

export type NotificationCategory =
  | "lifecycle"
  | "project-error"
  | "agent-board"
  | "updates"
  | "crash"
  | "infrastructure"
  | "account-sync";

export type NotificationChannel = "toast" | "bell" | "banner" | "sound";
export type NotificationSeverityFloor = "errors_only" | "errors_and_warnings" | "everything";
export type NotificationCue = "done" | "comment" | "attention" | "error";

// The three distinct agent-board events that can each carry their own sound.
// Splitting these out of the single "agent-board" sound toggle lets a user
// hear, say, completed cards but stay silent for comments.
export type AgentSoundEvent = "done" | "error" | "comment" | "learning";

export interface AgentSoundSetting {
  enabled: boolean;
  cue: NotificationCue;
}

export interface NotificationChannelPrefs {
  toast: boolean;
  bell: boolean;
  banner: boolean;
  sound: boolean;
}

export interface NotificationQuietHours {
  enabled: boolean;
  start: string;
  end: string;
  exemptErrors: boolean;
}

export interface NotificationSoundPrefs {
  volumeFollowsOs: boolean;
  cuePerCategory: Record<NotificationCategory, NotificationCue>;
  /**
   * Per-agent-event sound. Each agent-board event — a completed card, an
   * execution error, a new comment — carries its own on/off + cue so they're
   * tunable independently instead of collapsed into one toggle. These gate
   * playback for the `agent-board` category in place of a single sound switch.
   */
  agentEvents: Record<AgentSoundEvent, AgentSoundSetting>;
}

export interface NotificationPrefs {
  schemaVersion: number;
  channels: Record<NotificationCategory, NotificationChannelPrefs>;
  severityFloor: NotificationSeverityFloor;
  quietHours: NotificationQuietHours;
  snoozeUntil: number | null;
  sound: NotificationSoundPrefs;
}

// `sound` marks the categories that may carry an audible cue. Sound is
// reserved for agent/task activity (a completed card, a comment, an agent
// error) — the one place an audible nudge earns its interruption. Every
// other category is bell/toast only. Account & sync is intentionally absent:
// those events (sign-in, license expiry, sync status) still post to the bell
// via DEFAULT_NOTIFICATION_PREFS, they just aren't user-tunable.
export const NOTIFICATION_CATEGORIES: {
  id: NotificationCategory;
  label: string;
  description: string;
  sound?: boolean;
}[] = [
  {
    id: "lifecycle",
    label: "Project lifecycle",
    description: "Start, stop, and restart confirmations.",
  },
  {
    id: "project-error",
    label: "Project errors",
    description: "Start failures, port conflicts, crashes, and sidecar failures.",
  },
  {
    id: "agent-board",
    label: "Agent board activity",
    description: "Agent comments, blocked cards, dispatches, and completed cards.",
    sound: true,
  },
  {
    id: "updates",
    label: "Updates",
    description: "Version checks, completed installs, and restart prompts.",
  },
  {
    id: "crash",
    label: "Crash & diagnostics",
    description: "Crash-report prompts and diagnostic upload outcomes.",
  },
  {
    id: "infrastructure",
    label: "Infrastructure",
    description: "Certificates, DNS, tunnels, databases, and local services.",
  },
];

// Categories that expose an audible cue in Settings → Notifications → Sound.
export const SOUND_CATEGORIES = NOTIFICATION_CATEGORIES.filter((category) => category.sound);

export const NOTIFICATION_CHANNELS: {
  id: NotificationChannel;
  label: string;
  shortLabel: string;
}[] = [
  { id: "toast", label: "In-app toast", shortLabel: "Toast" },
  { id: "bell", label: "Bell history", shortLabel: "Bell" },
  { id: "banner", label: "Desktop banner", shortLabel: "Banner" },
  { id: "sound", label: "Sound", shortLabel: "Sound" },
];

export const NOTIFICATION_CUES: { id: NotificationCue; label: string; file: string }[] = [
  { id: "done", label: "Done chime", file: "/sounds/done.wav" },
  { id: "comment", label: "Comment ping", file: "/sounds/comment.wav" },
  { id: "attention", label: "Attention tone", file: "/sounds/attention.wav" },
  { id: "error", label: "Error tone", file: "/sounds/error.wav" },
];

// The agent-board sound events surfaced in Settings → Notifications → Sound,
// each with its own enable toggle + cue.
export const AGENT_SOUND_EVENTS: {
  id: AgentSoundEvent;
  label: string;
  description: string;
}[] = [
  { id: "done", label: "Card completed", description: "An agent moved a card to Done." },
  {
    id: "error",
    label: "Agent execution error",
    description: "An agent hit an error or got blocked while running.",
  },
  { id: "comment", label: "Agent comment", description: "An agent left a comment on a card." },
  {
    id: "learning",
    label: "Agent learning",
    description: "An agent recorded a project learning (a rule for next time).",
  },
];

export const DEFAULT_NOTIFICATION_PREFS: NotificationPrefs = {
  schemaVersion: 1,
  channels: {
    // `sound` is only ever true for agent-board — task activity is the sole
    // category surfaced in the Sound section. The rest are silent by design.
    lifecycle: { toast: false, bell: true, banner: false, sound: false },
    "project-error": { toast: true, bell: true, banner: false, sound: false },
    "agent-board": { toast: false, bell: true, banner: false, sound: true },
    updates: { toast: false, bell: true, banner: false, sound: false },
    crash: { toast: true, bell: true, banner: false, sound: false },
    infrastructure: { toast: false, bell: true, banner: false, sound: false },
    "account-sync": { toast: false, bell: true, banner: false, sound: false },
  },
  severityFloor: "everything",
  quietHours: {
    enabled: false,
    start: "22:00",
    end: "07:00",
    exemptErrors: true,
  },
  snoozeUntil: null,
  sound: {
    volumeFollowsOs: true,
    cuePerCategory: {
      lifecycle: "done",
      "project-error": "error",
      "agent-board": "comment",
      updates: "done",
      crash: "error",
      infrastructure: "attention",
      "account-sync": "comment",
    },
    agentEvents: {
      done: { enabled: true, cue: "done" },
      error: { enabled: true, cue: "error" },
      comment: { enabled: true, cue: "comment" },
      // A recorded learning is informational — silent by default; it still
      // lands in the bell. Toggle on like the others to hear it.
      learning: { enabled: false, cue: "attention" },
    },
  },
};

export function normaliseNotificationPrefs(input: Partial<NotificationPrefs>): NotificationPrefs {
  const base = structuredClone(DEFAULT_NOTIFICATION_PREFS);
  const channels = { ...base.channels, ...(input.channels ?? {}) };
  const cues = { ...base.sound.cuePerCategory, ...(input.sound?.cuePerCategory ?? {}) };
  const inAgent = input.sound?.agentEvents;
  const agentEvents = {
    done: { ...base.sound.agentEvents.done, ...(inAgent?.done ?? {}) },
    error: { ...base.sound.agentEvents.error, ...(inAgent?.error ?? {}) },
    comment: { ...base.sound.agentEvents.comment, ...(inAgent?.comment ?? {}) },
    learning: { ...base.sound.agentEvents.learning, ...(inAgent?.learning ?? {}) },
  };
  return {
    ...base,
    ...input,
    channels,
    quietHours: { ...base.quietHours, ...(input.quietHours ?? {}) },
    sound: {
      ...base.sound,
      ...(input.sound ?? {}),
      cuePerCategory: cues,
      agentEvents,
    },
  };
}

export function severityForEnvelope(e: { severity?: Severity; whoCausedIt: string }): Severity {
  return e.severity ?? (e.whoCausedIt === "user" ? "warning" : "error");
}

export function shouldDeliver(
  prefs: NotificationPrefs,
  category: NotificationCategory,
  severity: Severity,
  channel: NotificationChannel,
  now = new Date(),
): boolean {
  return (
    Boolean(prefs.channels[category]?.[channel]) &&
    passesSeverityFloor(severity, prefs.severityFloor) &&
    !isNotificationSuppressed(prefs, severity, now)
  );
}

export function isNotificationSuppressed(
  prefs: NotificationPrefs,
  severity: Severity,
  now = new Date(),
): boolean {
  if (prefs.quietHours.exemptErrors && severity === "error") return false;
  if (prefs.snoozeUntil && now.getTime() < prefs.snoozeUntil) return true;
  if (!prefs.quietHours.enabled) return false;
  return isInQuietWindow(prefs.quietHours, now);
}

export function passesSeverityFloor(severity: Severity, floor: NotificationSeverityFloor): boolean {
  if (floor === "everything") return true;
  if (floor === "errors_and_warnings") return severity === "error" || severity === "warning";
  return severity === "error";
}

function isInQuietWindow(quiet: NotificationQuietHours, now: Date): boolean {
  const start = parseTime(quiet.start);
  const end = parseTime(quiet.end);
  if (start === null || end === null) return false;
  if (start === end) return true;
  const minute = now.getHours() * 60 + now.getMinutes();
  return start < end ? minute >= start && minute < end : minute >= start || minute < end;
}

function parseTime(value: string): number | null {
  const [h, m] = value.split(":");
  const hour = Number(h);
  const minute = Number(m);
  if (!Number.isInteger(hour) || !Number.isInteger(minute)) return null;
  if (hour < 0 || hour > 23 || minute < 0 || minute > 59) return null;
  return hour * 60 + minute;
}
