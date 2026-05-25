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

export interface Preferences {
  /** Install the menu-bar tray icon at launch. */
  showTrayIcon: boolean;
  /** When true, closing the window hides instead of quits the app. */
  closeToMenuBar: boolean;
  /** Internal: the one-time "still running" toast has been shown. */
  closeToMenuBarToastSeen: boolean;
  /** Explicit opt-in for usage telemetry and crash-report upload. */
  telemetryEnabled: boolean;
  /** Opt into early-access (experimental) features. Pro-gated in Settings. */
  earlyAccessOptIn: boolean;

  // General
  launchAtLogin: boolean;
  reopenPreviousProjects: boolean;
  confirmBeforeStopAll: boolean;
  desktopNotifications: boolean;

  // Appearance
  accentColor: AccentColor;

  // Workspace
  defaultWorkspaceFolder: string;
  autoDetectProjects: boolean;
  defaultSort: DefaultSort;
  defaultStartBehavior: StartBehavior;
  /** Web server pre-selected for new PHP projects. null → Caddy. */
  defaultWebServer: WebServer | null;

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
  closeToMenuBar: true,
  closeToMenuBarToastSeen: false,
  telemetryEnabled: false,
  earlyAccessOptIn: false,
  launchAtLogin: false,
  reopenPreviousProjects: false,
  confirmBeforeStopAll: true,
  desktopNotifications: false,
  accentColor: "blue",
  defaultWorkspaceFolder: "",
  autoDetectProjects: false,
  defaultSort: "name-asc",
  defaultStartBehavior: "manual",
  defaultWebServer: null,
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

function createPreferencesStore() {
  let value = $state<Preferences>({ ...DEFAULTS });
  let loaded = $state<boolean>(false);

  async function load(): Promise<void> {
    if (!browser) return;
    try {
      value = await safeInvoke<Preferences>("get_preferences");
      applyAccent(value.accentColor);
    } catch {
      // safeInvoke already showed the toast; keep defaults so the UI
      // stays interactive rather than blocked behind an opaque error.
    } finally {
      loaded = true;
    }
  }

  async function update(patch: Partial<Preferences>): Promise<void> {
    const next: Preferences = { ...value, ...patch };
    try {
      // The backend returns the persisted snapshot; trust it over the
      // optimistic patch in case server-side normalisation kicks in.
      value = await safeInvoke<Preferences>("set_preferences", { prefs: next });
      applyAccent(value.accentColor);
    } catch {
      // safeInvoke already showed the toast; leave `value` untouched
      // so the UI rolls back automatically.
    }
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
