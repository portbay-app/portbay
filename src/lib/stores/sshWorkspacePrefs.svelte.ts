/**
 * Terminal/workspace preferences for the SSH host workspace.
 *
 * Frontend-only and global (one set across hosts): font size, scrollback depth,
 * cursor blink, and an optional startup command run in each new interactive
 * shell. Persisted to localStorage — honest about being a UI preference, not a
 * server-side setting. The Settings tab edits these; SshTerminalSession reads
 * them when it creates a terminal.
 */
import { browser } from "$app/environment";

import {
  MAX_HIGHLIGHT_RULES,
  type HighlightRenderMode,
  type HighlightRule,
} from "$lib/ssh/terminalHighlight";

export type { HighlightRule };

const RENDER_MODES: HighlightRenderMode[] = ["background", "underline", "outline"];

export interface TerminalPrefs {
  /** xterm font size in px. */
  fontSize: number;
  /** Lines of scrollback retained per shell. */
  scrollback: number;
  /** Blinking block cursor. */
  cursorBlink: boolean;
  /** Command sent to every new interactive shell on open (blank = none). */
  startupCommand: string;
  /** Ordered regex → colour rules that tint matching terminal output. Order is
   *  priority: earlier rules win an overlap. */
  highlightRules: HighlightRule[];
  /** Feed recent terminal output (not just typed commands) to the host model
   *  for the inline next-command suggestion. Off by default: the buffer can hold
   *  secrets. When on it's capped + lightly redacted and still only ever sent to
   *  the host's own model over SSH. */
  suggestBufferContext: boolean;
}

const STORAGE_KEY = "portbay.ssh.terminalPrefs";

/** Build a full rule from a partial, filling production defaults. */
function makeRule(partial: Partial<HighlightRule> & Pick<HighlightRule, "id" | "pattern">): HighlightRule {
  return {
    label: "",
    isRegex: true,
    caseSensitive: false,
    color: "#3b82f6",
    renderMode: "background",
    enabled: true,
    ...partial,
  };
}

/** Starter rules so the feature is visibly useful out of the box (the two the
 *  task names): ERROR red, WARN amber. The user can edit, reorder, or remove. */
const DEFAULT_RULES: HighlightRule[] = [
  makeRule({ id: "default-error", label: "Errors", pattern: "\\b(error|fatal|fail(ed|ure)?)\\b", color: "#ef4444" }),
  makeRule({ id: "default-warn", label: "Warnings", pattern: "\\b(warn(ing)?|deprecated)\\b", color: "#f59e0b" }),
];

const DEFAULTS: TerminalPrefs = {
  fontSize: 13,
  scrollback: 10000,
  cursorBlink: true,
  startupCommand: "",
  highlightRules: DEFAULT_RULES,
  suggestBufferContext: false,
};

/** Coerce stored/parsed JSON into a clean rule list (drops malformed entries,
 *  clamps to the cap). Defensive because this comes off localStorage. */
function sanitizeRules(raw: unknown): HighlightRule[] {
  if (!Array.isArray(raw)) return DEFAULT_RULES.map((r) => ({ ...r }));
  const out: HighlightRule[] = [];
  for (const item of raw) {
    if (!item || typeof item !== "object") continue;
    const r = item as Record<string, unknown>;
    if (typeof r.pattern !== "string" || typeof r.color !== "string") continue;
    const renderMode =
      typeof r.renderMode === "string" && RENDER_MODES.includes(r.renderMode as HighlightRenderMode)
        ? (r.renderMode as HighlightRenderMode)
        : "background";
    out.push(
      makeRule({
        id: typeof r.id === "string" && r.id ? r.id : freshId(),
        label: typeof r.label === "string" ? r.label : "",
        pattern: r.pattern,
        // Pre-feature rules had no `isRegex` key and were always regex.
        isRegex: r.isRegex !== false,
        caseSensitive: r.caseSensitive === true,
        color: r.color,
        renderMode,
        enabled: r.enabled !== false,
      }),
    );
    if (out.length >= MAX_HIGHLIGHT_RULES) break;
  }
  return out;
}

/** A stable rule id. `crypto.randomUUID` is available in the app webview. */
function freshId(): string {
  if (browser && typeof crypto !== "undefined" && crypto.randomUUID) {
    return crypto.randomUUID();
  }
  return `rule-${Date.now()}-${Math.floor(Math.random() * 1e6)}`;
}

function load(): TerminalPrefs {
  if (!browser) return { ...DEFAULTS };
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return { ...DEFAULTS };
    const parsed = JSON.parse(raw) as Partial<TerminalPrefs>;
    return {
      fontSize: clamp(Number(parsed.fontSize) || DEFAULTS.fontSize, 9, 24),
      scrollback: clamp(Number(parsed.scrollback) || DEFAULTS.scrollback, 100, 200000),
      cursorBlink: parsed.cursorBlink ?? DEFAULTS.cursorBlink,
      startupCommand: typeof parsed.startupCommand === "string" ? parsed.startupCommand : "",
      // A stored prefs blob from before this feature has no `highlightRules`
      // key; seed the defaults so existing users get the starter rules too.
      highlightRules:
        "highlightRules" in parsed
          ? sanitizeRules(parsed.highlightRules)
          : DEFAULT_RULES.map((r) => ({ ...r })),
      suggestBufferContext: parsed.suggestBufferContext ?? DEFAULTS.suggestBufferContext,
    };
  } catch {
    return { ...DEFAULTS };
  }
}

function clamp(n: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, n));
}

function createTerminalPrefsStore() {
  let prefs = $state<TerminalPrefs>(load());

  function persist() {
    if (!browser) return;
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(prefs));
    } catch {
      /* storage unavailable (private mode); keep the in-memory value */
    }
  }

  function setRules(rules: HighlightRule[]) {
    prefs = { ...prefs, highlightRules: rules };
    persist();
  }

  return {
    get value() {
      return prefs;
    },
    /** Merge a partial update and persist. */
    update(patch: Partial<TerminalPrefs>) {
      prefs = { ...prefs, ...patch };
      persist();
    },
    reset() {
      prefs = { ...DEFAULTS, highlightRules: DEFAULT_RULES.map((r) => ({ ...r })) };
      persist();
    },

    /** Append a rule (blank by default, or seeded from a preset partial).
     *  No-op at the cap. Returns the new rule's id, or null if capped. */
    addHighlightRule(preset?: Partial<Omit<HighlightRule, "id">>): string | null {
      if (prefs.highlightRules.length >= MAX_HIGHLIGHT_RULES) return null;
      const rule = makeRule({ id: freshId(), pattern: "", ...preset });
      setRules([...prefs.highlightRules, rule]);
      return rule.id;
    },
    updateHighlightRule(id: string, patch: Partial<Omit<HighlightRule, "id">>) {
      setRules(prefs.highlightRules.map((r) => (r.id === id ? { ...r, ...patch } : r)));
    },
    removeHighlightRule(id: string) {
      setRules(prefs.highlightRules.filter((r) => r.id !== id));
    },
    /** Move the rule at `from` to index `to` (drag-reorder = re-prioritise). */
    reorderHighlightRule(from: number, to: number) {
      const rules = [...prefs.highlightRules];
      if (from < 0 || from >= rules.length || to < 0 || to >= rules.length || from === to) return;
      const [moved] = rules.splice(from, 1);
      rules.splice(to, 0, moved);
      setRules(rules);
    },
  };
}

export const terminalPrefs = createTerminalPrefsStore();
