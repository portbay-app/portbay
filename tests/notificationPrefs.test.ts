import { describe, expect, it } from "vitest";

import {
  DEFAULT_NOTIFICATION_PREFS,
  isNotificationSuppressed,
  normaliseNotificationPrefs,
  shouldDeliver,
} from "../src/lib/notifications/prefs";

describe("notification preference routing", () => {
  it("backfills missing category defaults", () => {
    const prefs = normaliseNotificationPrefs({
      channels: {
        updates: { toast: false, bell: false, banner: false, sound: false },
      },
    } as Partial<typeof DEFAULT_NOTIFICATION_PREFS>);

    expect(prefs.channels.updates.bell).toBe(false);
    expect(prefs.channels["project-error"].toast).toBe(true);
    expect(prefs.channels["agent-board"].sound).toBe(true);
  });

  it("applies severity floor before channel delivery", () => {
    const prefs = normaliseNotificationPrefs({
      severityFloor: "errors_only",
    });

    expect(shouldDeliver(prefs, "project-error", "error", "toast")).toBe(true);
    expect(shouldDeliver(prefs, "project-error", "warning", "toast")).toBe(false);
  });

  it("suppresses across midnight and exempts errors", () => {
    const prefs = normaliseNotificationPrefs({
      quietHours: {
        enabled: true,
        start: "22:00",
        end: "07:00",
        exemptErrors: true,
      },
    });
    const twoAm = new Date(2026, 0, 1, 2, 0);

    expect(isNotificationSuppressed(prefs, "warning", twoAm)).toBe(true);
    expect(isNotificationSuppressed(prefs, "error", twoAm)).toBe(false);
  });

  it("snooze expires exactly at the stored timestamp", () => {
    const prefs = normaliseNotificationPrefs({
      snoozeUntil: 2_000,
    });

    expect(isNotificationSuppressed(prefs, "info", new Date(1_999))).toBe(true);
    expect(isNotificationSuppressed(prefs, "info", new Date(2_000))).toBe(false);
  });
});
