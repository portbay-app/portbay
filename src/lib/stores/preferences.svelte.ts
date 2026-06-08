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
import type { AiPrefs } from "$lib/types/ai";

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

/** How aggressively dictated transcripts are post-processed. */
export type DictationRewriteMode = "off" | "light" | "smart";

/** Smart Dictation — the rewrite layer over macOS dictation. Smart by
 * default via the on-device Apple provider (latches off for the session on
 * machines without Apple Intelligence); the transcript only ever goes to the
 * configured (local) provider, never audio, never the network beyond
 * `endpoint`. */
export interface DictationPrefs {
  mode: DictationRewriteMode;
  /** Provider id — "apple" (on-device Foundation Models, macOS 26+, the
   * zero-setup default) or "ollama" (local server the user runs). */
  provider: string;
  /** Provider base URL (Ollama only; Apple is on-device). */
  endpoint: string;
  /** Model name; empty = auto-pick from installed models (Ollama only). */
  model: string;
  /** Push-to-talk: hold the Fn (🌐) key with a dictation field focused to
   * dictate; release to stop. Independent of the rewrite mode. */
  pushToTalk: boolean;
  /** User-curated dictation terms ("refactor", "Tailwind", "Shopify") merged
   * ahead of every automatic vocabulary source backend-side — the plain
   * words and niche brands dictation garbles that no harvest can supply.
   * The prompt takes the first 12. */
  customTerms: string[];
  /** Transcription engine: "macos" (system dictation types into the field,
   * the default) or "local" (the bundled sidecar captures the mic and runs
   * a downloaded Whisper/Parakeet model on-device). */
  sttEngine: string;
  /** Local STT model (catalog id, e.g. "parakeet-tdt-v3"). Only read when
   * sttEngine === "local"; empty = no model chosen yet. */
  sttModel: string;
  /** "Dictate anywhere": hold Fn in ANY app and the local engine's
   * transcript is pasted into it. Off by default — needs the Accessibility
   * grant and a local model. Only read when sttEngine === "local". */
  anywhere: boolean;
  /** Hands-free variant of "dictate anywhere": double-tap Fn starts a
   * session that stays live without holding the key; a single Fn tap (or
   * the notch's stop button, or Esc) finishes it. On by default within the
   * anywhere opt-in. */
  anywhereDoubleTap: boolean;
  /** Recording-overlay placement: "notch" (camera-housing HUD, default) or
   * "bottom" (floating pill near the bottom of the pointer's screen — for
   * Macs without a notch). */
  overlayPosition: string;
  /** Raw mic-RMS floor below which the overlay waveform stays flat —
   * raise it so the bars don't dance to a noisy room. Backend clamps to
   * 0–0.05; default 0.01 (the previously hardcoded floor). */
  overlayNoiseFloor: number;
  /** Live-transcript preview tail: the overlay keeps the last N characters
   * (head-truncated so the newest words stay visible). Backend clamps to
   * 50–800; default 150. */
  overlayPreviewChars: number;
  /** "Polish dictation everywhere": run the Smart Dictation rewrite engine
   * over the system-wide transcript before pasting, so rambly speech lands
   * clean in any app. Off by default; only read inside the anywhere opt-in.
   * A failed/timed-out rewrite degrades to the raw transcript. */
  anywherePolish: boolean;
  /** Per-app RewriteContext overrides for the polished anywhere path:
   * frontmost bundle id → context wire string (snake_case). Resolution falls
   * back to built-ins (terminals → terminal_command) then GeneralNote, so an
   * empty list still does the right thing. */
  anywhereAppContexts: AppContextRule[];
}

/** One per-app rewrite-context override for the polished anywhere path. */
export interface AppContextRule {
  bundleId: string;
  context: string;
}

/** Local speech-to-text model storage (the AI page's "Speech to text"
 * section manages it). Which engine transcribes lives on `dictation`. */
export interface SttPrefs {
  /** Where downloaded STT models live; one subdirectory per catalog id. */
  modelsDir: string;
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

  // AI / local Ollama
  ai: AiPrefs;

  // Dictation
  dictation: DictationPrefs;

  // Local speech-to-text storage
  stt: SttPrefs;
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
  ai: {
    endpoint: "http://127.0.0.1:11434",
    modelsDir: "",
    binaryPath: "",
    keepAlive: "5m",
    flashAttention: false,
    origins: "http://localhost,https://localhost,http://127.0.0.1,https://127.0.0.1",
    numParallel: null,
    debug: false,
    modelDownloadThreads: null,
    noHistory: false,
    noPrune: false,
    scheduleSpread: false,
    multiUserCache: false,
    kvCacheType: "",
    gpuOverhead: null,
    loadTimeout: "",
    maxLoadedModels: null,
    maxQueue: null,
    llmLibrary: "",
    httpProxy: "",
    httpsProxy: "",
    noProxy: "",
  },
  dictation: {
    // Smart by default: zero-setup on-device polish (Wispr-Flow-style). On
    // machines without Apple Intelligence the first rewrite resolves
    // `no_model` and the rewriter latches the provider off for the session —
    // dictation keeps working raw, with no nagging.
    mode: "smart",
    provider: "apple",
    endpoint: "http://127.0.0.1:11434",
    model: "",
    pushToTalk: true,
    customTerms: [],
    sttEngine: "macos",
    sttModel: "",
    anywhere: false,
    anywhereDoubleTap: true,
    overlayPosition: "notch",
    overlayNoiseFloor: 0.01,
    overlayPreviewChars: 150,
    anywherePolish: false,
    anywhereAppContexts: [],
  },
  stt: {
    // Backend materializes the real platform default (…/PortBay/ai-models/speech)
    // into get_preferences; this empty string never reaches the UI.
    modelsDir: "",
  },
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
  const ai = { ...DEFAULTS.ai, ...(prefs.ai ?? {}) };
  if (!ai.endpoint && prefs.dictation?.endpoint) ai.endpoint = prefs.dictation.endpoint;
  const dictation = { ...DEFAULTS.dictation, ...(prefs.dictation ?? {}), endpoint: ai.endpoint };
  return {
    ...prefs,
    notifications: normaliseNotificationPrefs(prefs.notifications ?? DEFAULT_NOTIFICATION_PREFS),
    accessibility: { ...DEFAULTS.accessibility, ...(prefs.accessibility ?? {}) },
    ai,
    dictation,
    stt: { ...DEFAULTS.stt, ...(prefs.stt ?? {}) },
  };
}

function clonePreferences(prefs: Preferences): Preferences {
  return normalisePreferences(JSON.parse(JSON.stringify(prefs)) as Preferences);
}
