<!--
  SshAgent — the Agent tab. One chat UI, three interchangeable brains selected by
  the provider switcher:

  • Local model (ollama): a model running ON the host via the cached SSH session,
    with PortBay's own per-command approval gate — the model proposes a shell
    command in a ```run block, the user approves, PortBay runs it, output fed back.
  • Claude Code / Codex: we drive the host's *officially-installed* CLI in its
    own non-interactive streaming mode and render the provider's own event stream
    (assistant text + tool activity) verbatim. We never inject a system prompt or
    re-implement the loop — the binary owns reasoning, tool execution, and safety;
    the user picks the official --permission-mode. Auth stays on the host
    (`claude login` / `codex login`); no API keys in PortBay.

  When nothing is detected, falls back to an install hint.
-->
<script lang="ts">
  import { onDestroy, onMount, tick } from "svelte";

  import { browser } from "$app/environment";

  import Icon, { type IconName } from "$lib/components/atoms/Icon.svelte";
  import Popover from "$lib/components/atoms/Popover.svelte";
  import AgentDropdown from "$lib/components/connections/agent/AgentDropdown.svelte";
  import AgentErrorBanner from "$lib/components/connections/agent/AgentErrorBanner.svelte";
  import CliAuthPrompt from "$lib/components/connections/agent/CliAuthPrompt.svelte";
  import IconLoading from "$lib/components/connections/agent/IconLoading.svelte";
  import Markdown from "$lib/components/connections/agent/Markdown.svelte";
  import NoAgentBackend from "$lib/components/connections/agent/NoAgentBackend.svelte";
  import OllamaModelHint from "$lib/components/connections/agent/OllamaModelHint.svelte";
  import ToolHeader from "$lib/components/connections/agent/ToolHeader.svelte";
  import DictationRewriteChip from "$lib/components/shared/DictationRewriteChip.svelte";
  import { createPushToTalk } from "$lib/dictation/pushToTalk";
  import { micSession } from "$lib/dictation/micSession.svelte";
  import { DictationRewriter } from "$lib/dictation/rewriter.svelte";
  import { readScrollback } from "$lib/dictation/terminalScrollback";
  import { extractTechnicalTerms } from "$lib/dictation/vocabulary";
  import { fuzzyMatch } from "$lib/fuzzy";
  import { preferences } from "$lib/stores/preferences.svelte";
  import { safeInvoke } from "$lib/ipc";
  import { openUrl } from "$lib/security/openUrl";
  import { agentModelCatalog, AGENT_MODELS, type AgentModel } from "$lib/ssh/agentModels";
  import {
    agentAbort,
    agentChat,
    agentCleanupAttachments,
    agentCliChat,
    agentClose,
    agentForwardStart,
    agentForwardStop,
    agentRun,
    agentUploadBytes,
    agentUploadPath,
    openAgent,
    type AgentEvent,
    type AgentInfo,
    type ChatMessage,
    type TodoItem,
  } from "$lib/ssh/agent";
  import { openPty, ptyClose, ptyInput, type PtyEvent } from "$lib/ssh/pty";
  import { agentProviderPref, type AgentProvider } from "$lib/stores/agentProviderPref.svelte";
  import {
    loadThreadStore,
    saveThreadStore,
    threadTitle,
    type AgentThread,
    type ChatMode,
    type ToolActivity,
    type Turn,
  } from "$lib/stores/agentThreads.svelte";

  let {
    connectionId,
    label,
    onClose,
    cwdRequest = null,
  }: {
    connectionId: string;
    label: string;
    onClose?: () => void;
    /** External working-dir push (Explorer's "Open agent here"). The nonce makes
        the same path re-applyable; we apply once the agent is ready so it wins
        over the cwd a restored thread loaded on mount. */
    cwdRequest?: { path: string; nonce: number } | null;
  } = $props();

  const SYSTEM_PROMPT =
    "You are a careful operations assistant working on a remote Linux host over SSH. " +
    "When you need to inspect or change the system, propose ONE shell command in a fenced " +
    "code block labelled `run`, e.g.\n```run\nls -la /var/log\n```\nThen stop — the command " +
    "only executes after the user approves it, and its output is sent back to you. Prefer " +
    "read-only commands first, and never destructive ones without explaining why. Be concise.";

  // `Turn`, `ToolActivity`, and `ChatMode` now live in the agent-threads store
  // (they're persisted per thread). `Attachment` stays here — it's transient.

  /** One file staged for the next turn. `path` items are read by the backend
      from a local path (picker / drag-drop); `bytes` items carry their data
      (clipboard-pasted images, which have no local path). */
  interface Attachment {
    id: string;
    name: string;
    size?: number;
    kind: "path" | "bytes";
    localPath?: string;
    dataBase64?: string;
  }

  let status = $state<"connecting" | "ready" | "noagents" | "error">("connecting");
  let info = $state<AgentInfo | null>(null);
  let model = $state("");
  // Model override for the CLI agents (Claude/Codex), set via `/model`. Passed as
  // the official `--model` flag; null = the provider's own default.
  let cliModel = $state<string | null>(null);
  let provider = $state<AgentProvider>("ollama");
  // Claude/Codex CLI session id we thread across turns with --resume. We don't
  // pick a permission mode — the official agent handles approvals in-chat itself.
  let sessionId = $state<string | null>(null);
  // Void-style chat mode (Agent / Gather / Normal). Each maps to the official
  // permission posture the CLI runs under (see MODE_PERMISSION). The `ChatMode`
  // type comes from the threads store (it's persisted per thread).
  const CHAT_MODES: ChatMode[] = ["normal", "gather", "agent"];
  const MODE_LABEL: Record<ChatMode, string> = { normal: "Chat", gather: "Gather", agent: "Agent" };
  const MODE_DETAIL: Record<ChatMode, string> = {
    normal: "Normal chat",
    gather: "Reads files, but can't edit",
    agent: "Edits files and uses tools",
  };
  // One glyph per mode for the selector: plain chat, read-only, agentic.
  const MODE_ICON: Record<ChatMode, IconName> = {
    normal: "message-square",
    gather: "eye",
    agent: "sparkles",
  };
  // The permission posture sent to the CLI for each mode. We send Claude's
  // official `--permission-mode` vocabulary; the backend translates it to Codex's
  // sandbox flags too (read-only / --full-auto). Normal = converse (Claude has no
  // true "no-tools" mode in -p, so `default` may attempt+auto-deny an edit);
  // Gather = `plan` (read + plan, never writes); Agent = `acceptEdits` (auto-accept
  // edits and commands — the chosen live-server posture). The CLI still owns the
  // actual approval loop; this only picks which official posture it runs under.
  const MODE_PERMISSION: Record<ChatMode, string> = {
    normal: "default",
    gather: "plan",
    agent: "acceptEdits",
  };
  let chatMode = $state<ChatMode>("agent");

  function setMode(m: ChatMode) {
    if (m === chatMode) return;
    chatMode = m;
    // A new permission posture starts a fresh CLI session so it takes effect
    // cleanly (same reasoning as changing the model).
    sessionId = null;
    persistActive();
  }

  // Working directory the agent (and approved commands) run in, so dropped-in
  // attachments and project-relative paths resolve like a local checkout. "~" =
  // the host's home (the default; no `cd` is issued). Persisted per host.
  let chatCwd = $state("~");
  const cwdKey = (id: string) => `portbay.agent.cwd.${id}`;
  function setCwd(v: string) {
    chatCwd = v.trim() || "~";
    // Keep a per-host "last cwd" as the default for new threads, and snapshot it
    // onto the active thread (cwd is per-thread now).
    if (browser) localStorage.setItem(cwdKey(connectionId), chatCwd);
    persistActive();
  }

  // Apply an external "Open agent here" request. Gated on `status === "ready"` so
  // it runs after onMount has restored the active thread (which sets `chatCwd`);
  // otherwise the restore would clobber the pushed dir. The nonce guard makes a
  // repeat request for the same folder re-apply without looping.
  let appliedCwdNonce = $state(-1);
  $effect(() => {
    const req = cwdRequest;
    if (!req || req.nonce === appliedCwdNonce || status !== "ready") return;
    appliedCwdNonce = req.nonce;
    setCwd(req.path);
  });

  // Voice-to-text via the OS's built-in dictation (same as the task board):
  // the backend sends `startDictation:` down the responder chain (the Edit ▸
  // "Start Dictation…" code path) so macOS types speech into the composer. No
  // permission grants involved; if Dictation is off, macOS shows its own
  // "Enable Dictation?" dialog.
  //
  // The mic is a TOGGLE owned by the shared `micSession` controller (see
  // $lib/dictation/micSession.svelte.ts): while held the button is the red
  // stop control with ping rings and an mm:ss clock. Internally the
  // controller tracks arming → live (the backend may spend seconds waiting
  // out a teardown cool-down and confirming with DictationIM) so clicks are
  // idempotent during the window; it owns the timers, the OS-event
  // listeners, the stop invokes, and the end-of-session handoff to the
  // rewrite layer.
  const COMPOSER_MIC = "ssh-composer";
  const COMMAND_MIC = "ssh-command";
  let composerTextarea = $state<HTMLTextAreaElement | null>(null);
  const dictating = $derived(micSession.heldBy(COMPOSER_MIC));
  /** Mic confirmed hot — the button becomes the stop control and the clock
   * runs. While merely arming it stays the (pulsing) mic. */
  const dictationLive = $derived(micSession.liveFor(COMPOSER_MIC));
  const dictationClock = $derived(
    `${String(Math.floor(micSession.seconds / 60)).padStart(2, "0")}:${String(micSession.seconds % 60).padStart(2, "0")}`,
  );
  // Blur-vs-stop-click ordering guard (see CardEditor): pointerdown precedes
  // any focus change, so it marks a click on the action button in flight and
  // the composer's blur defers to it.
  let micPressGuard = false;
  function guardMicPress() {
    micPressGuard = true;
    setTimeout(() => (micPressGuard = false), 400);
  }
  /** Composer blur = dictation over (macOS stops inserting on focus loss). */
  function onComposerBlur() {
    if (micPressGuard) return;
    micSession.release(COMPOSER_MIC);
  }

  // Smart Dictation: optional rewrite layer over what macOS typed into the
  // composer. Snapshot on start, diff on stop, rewrite only the inserted
  // segment via the (local) provider — every failure mode keeps the raw
  // transcript that's already in the field. See $lib/dictation.
  const dictationRewriter = new DictationRewriter({
    read: () => input,
    write: (v) => (input = v),
    // The composer talks to an AI agent; rewrites shape speech into a
    // precise, implementation-ready instruction.
    context: () => "agent_prompt",
    // Spelling reference for the rewrite: the host's label, the terminal
    // panes' recent buffer (the richest jargon source — hostnames, services,
    // paths live ON screen), and the recent conversation hold the exact
    // identifiers the user is likely speaking — the terms ASR mangles.
    vocabulary: () =>
      extractTechnicalTerms([
        label,
        readScrollback(connectionId),
        ...turns.slice(-6).map((t) => t.content),
      ]),
    // Slash commands are app syntax, not speech — never touch them.
    skip: (_inserted, value) => value.trimStart().startsWith("/"),
    // Voice Edit Mode: a selection at session start makes the dictation an
    // instruction about it ("make this more concise").
    selection: () =>
      composerTextarea
        ? { start: composerTextarea.selectionStart, end: composerTextarea.selectionEnd }
        : null,
  });

  // Pending-command gate dictation: its own surface (mic only, no clock) so
  // a voice tweak to a proposed command gets the terminal_command rewrite
  // shape (spoken operators → symbols, never invented flags). Same shared
  // controller — it owns the one OS session, so starting here hands off from
  // the composer (and vice versa) without a stop/start race.
  let commandTextarea = $state<HTMLTextAreaElement | null>(null);
  const commandDictating = $derived(micSession.heldBy(COMMAND_MIC));
  const commandLive = $derived(micSession.liveFor(COMMAND_MIC));
  let commandMicGuard = false;
  function guardCommandMicPress() {
    commandMicGuard = true;
    setTimeout(() => (commandMicGuard = false), 400);
  }
  function onCommandBlur() {
    if (commandMicGuard) return;
    micSession.release(COMMAND_MIC);
  }
  const commandRewriter = new DictationRewriter({
    read: () => pendingCommand ?? "",
    // The gate may have been approved/cancelled while the model worked; a
    // null field means the splice target is gone — drop the result.
    write: (v) => {
      if (pendingCommand !== null) pendingCommand = v;
    },
    context: () => "terminal_command",
    // The pending command's own tokens lead the spelling reference — a voice
    // edit almost always names the flags/paths already in it — then the
    // terminal buffer, then the conversation.
    vocabulary: () =>
      extractTechnicalTerms([
        pendingCommand ?? "",
        label,
        readScrollback(connectionId),
        ...turns.slice(-4).map((t) => t.content),
      ]),
    selection: () =>
      commandTextarea
        ? { start: commandTextarea.selectionStart, end: commandTextarea.selectionEnd }
        : null,
  });
  // Per-surface session hooks: focus + rewrite snapshot when the controller
  // grants the session; rewrite finish on its single end path.
  const composerMicHooks = {
    begin: () => {
      composerTextarea?.focus();
      dictationRewriter.begin();
    },
    // Queried after begin(): "edit" when a selection flipped the session
    // into Voice Edit Mode — labels the notch overlay's leading slot.
    mode: () => dictationRewriter.sessionMode,
    // Local STT engine: the sidecar's final transcript lands at the
    // begin()-time caret (macOS dictation types it itself).
    insertTranscript: (text: string) => dictationRewriter.insert(text),
    end: () => dictationRewriter.finish(),
  };
  const commandMicHooks = {
    begin: () => {
      commandTextarea?.focus();
      commandRewriter.begin();
    },
    mode: () => commandRewriter.sessionMode,
    insertTranscript: (text: string) => commandRewriter.insert(text),
    end: () => commandRewriter.finish(),
  };
  function runCommandDictation() {
    commandMicGuard = false; // click arrived; blur guard no longer needed
    micSession.toggle(COMMAND_MIC, commandMicHooks);
  }
  // Surface unmount must never leave a session running with nobody watching.
  $effect(() => {
    return () => {
      micSession.release(COMPOSER_MIC);
      micSession.release(COMMAND_MIC);
    };
  });

  // Push-to-talk (hold the Fn key): same start/stop/rewrite path as the
  // mic buttons, targeting whichever dictation field holds focus (composer
  // or the pending-command gate). Disposed by the $effect teardown.
  $effect(() => {
    return createPushToTalk<"composer" | "command">({
      // Always on (2026-06-06, user decision: the Settings toggle is gone —
      // push-to-talk is just part of how dictation works).
      enabled: () => true,
      target: () => {
        const el = document.activeElement;
        if (composerTextarea && el === composerTextarea) return "composer";
        if (commandTextarea && el === commandTextarea) return "command";
        return null;
      },
      start: (which) => {
        if (which === "composer") {
          if (!dictating) runDictation();
        } else if (!commandDictating) {
          runCommandDictation();
        }
      },
      stop: () => {
        micSession.release(COMPOSER_MIC);
        micSession.release(COMMAND_MIC);
      },
    });
  });

  const PROVIDER_LABELS: Record<AgentProvider, string> = {
    ollama: "Ollama",
    claude: "Claude Code",
    codex: "Codex",
  };

  // Empty-state suggestion chips (Void's landing-page suggestions), tuned per
  // brain: ollama is host-ops focused (it drives PortBay's command gate), the CLI
  // agents are project/code focused.
  const SUGGESTIONS: Record<AgentProvider, string[]> = {
    ollama: [
      "Summarize what's running on this host",
      "Check disk and memory usage",
      "Show the most recent errors in the system log",
    ],
    claude: [
      "Summarize this project's structure",
      "Find and fix a failing test",
      "Explain what the main entry point does",
    ],
    codex: [
      "Explain this repo's structure",
      "Write a unit test for the main module",
      "Refactor the largest source file",
    ],
  };

  // Map a raw agent tool name (Claude's PascalCase tools, Codex's snake_case) to
  // a friendly Void-style title: [done, running]. Unknown tools fall back to the
  // raw name so a new/MCP tool still reads sensibly.
  function friendlyTool(name: string): [string, string] {
    const map: Record<string, [string, string]> = {
      Read: ["Read file", "Reading file"],
      Edit: ["Edited file", "Editing file"],
      MultiEdit: ["Edited file", "Editing file"],
      Write: ["Wrote file", "Writing file"],
      NotebookEdit: ["Edited notebook", "Editing notebook"],
      Bash: ["Ran command", "Running command"],
      Glob: ["Searched files", "Searching files"],
      Grep: ["Searched", "Searching"],
      LS: ["Listed folder", "Listing folder"],
      WebFetch: ["Fetched page", "Fetching page"],
      WebSearch: ["Searched web", "Searching web"],
      Task: ["Delegated task", "Delegating task"],
      shell: ["Ran command", "Running command"],
      web_search: ["Searched web", "Searching web"],
    };
    return map[name] ?? [name, name];
  }

  // One brand icon per provider. ollama has no app icon, so it gets a glyph.
  const PROVIDER_IMG: Partial<Record<AgentProvider, string>> = {
    claude: "/apps/claude.png",
    codex: "/apps/codex.png",
  };

  // The single picker lists one row per provider the host offers. Ollama shows
  // whenever the host has it at all (the runtime/CLI is present, or it already
  // reported models) — not only once a model is pulled — so people who want to
  // use it can pick it and pull a model from the prompt below. The CLI agents
  // just need their binary on PATH.
  const availableProviders = $derived.by<AgentProvider[]>(() => {
    if (!info) return [];
    const list: AgentProvider[] = [];
    if (info.hasOllama || info.ollamaModels.length > 0) list.push("ollama");
    if (info.hasClaude) list.push("claude");
    if (info.hasCodex) list.push("codex");
    return list;
  });

  function selectProvider(next: AgentProvider) {
    if (next === provider || busy) return;
    agentProviderPref.set(connectionId, next);
    if (turns.length === 0) {
      // Empty thread: just swap the brain in place (no history to lose).
      provider = next;
      cliModel = null; // model overrides are provider-specific
      if (next === "ollama" && info?.ollamaModels.length) model = info.ollamaModels[0];
      sessionId = null;
      todos = [];
      persistActive();
    } else {
      // A started conversation belongs to its brain — open a fresh thread for the
      // new one instead of mixing transcripts.
      newThread(next);
    }
  }

  // --- Multiple threads (Void's chatThreadService) ---
  // All conversations for this host; the active one's fields are mirrored into
  // the working state below (turns / sessionId / provider / model / mode / cwd /
  // todos) so the rest of the component is unchanged. `persistActive` snapshots
  // them back into `threads` and to localStorage.
  let threads = $state<AgentThread[]>([]);
  let activeThreadId = $state("");
  // Two-step delete confirmation in the history menu (Void's TrashButton).
  let confirmDeleteId = $state<string | null>(null);
  const sortedThreads = $derived([...threads].sort((a, b) => b.lastModified - a.lastModified));

  let turns = $state<Turn[]>([]);
  let input = $state("");
  let busy = $state(false);
  // Double-Escape interrupt (Codex / Claude Code convention): the first Esc
  // while busy arms a short window + hint, a second press within it stops the
  // turn — one stray Esc can't kill a long-running turn.
  let escArmed = $state(false);
  let escArmTimer: ReturnType<typeof setTimeout> | null = null;
  let streaming = $state("");
  let streamingReasoning = $state("");
  let streamingTools = $state<ToolActivity[]>([]);
  let pendingCommand = $state<string | null>(null);
  let scroller = $state<HTMLDivElement | null>(null);
  // Void-style scroll behaviour: stick to the bottom only while the user is
  // already there, so streaming tokens don't yank them down mid-read. A
  // scroll-to-bottom button appears when they've scrolled up.
  let atBottom = $state(true);
  // One error channel for the agent surface. Auth failures still use the sign-in
  // CTA, but connection, turn, and upload failures all land here by slot.
  type AgentErrorSlot = "connection" | "turn" | "upload";
  let agentError = $state<{ slot: AgentErrorSlot; message: string } | null>(null);
  // In-place edit of a past user message (Void's UserMessageComponent edit mode):
  // the index being edited and its working text.
  let editingIndex = $state<number | null>(null);
  let editText = $state("");
  // The agent's own plan (Claude TodoWrite / Codex todo_list), parsed from the
  // stream we already read — no extra tokens. Persists after the turn so it can
  // be reviewed; cleared when a new turn starts or the chat is reset.
  let todos = $state<TodoItem[]>([]);
  let planOpen = $state(true);
  const todoDone = $derived(todos.filter((t) => t.status === "completed").length);
  // --- In-app sign-in (Feature 1) ---
  // Which CLI provider's last turn failed for auth, so we offer a sign-in CTA.
  let authNeeded = $state<"claude" | "codex" | null>(null);
  // The sign-in flow drives the official `setup-token` / `login` over a PTY at a
  // WIDE width (so the CLI doesn't hard-wrap the OAuth URL), extracts the URL,
  // auto-opens it, and renders it as a real HTML link — then the user pastes the
  // code back. Null when not signing in; `done` once the command exits.
  let signIn = $state<null | {
    provider: "claude" | "codex";
    ptyId: string | null;
    url: string | null;
    stage: "starting" | "awaiting" | "error";
    error?: string;
  }>(null);
  let signInCode = $state("");
  // True once the code has been sent, while we wait for the CLI to finish.
  let signInSubmitted = $state(false);
  // Non-reactive accumulation of PTY output, scanned for the OAuth URL.
  let signInBuf = "";
  // Wide enough that the CLI prints the (long) URL on a single line.
  const SIGNIN_WIDTH = 1000;
  // `codex login`'s loopback OAuth callback port (forwarded local→remote so the
  // browser redirect reaches the host's callback server).
  const CODEX_CALLBACK_PORT = 1455;

  // --- Attachments (Feature 2) ---
  let attachments = $state<Attachment[]>([]);
  let composerEl = $state<HTMLDivElement | null>(null);
  let dragOver = $state(false);
  let attachSeq = 0;
  let pasteCount = 0;
  let pendingAttachmentCleanupTurnId = $state<string | null>(null);

  // Official sign-in / sign-out commands. `claude auth login` (default
  // --claudeai) creates the *subscription session credential that `claude -p`
  // reads* — `setup-token` only mints a long-lived token for SDK/API use and does
  // NOT authenticate the CLI, so it 401s. Codex uses its own `login`/`logout`.
  const SIGNIN_CMD: Record<"claude" | "codex", string> = {
    claude: "claude auth login",
    codex: "codex login",
  };
  const LOGOUT_CMD: Record<"claude" | "codex", string> = {
    claude: "claude auth logout",
    codex: "codex logout",
  };

  // Pull the first ```run / ```bash / ```sh command out of an assistant reply.
  function extractCommand(content: string): string | null {
    const m = content.match(/```(?:run|bash|sh)\s*\n([\s\S]*?)```/i);
    return m ? m[1].trim() || null : null;
  }

  function formatResult(stdout: string, stderr: string, exitCode: number): string {
    let out = stdout;
    if (stderr.trim()) out += (out ? "\n" : "") + stderr;
    out = out.trim();
    if (out.length > 6000) out = out.slice(0, 6000) + "\n…(truncated)";
    const code = exitCode !== 0 ? `\n[exit ${exitCode}]` : "";
    return (out || "(no output)") + code;
  }

  /** Scroll to the newest content. While streaming we only follow if the user is
      already at the bottom (`atBottom`); `force` overrides that for their own
      actions (sending, jumping via the button). */
  async function scrollToEnd(force = false) {
    if (!force && !atBottom) return;
    await tick();
    if (scroller) {
      scroller.scrollTop = scroller.scrollHeight;
      atBottom = true;
    }
  }

  /** Track whether the transcript is pinned to the bottom (within 8px). */
  function onScroll() {
    if (!scroller) return;
    atBottom = scroller.scrollHeight - scroller.clientHeight - scroller.scrollTop < 8;
  }

  /** Pull a human message off a rejected invoke (AppError) or any thrown value. */
  function errText(e: unknown, fallback = "Something went wrong."): string {
    return e && typeof e === "object" && "whatHappened" in e
      ? String((e as { whatHappened: unknown }).whatHappened)
      : fallback;
  }

  function setAgentError(slot: AgentErrorSlot, message: string) {
    agentError = { slot, message };
  }

  function clearAgentError(slot?: AgentErrorSlot) {
    if (!slot || agentError?.slot === slot) agentError = null;
  }

  // --- Sign-in helpers (Feature 1) ---

  /** Strip ANSI/OSC escapes so the URL scan isn't fooled by colour codes. */
  function stripAnsi(s: string): string {
    // eslint-disable-next-line no-control-regex
    return s.replace(/\x1b\[[0-9;?]*[ -/]*[@-~]/g, "").replace(/\x1b\][^\x07]*(\x07|\x1b\\)/g, "");
  }

  /** Pick the OAuth URL out of the (wide, unwrapped) CLI output. Prefer an
      auth-looking URL; trim trailing punctuation (e.g. codex's `:1455.`). */
  function pickAuthUrl(text: string): string | null {
    // eslint-disable-next-line no-control-regex
    const urls = text.match(/https?:\/\/[^\s\x00-\x1f]+/g);
    if (!urls) return null;
    const clean = (u: string) => u.replace(/[.,)\]}>"'”]+$/, "");
    const preferred = urls.find((u) => /oauth|authoriz|redirect_uri|login|setup/i.test(u));
    return clean(preferred ?? [...urls].sort((a, b) => b.length - a.length)[0]);
  }

  /** Start the official sign-in over a PTY at a wide width (so the URL isn't
      wrapped), extract the URL, auto-open it, and surface it as a clickable HTML
      link. The binary owns the auth; nothing touches PortBay. */
  async function startSignIn(provider: "claude" | "codex") {
    authNeeded = null;
    signInCode = "";
    signInSubmitted = false;
    signInBuf = "";
    signIn = { provider, ptyId: null, url: null, stage: "starting" };
    // `codex login` runs its OAuth callback server on a loopback port of the HOST;
    // forward the same local port so the browser's redirect to localhost reaches it.
    if (provider === "codex") {
      try {
        await agentForwardStart(connectionId, CODEX_CALLBACK_PORT, CODEX_CALLBACK_PORT);
      } catch (e) {
        signIn = {
          ...signIn,
          stage: "error",
          error: errText(
            e,
            `Couldn't open the Codex callback forward (local port ${CODEX_CALLBACK_PORT} may be in use).`,
          ),
        };
        return;
      }
    }
    const onEvent = (e: PtyEvent) => {
      if (!signIn) return;
      if (e.type === "data") {
        signInBuf = (signInBuf + new TextDecoder().decode(new Uint8Array(e.bytes))).slice(-20000);
        if (!signIn.url) {
          const url = pickAuthUrl(stripAnsi(signInBuf));
          if (url) {
            signIn = { ...signIn, url, stage: "awaiting" };
            void openUrl(url); // auto-open the authorization page
          }
        }
      } else if (e.type === "exit") {
        const prov = signIn.provider;
        if (prov === "codex") agentForwardStop(connectionId);
        signIn = null;
        // The buffer can hold an echo of the pasted auth code — drop it now.
        signInBuf = "";
        finishSignIn(prov);
      }
    };
    try {
      const id = await openPty(connectionId, label, SIGNIN_WIDTH, 24, onEvent, SIGNIN_CMD[provider]);
      if (signIn) signIn = { ...signIn, ptyId: id };
    } catch (e) {
      if (provider === "codex") agentForwardStop(connectionId);
      if (signIn) signIn = { ...signIn, stage: "error", error: errText(e, "Couldn't start sign-in.") };
    }
  }

  /** Send the code the user pasted from the browser back to the waiting CLI.
      The Enter is a SEPARATE write: some CLI TUIs (Ink) batch a `"code\r"` chunk
      and append the CR to the field instead of submitting it. */
  function submitSignInCode() {
    const id = signIn?.ptyId;
    const code = signInCode.trim();
    if (!id || !code) return;
    ptyInput(id, code);
    setTimeout(() => ptyInput(id, "\r"), 50);
    signInCode = "";
    signInSubmitted = true;
  }

  function cancelSignIn() {
    if (signIn?.ptyId) ptyClose(signIn.ptyId);
    if (signIn?.provider === "codex") agentForwardStop(connectionId);
    signIn = null;
    signInBuf = "";
    if (pendingAttachmentCleanupTurnId) void cleanupUploadedTurn(pendingAttachmentCleanupTurnId);
  }

  /** Sign-in PTY exited → the host's `claude`/`codex login` finished. Confirm it
      in the transcript so the user knows they're authenticated, then auto-resume
      the turn they were trying to send (if any) on a fresh CLI session — no
      manual "retry" step. The stored turn still carries any `@path` attachment
      references (already uploaded), so resending keeps them. */
  function finishSignIn(provider: AgentProvider) {
    authNeeded = null;
    pushInfo(`✓ Signed in to ${PROVIDER_LABELS[provider]} on this host.`);
    if (busy || !turns.some((t) => t.role === "user")) return;
    sessionId = null;
    void runTurn();
  }

  // --- Attachment helpers (Feature 2) ---

  function localBasename(p: string): string {
    const seg = p.split(/[\\/]/).filter(Boolean);
    return seg.length ? seg[seg.length - 1] : p;
  }

  function addPathAttachment(localPath: string) {
    attachments = [
      ...attachments,
      { id: `a${attachSeq++}`, name: localBasename(localPath), kind: "path", localPath },
    ];
  }

  async function pickFiles() {
    // Host-side picker: the chosen paths are recorded in the approved set so
    // the later `ssh_agent_upload_path` read passes its approval check. (Same
    // command the file browser uses; the renderer never sees a raw dialog.)
    const paths = (await safeInvoke<string[]>("sftp_pick_upload_files")) ?? [];
    for (const p of paths) addPathAttachment(p);
  }

  function removeAttachment(id: string) {
    attachments = attachments.filter((a) => a.id !== id);
  }

  /** Best-effort removal of one turn's staged attachments from the host once the
      agent has consumed them. Only the `~/.portbay/agent-attachments/<turn>`
      staging copy is deleted — anything the agent copied/moved into the project
      during the turn lives outside that directory and is untouched. This keeps
      pasted screenshots and other one-off context from lingering on live hosts. */
  async function cleanupUploadedTurn(turnId: string) {
    if (pendingAttachmentCleanupTurnId === turnId) pendingAttachmentCleanupTurnId = null;
    try {
      await agentCleanupAttachments(connectionId, turnId);
    } catch {
      /* best-effort — closing the agent session wipes the whole staging dir */
    }
  }

  /** base64 of raw bytes, chunked so a large image doesn't blow the call stack. */
  function bytesToBase64(bytes: Uint8Array): string {
    let bin = "";
    const CHUNK = 0x8000;
    for (let i = 0; i < bytes.length; i += CHUNK) {
      bin += String.fromCharCode(...bytes.subarray(i, i + CHUNK));
    }
    return btoa(bin);
  }

  /** Clipboard image → in-memory byte attachment (no local path exists). */
  function onPaste(e: ClipboardEvent) {
    const items = e.clipboardData?.items;
    if (!items) return;
    for (const item of Array.from(items)) {
      if (item.kind === "file" && item.type.startsWith("image/")) {
        const blob = item.getAsFile();
        if (!blob) continue;
        e.preventDefault();
        const ext = item.type.split("/")[1] || "png";
        void blob.arrayBuffer().then((buf) => {
          attachments = [
            ...attachments,
            {
              id: `a${attachSeq++}`,
              name: `pasted-${++pasteCount}.${ext}`,
              size: blob.size,
              kind: "bytes",
              dataBase64: bytesToBase64(new Uint8Array(buf)),
            },
          ];
        });
      }
    }
  }

  /** Reference the uploaded host paths the way each brain reads them: Claude
      resolves `@path` natively; the others just get the paths mentioned. */
  function buildPrompt(text: string, paths: string[]): string {
    if (paths.length === 0) return text;
    const refs =
      provider === "claude"
        ? paths.map((p) => `@${p}`).join(" ")
        : `Files on the host: ${paths.join(", ")}`;
    return text ? `${refs}\n\n${text}` : refs;
  }

  function formatBytes(n?: number): string {
    if (n === undefined) return "";
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(0)} KB`;
    return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  }

  // --- Slash commands ---
  // We drive the CLI non-interactively, so its REPL slash commands (/login,
  // /clear, …) aren't processed by the binary. We handle the session/management
  // ones here (mapping /login to the official OAuth, etc.) and pass any UNKNOWN
  // slash through to the CLI unchanged — the official agent still owns custom and
  // project commands. See the agent-official-cli philosophy.
  interface SlashCommand {
    name: string;
    aliases?: string[];
    desc: string;
    /** Restrict to these providers; omitted = all. */
    providers?: AgentProvider[];
    /** Command takes an argument (e.g. `/model <name>`); the menu tab-completes it. */
    takesArg?: boolean;
    run: (arg: string) => void;
  }

  function pushInfo(text: string) {
    turns = [...turns, { role: "assistant", content: text }];
    persistActive();
    void scrollToEnd();
  }

  async function doLogout() {
    if (provider !== "claude" && provider !== "codex") return;
    busy = true;
    try {
      const res = await agentRun(connectionId, LOGOUT_CMD[provider]);
      sessionId = null;
      authNeeded = provider;
      pushInfo(res.stdout?.trim() || `Signed out of ${PROVIDER_LABELS[provider]} on this host.`);
    } catch {
      pushInfo(`Couldn't sign out of ${PROVIDER_LABELS[provider]} on this host.`);
    } finally {
      busy = false;
    }
  }

  function showHelp() {
    const lines = SLASH_COMMANDS.filter((c) => !c.providers || c.providers.includes(provider))
      .map((c) => `/${c.name}${c.takesArg ? " <value>" : ""} — ${c.desc}`)
      .join("\n");
    pushInfo(
      `Slash commands:\n${lines}\n\n(any other /command is passed to ${PROVIDER_LABELS[provider]} as-is)`,
    );
  }

  /** Set the model the agent uses. For Ollama this is the local model name; for
      Claude/Codex it rides the official `--model` flag on the next turn. */
  function setModel(arg: string) {
    const m = arg.trim();
    const current = provider === "ollama" ? model : cliModel;
    if (!m) {
      const head = `Current model: ${current || `${PROVIDER_LABELS[provider]} default`}. `;
      const names = agentModels.map((x) => x.id).join(", ");
      pushInfo(
        head +
          (names
            ? `Pick from the Model menu, or type \`/model <name>\` (${names}` +
              `${provider === "ollama" ? "" : ", or `default`"}).`
            : `Type \`/model <name>\` — it's passed straight to ${PROVIDER_LABELS[provider]}'s \`--model\`.`),
      );
      return;
    }
    if (provider === "ollama") {
      model = m;
    } else {
      // "default" isn't a real `--model` value — it means "clear the override
      // and let the CLI pick its own default" (null = no `--model` flag).
      cliModel = m === "default" ? null : m;
      sessionId = null; // a new model starts a fresh CLI session
    }
    pushInfo(
      m === "default" && provider !== "ollama"
        ? `Using the ${PROVIDER_LABELS[provider]} default model.`
        : `Model set to \`${m}\` for ${PROVIDER_LABELS[provider]}.`,
    );
  }

  const SLASH_COMMANDS: SlashCommand[] = [
    {
      name: "model",
      desc: "Set the model the agent uses",
      takesArg: true,
      run: (arg) => setModel(arg),
    },
    {
      name: "login",
      aliases: ["signin"],
      desc: "Sign in to this host's agent (official OAuth)",
      providers: ["claude", "codex"],
      run: () => startSignIn(provider as "claude" | "codex"),
    },
    {
      name: "logout",
      aliases: ["signout"],
      desc: "Sign out of this host's agent",
      providers: ["claude", "codex"],
      run: () => void doLogout(),
    },
    {
      name: "status",
      desc: "Show this host's agent sign-in status",
      providers: ["claude"],
      run: () => void doStatus(),
    },
    { name: "clear", desc: "Clear this conversation", run: () => reset() },
    { name: "help", desc: "Show available commands", run: () => showHelp() },
  ];

  /** Show Claude's own auth status (`claude auth status`) — useful to confirm a
      sign-in actually took (vs. a 401 from a token that doesn't authenticate -p). */
  async function doStatus() {
    if (provider !== "claude") return;
    busy = true;
    try {
      const res = await agentRun(connectionId, "claude auth status");
      const out = `${res.stdout}\n${res.stderr}`.trim();
      pushInfo(out || "No status reported by Claude Code.");
    } catch {
      pushInfo("Couldn't read Claude Code status on this host.");
    } finally {
      busy = false;
    }
  }

  /** Handle a recognised session/management slash command. Returns true when
      handled (don't send to the model); false to pass the text through. */
  function handleSlash(text: string): boolean {
    const m = text.match(/^\/([a-z][\w-]*)\s*([\s\S]*)$/i);
    if (!m) return false;
    const key = m[1].toLowerCase();
    const arg = m[2].trim();
    const known = SLASH_COMMANDS.find((c) => c.name === key || c.aliases?.includes(key));
    if (!known) return false; // unknown → pass through to the CLI (it owns it)
    if (known.providers && !known.providers.includes(provider)) {
      pushInfo(`\`/${key}\` isn't available for ${PROVIDER_LABELS[provider]}.`);
      return true;
    }
    known.run(arg);
    return true;
  }

  // Model picks reuse the app's ONE canonical resolver (`agentModelCatalog`) —
  // the same source the task-board card picker uses — so there's a single
  // place to maintain and the chat never drifts from the board. Where the CLI
  // publishes a live host catalog (Codex's server-refreshed
  // `models_cache.json`) that list is used, so provider releases/retirements
  // are picked up automatically; Claude's entries are version-agnostic aliases
  // (sonnet/opus/haiku) that resolve to the latest the CLI ships. Ollama is
  // the exception: the remote host reports its real installed models, so we
  // list those. Any model can still be set by hand via `/model <name>` — the
  // selector is just the discoverable shortcut.
  let liveModels = $state<Record<string, AgentModel[]>>({});
  $effect(() => {
    const p = provider;
    if (p !== "ollama" && !(p in liveModels) && (AGENT_MODELS[p] ?? []).length) {
      void agentModelCatalog(p).then((models) => {
        liveModels = { ...liveModels, [p]: models };
      });
    }
  });
  const agentModels = $derived<AgentModel[]>(
    provider === "ollama"
      ? (info?.ollamaModels ?? []).map((m) => ({ id: m, name: m }))
      : (liveModels[provider] ?? AGENT_MODELS[provider] ?? []),
  );
  // The currently-pinned model id, and its catalogue entry (if it's a known one).
  const currentModelId = $derived(provider === "ollama" ? model : cliModel);
  // "Default" (clear the override, let the CLI choose) is only meaningful for the
  // agent CLIs; Ollama always needs a concrete local model.
  const modelHasDefault = $derived(provider !== "ollama");

  // Ollama is selectable but the host has no model pulled yet — block sending and
  // show the pull hint instead of a chat that would 400 on an empty model name.
  const ollamaNeedsModel = $derived(provider === "ollama" && agentModels.length === 0);

  // --- Slash / argument menu (Esc dismisses until the next keystroke) ---
  let slashIndex = $state(0);
  let slashDismissed = $state(false);

  // Argument mode: when `/model` is fully typed, offer the provider's models to
  // pick (so the user doesn't have to know the name).
  const modelMenuOpen = $derived(
    !slashDismissed && (input === "/model" || input.toLowerCase().startsWith("/model ")),
  );
  // Command mode: a single `/token` before any space (and not the model-arg case).
  const commandMenuOpen = $derived(
    !slashDismissed && input.startsWith("/") && !input.trimEnd().includes(" ") && !modelMenuOpen,
  );

  // `indices` are the matched character positions in `name`, for highlighting
  // (Lapce-style). Empty when the query matched an alias/id rather than the
  // visible name, or when there's no query yet.
  type MenuItem =
    | { kind: "command"; name: string; desc: string; cmd: SlashCommand; indices: number[] }
    | { kind: "model"; name: string; desc: string; model: string; indices: number[] };

  /** Best fuzzy match across a primary string (highlighted) and optional
      fallbacks (id / aliases, matched but not highlighted). Returns the score
      and the indices to highlight (only when the primary won). */
  function bestMatch(
    q: string,
    primary: string,
    fallbacks: string[],
  ): { score: number; indices: number[] } | null {
    const onPrimary = fuzzyMatch(q, primary);
    let best = onPrimary ? { score: onPrimary.score, indices: onPrimary.indices } : null;
    for (const f of fallbacks) {
      const m = fuzzyMatch(q, f);
      // A fallback only wins on score; it never contributes name highlights.
      if (m && (!best || m.score > best.score)) best = { score: m.score, indices: [] };
    }
    return best;
  }

  const menuItems = $derived.by<MenuItem[]>(() => {
    if (modelMenuOpen) {
      const q = input.replace(/^\/model\s*/i, "");
      return agentModels
        .map((m) => {
          const match = bestMatch(q, m.name, [m.id]);
          if (!match) return null;
          const item: MenuItem = {
            kind: "model",
            name: m.name,
            desc: m.id === currentModelId ? "current" : (m.description ?? m.id),
            model: m.id,
            indices: match.indices,
          };
          return { item, score: match.score, len: m.name.length };
        })
        .filter((x): x is NonNullable<typeof x> => x !== null)
        .sort((a, b) => b.score - a.score || a.len - b.len)
        .map((x) => x.item);
    }
    if (commandMenuOpen) {
      const q = input.slice(1);
      return SLASH_COMMANDS.filter((c) => !c.providers || c.providers.includes(provider))
        .map((c) => {
          const match = bestMatch(q, c.name, c.aliases ?? []);
          if (!match) return null;
          const item: MenuItem = {
            kind: "command",
            name: c.name,
            desc: c.desc,
            cmd: c,
            indices: match.indices,
          };
          return { item, score: match.score, len: c.name.length };
        })
        .filter((x): x is NonNullable<typeof x> => x !== null)
        .sort((a, b) => b.score - a.score || a.len - b.len)
        .map((x) => x.item);
    }
    return [];
  });
  const menuOpen = $derived(menuItems.length > 0);
  $effect(() => {
    if (slashIndex >= menuItems.length) slashIndex = 0;
  });

  /** Activate a menu item: pick a model, run a no-arg command, or open the
      argument picker for a command that takes one (e.g. `/model`). */
  function selectMenuItem(item: MenuItem) {
    if (item.kind === "model") {
      input = "";
      slashDismissed = false;
      setModel(item.model);
      return;
    }
    const cmd = item.cmd;
    if (cmd.providers && !cmd.providers.includes(provider)) {
      input = "";
      slashDismissed = false;
      pushInfo(`\`/${cmd.name}\` isn't available for ${PROVIDER_LABELS[provider]}.`);
      return;
    }
    if (cmd.takesArg) {
      // Open the argument picker (e.g. the model list) instead of running.
      input = `/${cmd.name} `;
      slashDismissed = false;
      slashIndex = 0;
      return;
    }
    input = "";
    slashDismissed = false;
    cmd.run("");
  }

  // Stream one assistant turn. ollama replays the whole transcript through
  // PortBay's command-proposal gate; Claude/Codex get only the latest user turn
  // (the CLI keeps its own history via --resume) and stream their own events.
  async function runTurn() {
    busy = true;
    streaming = "";
    streamingReasoning = "";
    streamingTools = [];
    todos = []; // a new turn starts a fresh plan
    // A fresh attempt supersedes any prior sign-in prompt / error; they re-set if
    // it fails again.
    authNeeded = null;
    clearAgentError("turn");
    let content = "";
    let turnError: string | null = null;

    const onEvent = (e: AgentEvent) => {
      if (e.type === "token") {
        content += e.text;
        streaming = content;
        void scrollToEnd();
      } else if (e.type === "reasoning") {
        streamingReasoning += e.text;
        void scrollToEnd();
      } else if (e.type === "session") {
        sessionId = e.id;
      } else if (e.type === "toolUse") {
        streamingTools = [...streamingTools, { name: e.name, summary: e.summary }];
        void scrollToEnd();
      } else if (e.type === "toolResult") {
        // Attach to the most recent tool call still awaiting a result.
        const idx = [...streamingTools].reverse().findIndex((t) => t.result === undefined);
        if (idx !== -1) {
          const at = streamingTools.length - 1 - idx;
          streamingTools = streamingTools.map((t, i) =>
            i === at ? { ...t, result: e.summary, isError: e.isError } : t,
          );
        }
      } else if (e.type === "todos") {
        // The agent re-publishes the whole list each update — replace wholesale.
        todos = e.items;
        void scrollToEnd();
      } else if (e.type === "done") {
        content = e.content || content;
      } else if (e.type === "error") {
        turnError = e.message;
        // A structured auth failure (from the CLI providers) offers in-app sign-in.
        if (e.auth && (provider === "claude" || provider === "codex")) authNeeded = provider;
      }
    };

    try {
      if (provider === "ollama") {
        const messages: ChatMessage[] = [
          { role: "system", content: SYSTEM_PROMPT },
          ...turns.map((t) => ({ role: t.role, content: t.content })),
        ];
        await agentChat(connectionId, model, messages, info?.port ?? 11434, onEvent);
      } else {
        const lastUser = [...turns].reverse().find((t) => t.role === "user");
        // The chat mode picks the official permission posture (Claude
        // `--permission-mode`; the backend maps it to Codex's sandbox flags).
        // `cliModel` rides the official `--model` flag (null = provider default).
        await agentCliChat(
          connectionId,
          provider,
          lastUser?.content ?? "",
          MODE_PERMISSION[chatMode],
          sessionId,
          cliModel,
          chatCwd,
          onEvent,
        );
      }
    } catch (e) {
      turnError =
        e && typeof e === "object" && "whatHappened" in e
          ? String((e as { whatHappened: unknown }).whatHappened)
          : "The model request failed.";
    }

    const tools = streamingTools;
    const reasoning = streamingReasoning.trim() || undefined;
    streaming = "";
    streamingReasoning = "";
    streamingTools = [];
    // Commit a turn whenever anything surfaced — text, tool activity, or
    // reasoning — even if it ended in an error, so the tool rows stay visible.
    if (content || tools.length > 0 || reasoning) {
      turns = [...turns, { role: "assistant", content, tools, reasoning }];
      // Only the ollama brain proposes commands for PortBay to run; the CLI
      // agents execute their own tools.
      if (provider === "ollama") pendingCommand = extractCommand(content);
    }
    if (turnError) {
      // A real failure (a user abort is reported by the backend as success, so it
      // never lands here). Don't --resume a half-established session. Auth
      // failures show the sign-in CTA; everything else shows the error card.
      sessionId = null;
      if (!authNeeded) setAgentError("turn", turnError);
    }
    persistActive();
    busy = false;
    // The CLI consumed any staged attachments during the turn — remove them
    // from the host now. Skip while a sign-in retry is pending: finishSignIn
    // resends the same @path references, so the files must survive until then
    // (cancelSignIn cleans them up if the user bails instead).
    if (pendingAttachmentCleanupTurnId && !authNeeded) {
      void cleanupUploadedTurn(pendingAttachmentCleanupTurnId);
    }
    void scrollToEnd();
  }

  // The composer's action button morphs on this: empty → mic, content → send.
  const composerHasContent = $derived(input.trim() !== "" || attachments.length > 0);

  /** Toggle dictation for the composer. The shared controller handles every
   *  transition — arming/cancel, handoff from the command gate, failure
   *  toasts, and the OS-confirmed flip to live. */
  function runDictation() {
    micPressGuard = false; // click arrived; blur guard no longer needed
    micSession.toggle(COMPOSER_MIC, composerMicHooks);
  }

  async function send() {
    // Sending outranks polishing: cancel any rewrite still in flight so the
    // words as spoken go out now (and drop a stale Undo affordance).
    dictationRewriter.cancel();
    const text = input.trim();
    const atts = attachments;
    if ((!text && atts.length === 0) || busy) return;
    // Ollama selected but no model pulled — don't fire a request that 400s; the
    // composer shows a pull hint instead.
    if (ollamaNeedsModel) return;
    // Slash command (no attachments) → handle locally if recognised; an unknown
    // slash falls through and is sent to the CLI, which owns custom commands.
    if (text.startsWith("/") && atts.length === 0 && handleSlash(text)) {
      input = "";
      slashDismissed = false;
      return;
    }
    clearAgentError("upload");

    // Upload any attachments to the host first; reference their paths in the turn
    // so the official CLI reads them with its own tools. Abort the send if an
    // upload fails (don't send a turn that points at missing files).
    let remotePaths: string[] = [];
    if (atts.length > 0) {
      busy = true;
      const turnId = crypto.randomUUID();
      try {
        for (const a of atts) {
          const path =
            a.kind === "bytes"
              ? await agentUploadBytes(connectionId, turnId, a.name, a.dataBase64 ?? "")
              : await agentUploadPath(connectionId, turnId, a.name, a.localPath ?? "");
          remotePaths.push(path);
        }
      } catch (e) {
        setAgentError("upload", errText(e, "Couldn't upload an attachment."));
        busy = false;
        return;
      }
      busy = false;
      // A prior turn's staging only stays pending while a sign-in retry might
      // resend its @paths; a new send supersedes that retry — clear it now,
      // then track this turn's staging for cleanup once the agent consumes it.
      if (pendingAttachmentCleanupTurnId) void cleanupUploadedTurn(pendingAttachmentCleanupTurnId);
      pendingAttachmentCleanupTurnId = turnId;
    }

    input = "";
    attachments = [];
    pendingCommand = null;
    const modelText = buildPrompt(text, remotePaths);
    const turn: Turn =
      remotePaths.length > 0
        ? {
            role: "user",
            content: modelText,
            display: text || "(attachments only)",
            attachments: atts.map((a) => a.name),
          }
        : { role: "user", content: text };
    turns = [...turns, turn];
    persistActive();
    void scrollToEnd(true); // jump to the user's new message
    void runTurn();
  }

  // Approve + run the proposed command, feed its output back, continue the loop.
  async function approveRun() {
    // What the user approved is what runs: a rewrite landing after this
    // click must never change the command, so kill any in-flight polish.
    commandRewriter.cancel();
    micSession.release(COMMAND_MIC);
    const cmd = (pendingCommand ?? "").trim();
    if (!cmd || busy) return;
    pendingCommand = null;
    busy = true;
    try {
      const result = await agentRun(connectionId, cmd, chatCwd);
      const output = formatResult(result.stdout, result.stderr, result.exitCode);
      turns = [
        ...turns,
        { role: "user", content: `Output of \`${cmd}\`:\n\n\`\`\`\n${output}\n\`\`\`` },
      ];
      void scrollToEnd(true);
      await runTurn();
    } catch {
      // agentRun uses invokeQuiet; surface inline rather than a toast.
      turns = [...turns, { role: "user", content: `Running \`${cmd}\` failed.` }];
      busy = false;
    }
  }

  function skipRun() {
    commandRewriter.cancel();
    micSession.release(COMMAND_MIC);
    pendingCommand = null;
  }

  /** Stop a running turn (Stop button / Escape). The backend closes the stream
      channel so the remote model/CLI exits; the awaited runTurn then resolves
      with whatever streamed, committing the partial reply. */
  function abortTurn() {
    if (!busy) return;
    agentAbort(connectionId);
  }

  function reset() {
    // The dropped transcript can never resend its @paths — clear any staging
    // left pending by an unresolved sign-in.
    if (pendingAttachmentCleanupTurnId) void cleanupUploadedTurn(pendingAttachmentCleanupTurnId);
    turns = [];
    pendingCommand = null;
    streaming = "";
    streamingTools = [];
    streamingReasoning = "";
    todos = [];
    authNeeded = null;
    clearAgentError();
    editingIndex = null;
    editText = "";
    atBottom = true;
    // Start a fresh CLI conversation (drop the --resume thread).
    sessionId = null;
    persistActive();
  }

  // --- Thread management ---

  /** Clear the transient (non-persisted) UI state — used when switching threads
      so streaming buffers, attachments, sign-in, etc. don't bleed across. */
  function resetTransient() {
    input = "";
    attachments = [];
    pendingCommand = null;
    streaming = "";
    streamingReasoning = "";
    streamingTools = [];
    authNeeded = null;
    clearAgentError();
    confirmDeleteId = null;
    editingIndex = null;
    editText = "";
    atBottom = true;
    planOpen = true;
    if (signIn) cancelSignIn();
    // Switching threads abandons any sign-in retry, so staging left pending by
    // an auth-failed turn will never be resent — clear it from the host.
    if (pendingAttachmentCleanupTurnId) void cleanupUploadedTurn(pendingAttachmentCleanupTurnId);
  }

  /** Snapshot the active thread's working state back into `threads` + storage.
      Called after every durable change (a turn, model/mode/cwd/provider edit). */
  function persistActive() {
    const idx = threads.findIndex((t) => t.id === activeThreadId);
    if (idx === -1) return;
    const snap: AgentThread = {
      ...threads[idx],
      provider,
      model,
      cliModel,
      chatMode,
      cwd: chatCwd,
      sessionId,
      turns,
      todos,
      lastModified: Date.now(),
    };
    snap.title = threadTitle(snap);
    threads = threads.map((t, i) => (i === idx ? snap : t));
    saveThreadStore(connectionId, threads, activeThreadId);
  }

  /** Load a thread's persisted state into the working variables. */
  function applyThread(t: AgentThread) {
    activeThreadId = t.id;
    provider = t.provider;
    model = t.model;
    cliModel = t.cliModel;
    chatMode = t.chatMode;
    chatCwd = t.cwd;
    sessionId = t.sessionId;
    turns = t.turns;
    todos = t.todos;
    resetTransient();
  }

  /** Create a new thread (optionally for a different brain) and switch to it,
      inheriting the current model/mode/cwd unless the brain changes. */
  function newThread(nextProvider?: AgentProvider) {
    if (busy) return;
    persistActive();
    const p = nextProvider ?? provider;
    if (nextProvider) agentProviderPref.set(connectionId, nextProvider);
    const t: AgentThread = {
      id: crypto.randomUUID(),
      title: "New chat",
      provider: p,
      model: nextProvider ? (p === "ollama" ? (info?.ollamaModels[0] ?? "") : "") : model,
      cliModel: nextProvider ? null : cliModel,
      chatMode,
      cwd: chatCwd,
      sessionId: null,
      turns: [],
      todos: [],
      lastModified: Date.now(),
    };
    threads = [t, ...threads];
    applyThread(t);
    saveThreadStore(connectionId, threads, activeThreadId);
  }

  /** Switch to an existing thread, saving the current one first. */
  function switchThread(id: string) {
    if (id === activeThreadId || busy) return;
    persistActive();
    const t = threads.find((x) => x.id === id);
    if (!t) return;
    applyThread(t);
    void scrollToEnd(true);
  }

  /** Delete a thread. Deleting the active one falls back to the most recent
      remaining thread, or seeds a fresh one if it was the last. */
  function deleteThread(id: string) {
    if (busy) return;
    const remaining = threads.filter((t) => t.id !== id);
    if (id === activeThreadId) {
      threads = remaining;
      if (remaining.length === 0) {
        newThread();
        return;
      }
      applyThread([...remaining].sort((a, b) => b.lastModified - a.lastModified)[0]);
    } else {
      threads = remaining;
    }
    saveThreadStore(connectionId, threads, activeThreadId);
  }

  /** Duplicate a thread (Void's DuplicateButton) as a fresh CLI session. */
  function duplicateThread(id: string) {
    if (busy) return;
    persistActive();
    const src = threads.find((t) => t.id === id);
    if (!src) return;
    const copy: AgentThread = {
      ...src,
      id: crypto.randomUUID(),
      sessionId: null, // a copy starts its own --resume thread
      turns: src.turns.map((t) => ({ ...t })),
      todos: src.todos.map((t) => ({ ...t })),
      lastModified: Date.now(),
    };
    threads = [copy, ...threads];
    applyThread(copy);
    saveThreadStore(connectionId, threads, activeThreadId);
  }

  /** Seed the very first thread from the current working state (initial open,
      no persisted threads yet). */
  function seedInitialThread(p: AgentProvider) {
    const t: AgentThread = {
      id: crypto.randomUUID(),
      title: "New chat",
      provider: p,
      model: p === "ollama" ? (info?.ollamaModels[0] ?? model) : "",
      cliModel: null,
      chatMode,
      cwd: chatCwd,
      sessionId: null,
      turns: [],
      todos: [],
      lastModified: Date.now(),
    };
    threads = [t];
    activeThreadId = t.id;
    saveThreadStore(connectionId, threads, activeThreadId);
  }

  // --- Edit & re-run a past user message (Void's UserMessageComponent edit) ---

  /** Enter edit mode for the user message at `i`. */
  function startEditTurn(i: number) {
    if (busy) return;
    editingIndex = i;
    editText = turns[i].display ?? turns[i].content;
  }

  function cancelEdit() {
    editingIndex = null;
    editText = "";
  }

  /** Re-run from an edited message: drop everything from that message onward,
      resend the new text on a fresh CLI session (the official agents can't
      rewind their own server-side history). Any attachments the original message
      carried are dropped — re-attach if needed. */
  function submitEdit() {
    if (editingIndex === null || busy) return;
    const text = editText.trim();
    if (!text) return;
    const i = editingIndex;
    editingIndex = null;
    editText = "";
    turns = turns.slice(0, i);
    sessionId = null;
    pendingCommand = null;
    turns = [...turns, { role: "user", content: text }];
    persistActive();
    void scrollToEnd(true);
    void runTurn();
  }

  /** Compact relative time for the history list (Void's date column). */
  function relTime(ms: number): string {
    const d = new Date(ms);
    const now = new Date();
    const sameDay = d.toDateString() === now.toDateString();
    if (sameDay) return d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
    const yesterday = new Date(now);
    yesterday.setDate(now.getDate() - 1);
    if (d.toDateString() === yesterday.toDateString()) return "Yesterday";
    return d.toLocaleDateString([], { month: "short", day: "numeric" });
  }

  function onComposerKeydown(e: KeyboardEvent) {
    // ⌘Z reverts a just-applied dictation rewrite: the splice bypassed the
    // textarea's native undo stack, so route the gesture to the rewriter
    // while its undo is armed (then fall back to the browser's own undo).
    if ((e.metaKey || e.ctrlKey) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "z" && dictationRewriter.canUndo) {
      e.preventDefault();
      dictationRewriter.undo();
      return;
    }
    // Menu navigation (commands or model picker) takes precedence while open.
    if (menuOpen) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        slashIndex = (slashIndex + 1) % menuItems.length;
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        slashIndex = (slashIndex - 1 + menuItems.length) % menuItems.length;
        return;
      }
      if (e.key === "PageDown") {
        e.preventDefault();
        slashIndex = Math.min(menuItems.length - 1, slashIndex + 5);
        return;
      }
      if (e.key === "PageUp") {
        e.preventDefault();
        slashIndex = Math.max(0, slashIndex - 5);
        return;
      }
      if (e.key === "Tab") {
        e.preventDefault();
        selectMenuItem(menuItems[slashIndex]);
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        slashDismissed = true;
        return;
      }
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        selectMenuItem(menuItems[slashIndex]);
        return;
      }
    }
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      void send();
    }
  }

  function withinComposer(pos?: { x: number; y: number }): boolean {
    if (!pos || !composerEl) return false;
    const r = composerEl.getBoundingClientRect();
    return pos.x >= r.left && pos.x <= r.right && pos.y >= r.top && pos.y <= r.bottom;
  }

  // OS file drop onto the composer → stage as path attachments (scoped to the
  // composer's box so it doesn't collide with the file browser's drop target).
  $effect(() => {
    if (!browser) return;
    let unlisten: (() => void) | null = null;
    void (async () => {
      const { getCurrentWebview } = await import("@tauri-apps/api/webview");
      unlisten = await getCurrentWebview().onDragDropEvent((event) => {
        const t = event.payload.type;
        const pos = (event.payload as { position?: { x: number; y: number } }).position;
        if (t === "drop") {
          dragOver = false;
          if (!withinComposer(pos)) return;
          const paths = (event.payload as { paths?: string[] }).paths ?? [];
          for (const p of paths) addPathAttachment(p);
        } else if (t === "leave") {
          dragOver = false;
        } else if (t === "enter" || t === "over") {
          dragOver = withinComposer(pos);
        }
      });
    })();
    return () => unlisten?.();
  });

  // Escape ×2 stops a running turn (Codex / Claude Code convention: the first
  // press arms a hint, a second within the window interrupts). The composer
  // textarea is disabled while busy, so listen at the window — but stand down
  // while a menu or the sign-in flow is open, which own Escape themselves.
  // Teardown (turn ends or unmount) disarms, so the hint never outlives the run.
  $effect(() => {
    if (!browser || !busy) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || menuOpen || signIn) return;
      e.preventDefault();
      if (escArmed) {
        escArmed = false;
        if (escArmTimer) clearTimeout(escArmTimer);
        escArmTimer = null;
        abortTurn();
        return;
      }
      escArmed = true;
      if (escArmTimer) clearTimeout(escArmTimer);
      escArmTimer = setTimeout(() => (escArmed = false), 1500);
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
      if (escArmTimer) clearTimeout(escArmTimer);
      escArmTimer = null;
      escArmed = false;
    };
  });

  onMount(() => {
    void (async () => {
      if (browser) {
        const savedCwd = localStorage.getItem(cwdKey(connectionId));
        if (savedCwd) chatCwd = savedCwd;
      }
      try {
        const detected = await openAgent(connectionId, label);
        info = detected;
        const available: AgentProvider[] = [];
        if (detected.hasOllama || detected.ollamaModels.length > 0) available.push("ollama");
        if (detected.hasClaude) available.push("claude");
        if (detected.hasCodex) available.push("codex");
        if (available.length === 0) {
          status = "noagents";
          return;
        }
        if (detected.ollamaModels.length > 0) model = detected.ollamaModels[0];

        // Restore persisted threads (dropping any whose brain the host no longer
        // offers), else seed the first thread with the best default provider.
        const stored = await loadThreadStore(connectionId);
        const valid = stored?.threads.filter((t) => available.includes(t.provider)) ?? [];
        if (valid.length > 0 && stored) {
          threads = valid;
          applyThread(valid.find((t) => t.id === stored.activeId) ?? valid[0]);
        } else {
          // Prefer a provider that's ready to use — don't land on Ollama with no
          // model when Claude/Codex (or Ollama-with-a-model) is available.
          const saved = agentProviderPref.get(connectionId);
          const usable = available.filter(
            (p) => p !== "ollama" || detected.ollamaModels.length > 0,
          );
          provider = saved && available.includes(saved) ? saved : (usable[0] ?? available[0]);
          seedInitialThread(provider);
        }
        status = "ready";
      } catch (e) {
        status = "error";
        setAgentError("connection", errText(e, "Couldn't reach this host."));
      }
    })();
  });

  onDestroy(() => {
    if (signIn?.ptyId) ptyClose(signIn.ptyId);
    agentClose(connectionId);
  });
