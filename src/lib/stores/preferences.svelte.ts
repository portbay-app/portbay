/**
 * App preferences — behavioural toggles backed by the Rust
 * `preferences.json` file. Single source of truth: the Rust core. The
 * frontend mirrors a snapshot in memory so the Settings UI reads
 * synchronously after the initial `load()`.
 *
 * Pairs with `commands::preferences::{get_preferences, set_preferences,
 * mark_close_toast_seen}` on the Rust side.
 */
import { browser } from "$app/environment";

import { safeInvoke } from "$lib/ipc";
import {
  DEFAULT_NOTIFICATION_PREFS,
  normaliseNotificationPrefs,
  type NotificationPrefs,
} from "$lib/notifications/prefs";
import type { WebServer } from "$lib/types/projects";

export type AccentColor =
  | "blue"
  | "purple"
  | "green"
  | "orange"
  | "red"
  | "yellow"
  | "gray";

export type DefaultSort = "name-asc" | "name-desc" | "status" | "port";
export type StartBehavior = "manual" | "auto";
export type AutoCleanSchedule = "off" | "weekly" | "monthly";
export type AccessibilityTextScale = "normal" | "large" | "larger";
export type AccessibilityFocusMode = "standard" | "strong";

export interface AccessibilityPrefs {
  reduceMotion: boolean;
  reduceTransparency: boolean;
  highContrast: boolean;
  textScale: AccessibilityTextScale;
  focusMode: AccessibilityFocusMode;
  underlineLinks: boolean;
  colorIndependentStatus: boolean;
}

export interface Preferences {
  /** Install the menu-bar tray icon at launch. */
  showTrayIcon: boolean;
  /** Show PortBay's icon in the Dock. When false, the app runs as a
   * menu-bar-only accessory (no Dock tile). macOS-only. */
  showDockIcon: boolean;
  /** When true, closing the window hides instead of quits the app. */
  closeToMenuBar: boolean;
  /** Internal: the one-time "still running" toast has been shown. */
  closeToMenuBarToastSeen: boolean;
  /** Explicit opt-in for usage telemetry and crash-report upload. */
  telemetryEnabled: boolean;
  /** Internal: the one-time diagnostics consent prompt (shown after the
   * first `portbay login`) has been answered. Set by the CLI; the GUI only
   * carries it through so neither surface re-asks. */
  telemetryConsentPrompted: boolean;
  /** Opt into early-access (experimental) features. Pro-gated in Settings. */
  earlyAccessOptIn: boolean;

  // General
  launchAtLogin: boolean;
  reopenPreviousProjects: boolean;
  confirmBeforeStopAll: boolean;
  desktopNotifications: boolean;
  notifications: NotificationPrefs;
  accessibility: AccessibilityPrefs;

  // Appearance
  accentColor: AccentColor;

  // Workspace
  defaultWorkspaceFolder: string;
  autoDetectProjects: boolean;
  defaultSort: DefaultSort;
  defaultStartBehavior: StartBehavior;
  /** Web server pre-selected for new PHP projects. null → Caddy. */
  defaultWebServer: WebServer | null;
  /** Terminal that hosts interactive agent dispatches (id from `installed_dev_tools`,
   * e.g. "warp" | "iterm" | "ghostty" | "terminal"). null → first detected. */
  preferredTerminal: string | null;
  /** Global default dispatch agent (kind id) for boards without their own config. null → Claude. */
  preferredAgent: string | null;
  /** Per-agent absolute binary path overrides, keyed by agent id (external drive / custom prefix). */
  agentPaths: Record<string, string>;

  // Domains & HTTPS
  manageHostsAutomatically: boolean;
  autoRenewCertificates: boolean;

  // Advanced
  storeLogsLocally: boolean;
  logRetentionDays: number;
  cliPath: string;

  // Artifacts
  /** Background auto-clean cadence across every project. */
  autoCleanSchedule: AutoCleanSchedule;
  /** Unix seconds of the last completed pass; 0 = never. */
  lastAutoClean: number;
  /** Extra project-relative dir names to treat as artifacts. */
  autoCleanExtraDirs: string[];
}

const DEFAULTS: Preferences = {
  showTrayIcon: true,
  showDockIcon: true,
  closeToMenuBar: true,
  closeToMenuBarToastSeen: false,
  telemetryEnabled: false,
  telemetryConsentPrompted: false,
  earlyAccessOptIn: false,
  launchAtLogin: false,
  reopenPreviousProjects: false,
  confirmBeforeStopAll: true,
  desktopNotifications: false,
  notifications: normaliseNotificationPrefs(DEFAULT_NOTIFICATION_PREFS),
  accessibility: {
    reduceMotion: false,
    reduceTransparency: false,
    highContrast: false,
    textScale: "normal",
    focusMode: "standard",
    underlineLinks: false,
    colorIndependentStatus: false,
  },
  accentColor: "blue",
  defaultWorkspaceFolder: "",
  autoDetectProjects: false,
  defaultSort: "name-asc",
  defaultStartBehavior: "manual",
  defaultWebServer: null,
  preferredTerminal: null,
  preferredAgent: null,
  agentPaths: {},
  manageHostsAutomatically: true,
  autoRenewCertificates: true,
  storeLogsLocally: true,
  logRetentionDays: 7,
  cliPath: "/usr/local/bin/portbay",
  autoCleanSchedule: "off",
  lastAutoClean: 0,
  autoCleanExtraDirs: [],
};

