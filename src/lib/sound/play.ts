import { browser } from "$app/environment";

import {
  isNotificationSuppressed,
  NOTIFICATION_CUES,
  passesSeverityFloor,
  type AgentSoundEvent,
  type NotificationCue,
} from "$lib/notifications/prefs";
import { notificationPrefs } from "$lib/stores/notificationPrefs.svelte";
import type { Severity } from "$lib/types/error";

const COALESCE_MS = 2_500;
const lastPlayed = new Map<NotificationCue, number>();

export function previewCue(cue: NotificationCue): void {
  playCue(cue, true);
}

// Agent-board sounds are gated solely by their own per-event toggle (plus the
// global severity floor and quiet hours / manual pause). They deliberately
// bypass the category's `sound` channel so the three toggles are the single
// source of truth — no hidden master switch.
//
// `cardOverride` is the per-card "Notify on agent activity" automation: when a
// card opts in, its activity plays even if Settings has this event muted. The
// override only lifts the per-event toggle — quiet hours and the severity floor
// (the "pause / don't-disturb" controls) still apply.
export function playAgentEventCue(
  event: AgentSoundEvent,
  severity: Severity,
  cardOverride = false,
): void {
  const prefs = notificationPrefs.value;
  const setting = prefs.sound.agentEvents[event];
  if (!setting) return;
  if (!setting.enabled && !cardOverride) return;
  const now = new Date();
  if (!passesSeverityFloor(severity, prefs.severityFloor)) return;
  if (isNotificationSuppressed(prefs, severity, now)) return;
  playCue(setting.cue, false, now.getTime());
}

function playCue(cue: NotificationCue, force: boolean, now = Date.now()): void {
  if (!browser) return;
  if (!force && now - (lastPlayed.get(cue) ?? 0) < COALESCE_MS) return;
  const file = NOTIFICATION_CUES.find((c) => c.id === cue)?.file;
  if (!file) return;
  lastPlayed.set(cue, now);
  const audio = new Audio(file);
  audio.preload = "auto";
  void audio.play().catch(() => {});
}