</script>

<div class="flex h-full min-h-0 flex-col bg-surface">
  {#snippet provIcon(p: AgentProvider, size: number)}
    {@const img = PROVIDER_IMG[p]}
    {#if img}
      <img src={img} alt="" width={size} height={size} class="shrink-0 rounded-sm" />
    {:else}
      <Icon name="server" {size} class="shrink-0 text-fg-muted" />
    {/if}
  {/snippet}
  {#snippet providerGlyph(p: AgentProvider)}{@render provIcon(p, 14)}{/snippet}
  {#snippet modeGlyph(m: ChatMode)}<Icon name={MODE_ICON[m]} size={14} class="shrink-0" />{/snippet}
  <!-- Header — the brain/model/mode selectors now live in the composer toolbar
       (Void's VoidChatArea layout), so the header is just the title + close. -->
  <header class="flex items-center gap-2 border-b border-border/60 px-6 py-3">
    <Icon name="bot" size={15} class="shrink-0 text-fg-muted" />
    <span class="text-[12.5px] font-medium text-fg">Agent</span>
    {#if status === "ready"}
      <!-- New chat + history (Void's thread selector). -->
      <button
        type="button"
        onclick={() => newThread()}
        disabled={busy}
        title="New chat"
        aria-label="New chat"
        class="ml-1 grid h-7 w-7 place-items-center rounded-md text-fg-muted hover:bg-surface-2 hover:text-fg disabled:opacity-50"
      >
        <Icon name="plus" size={15} />
      </button>
      <Popover title="Chats" align="left" width="19rem">
        {#snippet trigger(toggle: () => void, isOpen: boolean)}
          <button
            type="button"
            onclick={toggle}
            aria-pressed={isOpen}
            disabled={busy}
            title="Chat history"
            class="inline-flex h-7 items-center gap-1.5 rounded-md px-2 text-[11.5px] text-fg-muted hover:bg-surface-2 hover:text-fg disabled:opacity-50"
          >
            <Icon name="list" size={14} class="shrink-0" />
            <span>{threads.length} chat{threads.length === 1 ? "" : "s"}</span>
          </button>
        {/snippet}
        {#snippet children(close: () => void)}
          <div class="max-h-[60vh] space-y-0.5 overflow-auto">
            {#each sortedThreads as t (t.id)}
              <div
                class="group flex items-center gap-1 rounded-md px-1.5 py-1 {t.id === activeThreadId
                  ? 'bg-surface-2'
                  : 'hover:bg-surface-2/60'}"
              >
                <button
                  type="button"
                  onclick={() => {
                    switchThread(t.id);
                    close();
                  }}
                  class="flex min-w-0 flex-1 flex-col text-left"
                >
                  <span class="truncate text-[12px] text-fg">{t.title}</span>
                  <span class="truncate text-[10.5px] text-fg-subtle">
                    {PROVIDER_LABELS[t.provider]} · {t.turns.filter((x) => x.role === "user")
                      .length} msg · {relTime(t.lastModified)}
                  </span>
                </button>
                {#if confirmDeleteId === t.id}
                  <button
                    type="button"
                    onclick={() => (confirmDeleteId = null)}
                    title="Cancel"
                    class="shrink-0 rounded p-1 text-fg-muted hover:text-fg"
                  >
                    <Icon name="x" size={12} />
                  </button>
                  <button
                    type="button"
                    onclick={() => {
                      deleteThread(t.id);
                      confirmDeleteId = null;
                    }}
                    title="Confirm delete"
                    class="shrink-0 rounded p-1 text-status-crashed hover:brightness-110"
                  >
                    <Icon name="check" size={12} />
                  </button>
                {:else}
                  <button
                    type="button"
                    onclick={() => duplicateThread(t.id)}
                    disabled={busy}
                    title="Duplicate"
                    class="shrink-0 rounded p-1 text-fg-subtle opacity-0 hover:text-fg group-hover:opacity-100 disabled:opacity-0"
                  >
                    <Icon name="copy" size={12} />
                  </button>
                  <button
                    type="button"
                    onclick={() => (confirmDeleteId = t.id)}
                    disabled={busy}
                    title="Delete"
                    class="shrink-0 rounded p-1 text-fg-subtle opacity-0 hover:text-status-crashed group-hover:opacity-100 disabled:opacity-0"
                  >
                    <Icon name="trash-2" size={12} />
                  </button>
                {/if}
              </div>
            {/each}
          </div>
        {/snippet}
      </Popover>
    {/if}
    <div class="flex-1"></div>
    <!-- Working directory: where the agent runs, so attachments / relative paths
         resolve against the project you're working on. `~` = host home. -->
    <Popover title="Working directory" align="right" width="20rem">
      {#snippet trigger(toggle: () => void, isOpen: boolean)}
        <button
          type="button"
          onclick={toggle}
          aria-pressed={isOpen}
          title="Agent working directory — where files are placed and commands run"
          class="inline-flex h-7 max-w-[190px] items-center gap-1.5 rounded-md border border-border bg-surface px-2 text-[11.5px] text-fg-muted hover:bg-surface-2 hover:text-fg"
        >
          <Icon name="folder" size={13} class="shrink-0" />
          <span class="truncate font-mono">{chatCwd}</span>
        </button>
      {/snippet}
      {#snippet children(close: () => void)}
        <div class="space-y-2">
          <p class="text-[11.5px] leading-relaxed text-fg-muted">
            Where the agent runs. Files you attach can be placed here with
            project-relative paths (e.g.
            <code class="rounded bg-surface-2 px-1 font-mono text-[11px]">static/images</code>).
            <code class="rounded bg-surface-2 px-1 font-mono text-[11px]">~</code> is the host home.
          </p>
          <input
            value={chatCwd}
            onkeydown={(e) => {
              if (e.key === "Enter") {
                setCwd((e.currentTarget as HTMLInputElement).value);
                close();
              }
            }}
            onblur={(e) => setCwd((e.currentTarget as HTMLInputElement).value)}
            spellcheck="false"
            placeholder="~ or /var/www/site"
            class="h-8 w-full rounded-md border border-border bg-surface px-2.5 font-mono text-[12px] text-fg outline-none focus:border-accent"
          />
        </div>
      {/snippet}
    </Popover>
    {#if onClose}
      <button
        type="button"
        onclick={onClose}
        class="grid h-8 w-8 shrink-0 place-items-center rounded-md text-fg-muted hover:bg-surface-2 hover:text-fg"
        aria-label="Close agent panel"
        title="Close agent panel"
      >
        <Icon name="x" size={15} />
      </button>
    {/if}
  </header>

  {#if status === "connecting"}
    <div class="flex flex-1 items-center justify-center text-[12px] text-fg-subtle">
      <Icon name="refresh-cw" size={14} class="mr-2 animate-spin" /> Connecting…
    </div>
  {:else if status === "error"}
    <div class="flex flex-1 items-center justify-center p-6">
      <div class="max-w-sm rounded-lg border border-status-crashed/40 bg-status-crashed/10 p-4 text-center">
        <Icon name="circle-alert" size={18} class="mx-auto text-status-crashed" />
        <p class="mt-2 text-[12.5px] text-fg">
          {agentError?.slot === "connection" ? agentError.message : "Couldn't reach this host."}
        </p>
      </div>
    </div>
  {:else if status === "noagents"}
    <NoAgentBackend {info} />
  {:else}
    <!-- Tool-call rows (Void's ToolHeaderWrapper): one collapsible row per tool
         the agent ran. A still-running tool shows a spinner and no dropdown; a
         finished one expands to its result. -->
    {#snippet toolRows(tools: ToolActivity[])}
      <div class="flex flex-col gap-1">
        {#each tools as tool, ti (ti)}
          {@const titles = friendlyTool(tool.name)}
          {#if tool.result !== undefined}
            <ToolHeader title={titles[0]} desc1={tool.summary} isError={tool.isError}>
              <div
                class="max-h-44 overflow-y-auto whitespace-pre-wrap px-1 font-mono text-[11px] leading-relaxed text-fg-muted"
              >
                {tool.result}
              </div>
            </ToolHeader>
          {:else}
            <ToolHeader title={titles[1]} desc1={tool.summary} loading />
          {/if}
        {/each}
      </div>
    {/snippet}

    <!-- Reasoning row (Void's ReasoningWrapper): the agent's thinking, rendered
         as small markdown inside a collapsible "Reasoning" header. Opens while
         it's still streaming, collapses once the reply lands. -->
    {#snippet reasoningRow(reasoning: string, live: boolean)}
      <ToolHeader title="Reasoning" loading={live} defaultOpen={live}>
        <Markdown small source={reasoning} />
      </ToolHeader>
    {/snippet}

    <!-- Matched-character highlight for the completion menu (Lapce-style): the
         fuzzy-matched chars of `text` (at `indices`) render in the accent. -->
    {#snippet hl(text: string, indices: number[])}
      {#each text.split("") as ch, ci (ci)}<span
          class={indices.includes(ci) ? "font-semibold text-accent" : ""}>{ch}</span>{/each}
    {/snippet}

    <!-- Agent plan (collapsible) — the agent's own TodoWrite / todo_list,
         parsed from the stream we already read (no extra tokens). Pinned above
         the transcript so progress stays visible while it scrolls. -->
    {#if todos.length > 0}
      <div class="border-b border-border/60 bg-surface-2/30 px-6 py-2">
        <div class="mx-auto max-w-2xl">
          <button
            type="button"
            onclick={() => (planOpen = !planOpen)}
            class="flex w-full items-center gap-1.5 text-[11px] font-medium uppercase tracking-wide text-fg-subtle hover:text-fg"
          >
            <Icon name={planOpen ? "chevron-down" : "chevron-right"} size={12} />
            <Icon name="square-kanban" size={12} />
            <span>Plan</span>
            <span class="font-normal normal-case text-fg-muted">{todoDone}/{todos.length} done</span>
            {#if busy}
              <Icon name="refresh-cw" size={10} class="animate-spin text-fg-muted" />
            {/if}
          </button>
          {#if planOpen}
            <ul class="mt-1.5 space-y-1">
              {#each todos as todo, ti (ti)}
                <li class="flex items-start gap-1.5 text-[12px] leading-snug">
                  {#if todo.status === "completed"}
                    <Icon name="circle-check" size={13} class="mt-px shrink-0 text-status-running" />
                  {:else if todo.status === "in_progress"}
                    <Icon name="circle-dot" size={13} class="mt-px shrink-0 text-accent" />
                  {:else}
                    <span class="mt-0.5 h-3 w-3 shrink-0 rounded-full border border-fg-subtle/60"></span>
                  {/if}
                  <span class={todo.status === "completed" ? "text-fg-muted line-through" : "text-fg"}>
                    {todo.text}
                  </span>
                </li>
              {/each}
            </ul>
          {/if}
        </div>
      </div>
    {/if}

    <!-- Transcript (Void layout): user messages right-aligned in a small
         bubble; assistant turns full-width with reasoning + tool rows above the
         markdown reply. The live turn streams inline at the bottom. -->
    <div class="relative min-h-0 flex-1">
      <div bind:this={scroller} onscroll={onScroll} class="h-full overflow-y-auto px-6 py-4">
        <div class="mx-auto max-w-2xl space-y-4">
          {#if turns.length === 0 && !busy}
            {@const others = sortedThreads.filter(
              (t) => t.id !== activeThreadId && t.turns.length > 0,
            )}
            <!-- Onboarding empty state (Void's landing-page suggestions +
                 previous-threads list). -->
            <div class="flex flex-col items-center gap-3 pt-10 text-center">
              <Icon name="bot" size={22} class="text-fg-subtle" />
              <p class="text-[12.5px] text-fg-muted">
                Ask {PROVIDER_LABELS[provider]} anything about this host.
              </p>
              <div class="flex w-full max-w-sm flex-col items-stretch gap-1.5">
                {#each SUGGESTIONS[provider] as s (s)}
                  <button
                    type="button"
                    onclick={() => {
                      input = s;
                      void send();
                    }}
                    class="rounded-md border border-border bg-surface px-3 py-1.5 text-[12px] text-fg-muted hover:bg-surface-2 hover:text-fg"
                  >
                    {s}
                  </button>
                {/each}
              </div>
              {#if others.length > 0}
                <div class="mt-4 w-full max-w-sm">
                  <p
                    class="mb-1.5 text-left text-[10.5px] font-medium uppercase tracking-wide text-fg-subtle"
                  >
                    Previous chats
                  </p>
                  <div class="flex flex-col gap-1">
                    {#each others.slice(0, 5) as t (t.id)}
                      <button
                        type="button"
                        onclick={() => switchThread(t.id)}
                        class="flex items-center justify-between gap-2 rounded-md border border-border/60 bg-surface px-2.5 py-1.5 text-left hover:bg-surface-2"
                      >
                        <span class="truncate text-[12px] text-fg">{t.title}</span>
                        <span class="shrink-0 text-[10.5px] text-fg-subtle">
                          {relTime(t.lastModified)}
                        </span>
                      </button>
                    {/each}
                  </div>
                </div>
              {/if}
            </div>
          {/if}
          {#each turns as t, i (i)}
          {#if t.role === "user"}
            {#if editingIndex === i}
              <!-- Edit mode (Void's UserMessageComponent edit): re-runs from here. -->
              <div class="ml-auto w-full max-w-[90%]">
                <textarea
                  bind:value={editText}
                  onkeydown={(e) => {
                    if (e.key === "Enter" && !e.shiftKey) {
                      e.preventDefault();
                      submitEdit();
                    } else if (e.key === "Escape") {
                      e.preventDefault();
                      cancelEdit();
                    }
                  }}
                  rows="2"
                  spellcheck="false"
                  class="w-full resize-y rounded-lg border border-accent bg-surface px-2.5 py-2 text-[12.5px] leading-relaxed text-fg outline-none"
                ></textarea>
                <div class="mt-1.5 flex items-center justify-end gap-2">
                  <span class="mr-auto text-[10.5px] text-fg-subtle">Re-runs from this message</span>
                  <button
                    type="button"
                    onclick={cancelEdit}
                    class="rounded border border-border bg-surface px-2 py-1 text-[12px] text-fg hover:bg-surface-2"
                  >
                    Cancel
                  </button>
                  <button
                    type="button"
                    onclick={submitEdit}
                    disabled={!editText.trim()}
                    class="rounded bg-accent px-2 py-1 text-[12px] font-medium text-on-accent hover:brightness-110 disabled:opacity-50"
                  >
                    Send
                  </button>
                </div>
              </div>
            {:else}
              <div
                class="group relative ml-auto flex w-fit max-w-[85%] flex-col rounded-lg bg-surface p-2 text-[12.5px] leading-relaxed text-fg"
              >
                {#if t.attachments && t.attachments.length > 0}
                  <div class="mb-1.5 flex flex-wrap gap-1">
                    {#each t.attachments as name, ai (ai)}
                      <span
                        class="inline-flex items-center gap-1 rounded bg-surface-2/70 px-1.5 py-0.5 text-[10.5px] text-fg-muted"
                      >
                        <Icon name="paperclip" size={10} />
                        <span class="max-w-[140px] truncate font-mono">{name}</span>
                      </span>
                    {/each}
                  </div>
                {/if}
                <span class="whitespace-pre-wrap break-words px-0.5">{t.display ?? t.content}</span>
                {#if !busy}
                  <button
                    type="button"
                    onclick={() => startEditTurn(i)}
                    title="Edit & resend"
                    aria-label="Edit message"
                    class="absolute -left-1 top-1 grid h-6 w-6 -translate-x-full place-items-center rounded-md border border-border bg-surface text-fg-muted opacity-0 transition-opacity duration-200 hover:text-fg group-hover:opacity-100"
                  >
                    <Icon name="pencil" size={12} />
                  </button>
                {/if}
              </div>
            {/if}
          {:else}
            <div class="w-full space-y-1.5">
              {#if t.reasoning}
                {@render reasoningRow(t.reasoning, false)}
              {/if}
              {#if t.tools && t.tools.length > 0}
                {@render toolRows(t.tools)}
              {/if}
              {#if t.content}
                <div class="px-0.5"><Markdown source={t.content} /></div>
              {/if}
            </div>
          {/if}
        {/each}

        <!-- Live streaming assistant turn — reasoning + tools + prose inline. -->
        {#if busy || streaming || streamingReasoning || streamingTools.length > 0}
          <div class="w-full space-y-1.5">
            {#if streamingReasoning}
              {@render reasoningRow(streamingReasoning, true)}
            {/if}
            {#if streamingTools.length > 0}
              {@render toolRows(streamingTools)}
            {/if}
            {#if streaming}
              <div class="px-0.5"><Markdown source={streaming} /></div>
              {#if busy}
                <!-- Void's trailing "working" ellipsis after streamed text. -->
                <div class="px-0.5 text-[12.5px] text-fg opacity-50"><IconLoading /></div>
              {/if}
            {:else if !streamingReasoning && streamingTools.length === 0}
              <!-- Void's "thinking" indicator: a cycling ellipsis at low opacity. -->
              <div class="px-0.5 text-[12.5px] text-fg opacity-50"><IconLoading /></div>
            {/if}
          </div>
        {/if}
        </div>
      </div>
      <!-- Scroll-to-bottom button: appears once the user scrolls up off the
           bottom (Void's ScrollToBottomContainer affordance). -->
      {#if !atBottom}
        <button
          type="button"
          onclick={() => scrollToEnd(true)}
          title="Scroll to bottom"
          aria-label="Scroll to bottom"
          class="absolute bottom-3 right-4 grid h-7 w-7 place-items-center rounded-full border border-border bg-surface text-fg-muted shadow-md hover:bg-surface-2 hover:text-fg"
        >
          <Icon name="chevron-down" size={15} />
        </button>
      {/if}
    </div>

    <!-- Approval gate — Void's tool-request card (ToolHeaderWrapper look) with its
         Approve / Cancel buttons (ToolRequestAcceptRejectButtons). The command
         stays editable (our extra) before it runs in the agent's working dir. -->
    {#if pendingCommand}
      <div class="border-t border-border/60 bg-surface-2/30 px-6 py-3">
        <div class="mx-auto max-w-2xl">
          <div class="overflow-hidden rounded border border-border bg-surface-2">
            <div class="flex items-center gap-1.5 px-2 py-1 text-[12px] text-fg-muted">
              <Icon name="terminal" size={13} class="shrink-0" />
              <span>Run command</span>
              <span class="truncate text-[11px] italic text-fg-subtle">
                review before it runs on the host
              </span>
              <span class="ml-auto inline-flex shrink-0 items-center gap-1.5">
                <!-- Smart Dictation status for voice edits to the command. -->
                <DictationRewriteChip rewriter={commandRewriter} />
                <button
                  type="button"
                  onpointerdown={guardCommandMicPress}
                  onclick={runCommandDictation}
                  title={commandDictating ? "Stop dictation" : "Dictate an edit (voice to text)"}
                  aria-label={commandDictating ? "Stop voice dictation" : "Dictate an edit to the command"}
                  aria-pressed={commandDictating}
                  class="grid h-6 w-6 shrink-0 place-items-center rounded-full transition-colors
                    {commandDictating
                      ? 'bg-red-500 text-white shadow-lg shadow-red-500/40 hover:bg-red-600'
                      : 'text-fg-muted hover:bg-surface hover:text-fg'}"
                >
                  {#if commandLive}
                    <Icon name="square" size={11} class="fill-current" />
                  {:else}
                    <Icon name="mic" size={13} class={commandDictating ? "animate-pulse" : ""} />
                  {/if}
                </button>
              </span>
            </div>
            <textarea
              bind:this={commandTextarea}
              bind:value={pendingCommand}
              onblur={onCommandBlur}
              onkeydown={(e) => {
                // ⌘Z reverts a just-applied dictation rewrite (see composer).
                if ((e.metaKey || e.ctrlKey) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "z" && commandRewriter.canUndo) {
                  e.preventDefault();
                  commandRewriter.undo();
                }
              }}
              rows="2"
              spellcheck="false"
              class="w-full resize-y border-t border-border bg-surface px-2.5 py-2 font-mono text-[12px] text-fg outline-none focus:border-accent"
            ></textarea>
          </div>
          <div class="mx-0.5 mt-2 flex items-center gap-2">
            <button
              type="button"
              onclick={approveRun}
              disabled={busy}
              class="rounded bg-accent px-2 py-1 text-[13px] font-medium text-on-accent hover:brightness-110 disabled:opacity-50"
            >
              Approve
            </button>
            <button
              type="button"
              onclick={skipRun}
              disabled={busy}
              class="rounded border border-border bg-surface px-2 py-1 text-[13px] font-medium text-fg hover:bg-surface-2 disabled:opacity-50"
            >
              Cancel
            </button>
          </div>
        </div>
      </div>
    {/if}

    <!-- Sign-in CTA: shown when the last CLI turn failed for auth -->
    {#if authNeeded && !signIn}
      <CliAuthPrompt
        providerLabel={PROVIDER_LABELS[authNeeded]}
        onSignIn={() => startSignIn(authNeeded!)}
      />
    {/if}

    <!-- Error card (Void's ErrorDisplay): the current turn failure, dismissible.
         Auth failures use the sign-in CTA above instead, so this is for
         everything else. -->
    {#if agentError?.slot === "turn"}
      <AgentErrorBanner message={agentError.message} onDismiss={() => clearAgentError("turn")} />
    {/if}

    <!-- Sign-in flow: the official setup-token / login is driven over a wide PTY
         so the OAuth URL isn't wrapped; we extract it, auto-open it, show it as a
         real clickable link, and send the pasted code back to the CLI. -->
    {#if signIn}
      {@const s = signIn}
      <div class="border-t border-border/60 bg-surface-2/40 px-6 py-3">
        <div class="mx-auto max-w-2xl">
          <div class="mb-2 flex items-center gap-2">
            <Icon name="shield-check" size={14} class="text-fg-muted" />
            <span class="text-[12.5px] font-semibold text-fg">
              Sign in to {PROVIDER_LABELS[s.provider]} on this host
            </span>
            <div class="flex-1"></div>
            <button
              type="button"
              onclick={cancelSignIn}
              class="rounded-md p-1 text-fg-muted hover:bg-surface-2 hover:text-fg"
              aria-label="Close sign-in"
            >
              <Icon name="x" size={14} />
            </button>
          </div>

          {#if s.stage === "error"}
            <p class="text-[12px] text-status-crashed">{s.error}</p>
          {:else if s.url}
            <p class="mb-2 text-[12px] text-fg-muted leading-relaxed">
              Your browser was opened to the authorization page. Approve access, then paste the code
              it gives you below. Auth stays on the host.
            </p>
            <div class="flex flex-wrap items-center gap-2">
              <button
                type="button"
                onclick={() => openUrl(s.url!)}
                class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md text-[12px] font-medium bg-accent text-on-accent hover:brightness-110"
              >
                <Icon name="external-link" size={13} /> Open authorization page
              </button>
              <button
                type="button"
                onclick={() => navigator.clipboard.writeText(s.url ?? "")}
                class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md text-[12px] font-medium border border-border text-fg-muted hover:text-fg hover:bg-surface-2"
              >
                <Icon name="paperclip" size={12} /> Copy link
              </button>
            </div>
            <!-- The full URL, selectable + clickable (one HTML link, never wrapped
                 into separate clickable fragments like a terminal). -->
            <button
              type="button"
              onclick={() => openUrl(s.url!)}
              class="mt-2 block w-full break-all rounded-md border border-border bg-surface px-2.5 py-1.5 text-left font-mono text-[11px] text-accent hover:underline"
            >
              {s.url}
            </button>
            <div class="mt-2 flex items-end gap-2">
              <input
                bind:value={signInCode}
                onkeydown={(e) => e.key === "Enter" && (e.preventDefault(), submitSignInCode())}
                placeholder="Paste the code from your browser"
                spellcheck="false"
                class="h-8 flex-1 rounded-md border border-border bg-surface px-2.5 font-mono text-[12px] text-fg outline-none focus:border-accent"
              />
              <button
                type="button"
                onclick={submitSignInCode}
                disabled={!signInCode.trim()}
                class="inline-flex h-8 shrink-0 items-center gap-1.5 rounded-md bg-accent px-3 text-[12px] font-medium text-on-accent hover:brightness-110 disabled:opacity-50"
              >
                Submit
              </button>
            </div>
            {#if signInSubmitted}
              <p class="mt-2 flex items-center gap-1.5 text-[11.5px] text-fg-subtle">
                <Icon name="refresh-cw" size={11} class="animate-spin" /> Code sent — finishing
                sign-in… (if nothing happens, re-check the code and Submit again)
              </p>
            {/if}
          {:else}
            <p class="flex items-center gap-2 text-[12px] text-fg-muted">
              <Icon name="refresh-cw" size={13} class="animate-spin" />
              Starting the official sign-in… opening your browser to authorize.
            </p>
          {/if}
        </div>
      </div>
    {/if}

    <!-- Composer -->
    <div class="border-t border-border/60 px-6 py-3">
      {#if escArmed}
        <div class="mx-auto mb-2 max-w-2xl text-[11.5px] text-fg-subtle" aria-live="polite">
          Press <kbd class="rounded border border-border bg-surface-2 px-1 font-mono text-[10.5px]">esc</kbd> again to interrupt
        </div>
      {/if}
      {#if ollamaNeedsModel}
        <OllamaModelHint />
      {/if}
      {#if agentError?.slot === "upload"}
        <div class="mx-auto mb-2 flex max-w-2xl items-center gap-1.5 text-[11.5px] text-status-crashed">
          <Icon name="circle-alert" size={12} /> {agentError.message}
        </div>
      {/if}
      {#if attachments.length > 0}
        <div class="mx-auto mb-2 flex max-w-2xl flex-wrap gap-1.5">
          {#each attachments as a (a.id)}
            <span class="inline-flex items-center gap-1.5 rounded-md border border-border bg-surface-2/60 px-2 py-1 text-[11px] text-fg">
              <Icon name={a.kind === "bytes" ? "image" : "file-text"} size={11} class="shrink-0 text-fg-subtle" />
              <span class="max-w-[160px] truncate font-mono">{a.name}</span>
              {#if a.size !== undefined}
                <span class="text-fg-subtle">{formatBytes(a.size)}</span>
              {/if}
              <button
                type="button"
                onclick={() => removeAttachment(a.id)}
                class="rounded p-0.5 text-fg-subtle hover:bg-surface-2 hover:text-fg"
                aria-label={`Remove ${a.name}`}
              >
                <Icon name="x" size={11} />
              </button>
            </span>
          {/each}
        </div>
      {/if}
      {#if menuOpen}
        <div class="mx-auto mb-2 max-w-2xl overflow-hidden rounded-lg border border-border bg-surface shadow-lg">
          {#if modelMenuOpen}
            <div class="border-b border-border/60 px-3 py-1 text-[10.5px] uppercase tracking-wide text-fg-subtle">
              Model · {PROVIDER_LABELS[provider]}
            </div>
          {/if}
          {#each menuItems as item, i (item.kind + item.name)}
            <button
              type="button"
              onclick={() => selectMenuItem(item)}
              onmouseenter={() => (slashIndex = i)}
              class="flex w-full items-center gap-2 px-3 py-1.5 text-left text-[12px] {i === slashIndex ? 'bg-surface-2' : ''}"
            >
              <!-- Fixed-width kind badge (Lapce's per-kind letter): `/` command,
                   `M` model — so the row's category reads at a glance. -->
              <span
                class="grid h-4 w-4 shrink-0 place-items-center rounded text-[9px] font-bold
                  {item.kind === 'command' ? 'bg-accent/15 text-accent' : 'bg-status-running/15 text-status-running'}"
                aria-hidden="true"
              >
                {item.kind === "command" ? "/" : "M"}
              </span>
              <span class="font-mono text-fg">{#if item.kind === "command"}<span class="text-fg-subtle">/</span>{/if}{@render hl(item.name, item.indices)}</span>
              <span class="flex-1 truncate text-fg-subtle">{item.desc}</span>
              {#if item.kind === "model" && item.desc === "current"}
                <Icon name="check" size={12} class="shrink-0 text-accent" />
              {/if}
            </button>
          {/each}
        </div>
      {/if}
      <!-- Void's VoidChatArea shell: one rounded box holding the textarea and a
           bottom toolbar of [📎][brain ▾][model ▾][mode ▾] … submit. The brain
           (provider) + model + mode selectors live here, not in the header. -->
      <div
        bind:this={composerEl}
        class="mx-auto flex max-w-2xl flex-col gap-1.5 rounded-md border bg-surface p-2 transition-colors
          {dragOver
          ? 'border-accent ring-1 ring-accent'
          : 'border-border focus-within:border-accent hover:border-border-strong'}"
      >
        <textarea
          bind:this={composerTextarea}
          bind:value={input}
          onkeydown={onComposerKeydown}
          oninput={() => {
            slashDismissed = false;
            slashIndex = 0; // reset the highlighted row as the filter changes
          }}
          onblur={onComposerBlur}
          onpaste={onPaste}
          rows="2"
          placeholder={busy ? "Working…" : "Ask the agent…  / for commands"}
          disabled={busy}
          class="max-h-40 min-h-[40px] w-full resize-none bg-transparent px-0.5 py-0.5 text-[12.5px] text-fg outline-none placeholder:text-fg-subtle disabled:opacity-60"
        ></textarea>
        <!-- Bottom toolbar -->
        <div class="flex items-end justify-between gap-1">
          <div class="flex flex-wrap items-center gap-x-2 gap-y-1">
            <button
              type="button"
              onclick={pickFiles}
              disabled={busy}
              class="grid h-6 w-6 shrink-0 place-items-center rounded text-fg-muted hover:bg-surface-2 hover:text-fg disabled:opacity-50"
              aria-label="Attach files"
              title="Attach files (or drag-drop / paste an image)"
            >
              <Icon name="paperclip" size={14} />
            </button>
            <!-- Brain (provider) — keeps our agent selection. -->
            <AgentDropdown
              options={availableProviders}
              selected={provider}
              onChange={selectProvider}
              displayName={(p) => PROVIDER_LABELS[p]}
              optionIcon={providerGlyph}
              disabled={busy || availableProviders.length <= 1}
            />
            <!-- Model — reuses our canonical catalogue + setModel logic. -->
            {#if agentModels.length > 0}
              <AgentDropdown
                options={(modelHasDefault
                  ? [null, ...agentModels.map((m) => m.id)]
                  : agentModels.map((m) => m.id)) as (string | null)[]}
                selected={currentModelId}
                onChange={(id) => setModel(id ?? "default")}
                displayName={(id) =>
                  id ? (agentModels.find((m) => m.id === id)?.name ?? id) : "Default"}
                detail={(id) =>
                  id
                    ? agentModels.find((m) => m.id === id)?.description
                    : `Let ${PROVIDER_LABELS[provider]} choose`}
                bordered
                disabled={busy}
              />
            {/if}
            <!-- Mode (Agent / Gather / Normal) — Void's ChatModeDropdown. Now
                 wired to a real permission posture (MODE_PERMISSION). -->
            <AgentDropdown
              options={CHAT_MODES}
              selected={chatMode}
              onChange={setMode}
              displayName={(m) => MODE_LABEL[m]}
              detail={(m) => MODE_DETAIL[m]}
              optionIcon={modeGlyph}
              bordered
              disabled={busy}
            />
          </div>
          <!-- Morphing action button: busy → stop square (Void's ButtonStop),
               arming → red pulsing mic (start requested, mic not hot yet —
               no clock, no rings: nothing is being captured), live → the red
               button stays red and only the ICON morphs to the white stop
               square, plus ping rings + mm:ss clock (click stops; recording
               outranks the send morph so the stop control can't vanish the
               moment dictated text lands), content → accent send circle
               (Void's round ButtonSubmit shape), empty → mic. -->
          <!-- clock + mic grouped so the justify-between row keeps the timer
               glued to the icon instead of centering it. -->
          <span class="inline-flex shrink-0 items-center gap-2">
          <!-- Smart Dictation status: polishing spinner (cancellable) /
               polished + undo / kept-as-spoken. Renders nothing while idle. -->
          <DictationRewriteChip rewriter={dictationRewriter} />
          {#if dictationLive && !busy}
            <span class="font-mono text-[11px] font-semibold tabular-nums text-red-400" aria-live="off">
              {dictationClock}
            </span>
          {/if}
          <span class="relative inline-flex shrink-0">
            {#if dictationLive && !busy}
              {#each [0, 1, 2] as ring (ring)}
                <span
                  class="pointer-events-none absolute inset-0 rounded-full border-2 border-red-400/30 animate-ping"
                  style="animation-delay: {ring * 0.3}s; animation-duration: 2s;"
                ></span>
              {/each}
            {/if}
            <button
              type="button"
              onpointerdown={guardMicPress}
              onclick={() =>
                busy ? abortTurn() : dictating ? runDictation() : composerHasContent ? send() : runDictation()}
              disabled={!busy && !dictating && composerHasContent && ollamaNeedsModel}
              title={busy
                ? "Stop"
                : dictating
                  ? "Stop dictation"
                  : composerHasContent
                    ? "Send"
                    : "Dictate (voice to text) — hold Fn to push-to-talk; select text first to edit it by voice"}
              aria-label={busy
                ? "Stop generating"
                : dictating
                  ? "Stop voice dictation"
                  : composerHasContent
                    ? "Send"
                    : "Start voice dictation"}
              aria-pressed={!busy && dictating}
              class="relative z-10 grid h-7 w-7 shrink-0 place-items-center rounded-full transition-all duration-300 disabled:opacity-50
                {busy
                  ? 'bg-fg text-surface hover:brightness-110'
                  : dictating
                    ? 'bg-red-500 text-white shadow-lg shadow-red-500/40 hover:bg-red-600'
                    : composerHasContent
                      ? 'bg-accent text-on-accent hover:brightness-110'
                      : 'text-fg-muted hover:bg-surface-2 hover:text-fg'}"
            >
              {#if busy || dictationLive}
                <Icon name="square" size={12} class="fill-current" />
              {:else if !dictating && composerHasContent}
                <Icon name="arrow-up" size={15} />
              {:else}
                <Icon name="mic" size={15} class={dictating ? "animate-pulse" : ""} />
              {/if}
            </button>
          </span>
          </span>
        </div>
      </div>
    </div>
  {/if}
</div>