// Per-accent CSS variable values. Keyed off `accentColor`, applied to
// :root so every consumer of `var(--color-accent)` swaps without a
// re-render. The hover value is one HSL step toward white.
const ACCENT_PRESETS: Record<AccentColor, { base: string; hover: string }> = {
  blue:   { base: "#4d9cff", hover: "#6bb0ff" },
  purple: { base: "#a855f7", hover: "#c084fc" },
  green:  { base: "#22c55e", hover: "#4ade80" },
  orange: { base: "#f97316", hover: "#fb923c" },
  red:    { base: "#ef4444", hover: "#f87171" },
  yellow: { base: "#eab308", hover: "#facc15" },
  gray:   { base: "#9ca3af", hover: "#d1d5db" },
};

function applyAccent(color: AccentColor): void {
  if (!browser) return;
  const preset = ACCENT_PRESETS[color] ?? ACCENT_PRESETS.blue;
  document.documentElement.style.setProperty("--color-accent", preset.base);
  document.documentElement.style.setProperty("--color-accent-hover", preset.hover);
}

function applyAccessibility(prefs: AccessibilityPrefs): void {
  if (!browser) return;
  document.body.setAttribute("data-a11y-motion", prefs.reduceMotion ? "reduced" : "standard");
  document.body.setAttribute(
    "data-a11y-transparency",
    prefs.reduceTransparency ? "reduced" : "standard",
  );
  document.body.setAttribute("data-a11y-contrast", prefs.highContrast ? "high" : "standard");
  document.body.setAttribute("data-a11y-text", prefs.textScale);
  document.body.setAttribute("data-a11y-focus", prefs.focusMode);
  document.body.setAttribute("data-a11y-links", prefs.underlineLinks ? "underlined" : "standard");
  document.body.setAttribute(
    "data-a11y-status",
    prefs.colorIndependentStatus ? "shapes" : "color",
  );
}

function createPreferencesStore() {
  let value = $state<Preferences>({ ...DEFAULTS });
  let loaded = $state<boolean>(false);
  let saveSeq = 0;
  let inFlightSaves = 0;
  let saveChain: Promise<void> = Promise.resolve();

  async function load(): Promise<void> {
    if (!browser) return;
    const seq = saveSeq;
    try {
      const loadedPrefs = normalisePreferences(await safeInvoke<Preferences>("get_preferences"));
      if (seq === saveSeq && inFlightSaves === 0) {
        value = loadedPrefs;
        applyAccent(value.accentColor);
        applyAccessibility(value.accessibility);
      }
    } catch {
      // safeInvoke already showed the toast; keep defaults so the UI
      // stays interactive rather than blocked behind an opaque error.
    } finally {
      loaded = true;
    }
  }

  async function update(patch: Partial<Preferences>): Promise<void> {
    const previous = value;
    const next: Preferences = { ...value, ...patch };
    const seq = ++saveSeq;
    value = normalisePreferences(next);
    applyAccent(value.accentColor);
    applyAccessibility(value.accessibility);
    const snapshot = clonePreferences(value);
    inFlightSaves += 1;

    const run = async () => {
      try {
        // The backend returns the persisted snapshot; trust it over the
        // optimistic patch in case server-side normalisation kicks in.
        const saved = normalisePreferences(
          await safeInvoke<Preferences>("set_preferences", { prefs: snapshot }),
        );
        if (seq === saveSeq) {
          value = saved;
          applyAccent(value.accentColor);
          applyAccessibility(value.accessibility);
        }
      } catch {
        // safeInvoke already showed the toast; roll back the optimistic patch.
        if (seq === saveSeq) {
          value = previous;
          applyAccent(value.accentColor);
          applyAccessibility(value.accessibility);
        }
      } finally {
        inFlightSaves = Math.max(0, inFlightSaves - 1);
      }
    };

    const queued = saveChain.then(run, run);
    saveChain = queued.catch(() => {});
    await queued;
  }

  async function markCloseToastSeen(): Promise<void> {
    try {
      await safeInvoke<void>("mark_close_toast_seen");
      value = { ...value, closeToMenuBarToastSeen: true };
    } catch {
      /* benign — the toast won't suppress itself, but the app keeps working */
    }
  }

  return {
    get value() {
      return value;
    },
    get loaded() {
      return loaded;
    },
    load,
    update,
    markCloseToastSeen,
  };
}

export const preferences = createPreferencesStore();

function normalisePreferences(prefs: Preferences): Preferences {
  return {
    ...prefs,
    notifications: normaliseNotificationPrefs(prefs.notifications ?? DEFAULT_NOTIFICATION_PREFS),
    accessibility: { ...DEFAULTS.accessibility, ...(prefs.accessibility ?? {}) },
  };
}

function clonePreferences(prefs: Preferences): Preferences {
  return normalisePreferences(JSON.parse(JSON.stringify(prefs)) as Preferences);
}
