/**
 * Terminal next-command ghost — the xterm surface of the shared completion
 * engine. As the user types a command at the prompt, a faint suggestion of the
 * rest of the line appears after the cursor (fish/Warp-style); Tab accepts it,
 * Cmd/Ctrl+→ accepts the next word, Esc dismisses. At an empty prompt it can
 * predict the *next* command outright (the `=`-style new-command idea from
 * yetone/smart-suggestion).
 *
 * The shell owns the real line buffer, so this tracks a *bounded* local model of
 * the current input: plain forward typing + backspace only. Any control or
 * escape sequence (arrows, Ctrl-R history search, Tab completion, Ctrl-U…) means
 * we can no longer trust our model, so we drop the ghost and wait for the next
 * fresh line. That keeps it correct for the common case and silent otherwise.
 *
 * Context sent to the model (lifted from smart-suggestion, done more cleanly —
 * xterm *is* the buffer, so no `script`/tmux proxy): recent typed commands, and,
 * when the user opts in, the recent terminal output (capped + lightly redacted).
 * Either way it only ever travels over SSH to the host's own model.
 *
 * The ghost is drawn with xterm's decoration API (the same mechanism the
 * highlight engine uses), anchored to the cursor cell so it tracks scrolling.
 */
import type { IDecoration, IDisposable, IMarker, Terminal } from "@xterm/xterm";

import { CompletionEngine } from "$lib/autocomplete/engine";
import { MAX_BUFFER_LINES, nextWord, readTerminalTail, redactSecrets } from "$lib/ide/terminal/terminalContext";
import { detectCompletionModel, fetchCompletion, type CompletionModel } from "$lib/ssh/complete";

export interface TerminalGhostOptions {
  term: Terminal;
  connectionId: string;
  label: string;
  /** Send text to the pty as if typed (used when accepting). */
  sendInput: (data: string) => void;
  /** Opt-in: include recent terminal output as model context. Default off. */
  bufferContext?: () => boolean;
  /** Only predict at an empty prompt when this pane is the active/visible one. */
  isActive?: () => boolean;
}

export interface TerminalGhost {
  /** Consult on keydown (from attachCustomKeyEventHandler). Returns true when it
      consumed the key — the caller should then stop xterm forwarding it. */
  handleKey: (e: KeyboardEvent) => boolean;
  dispose: () => void;
}

export function createTerminalGhost(opts: TerminalGhostOptions): TerminalGhost {
  const { term, connectionId, label, sendInput } = opts;
  const bufferContext = opts.bufferContext ?? (() => false);
  const isActive = opts.isActive ?? (() => true);

  let disposed = false;
  let line = ""; // our model of the current command line
  let ghost = ""; // the suggestion remainder currently shown
  let marker: IMarker | null = null;
  let decoration: IDecoration | null = null;
  let predictTimer: ReturnType<typeof setTimeout> | null = null;

  const recentCommands: string[] = [];

  let model: CompletionModel | null = null;
  let modelChecked = false;
  let detect: Promise<void> | null = null;

  async function ensureModel(): Promise<void> {
    if (modelChecked) return;
    detect ??= (async () => {
      // A general chat model is acceptable for completing a shell line.
      model = await detectCompletionModel(connectionId, label, true);
      modelChecked = true;
    })();
    await detect;
  }

  function historyRemainder(prefix: string): string | null {
    if (prefix.length < 2) return null;
    for (let i = recentCommands.length - 1; i >= 0; i--) {
      const cmd = recentCommands[i];
      if (cmd.length > prefix.length && cmd.startsWith(prefix)) return cmd.slice(prefix.length);
    }
    return null;
  }

  /** Build the model prompt as a shell transcript so even a base completion
      model continues with a plausible command. When the user has opted into
      buffer context, the live buffer tail already ends with the current prompt +
      typed text, so it serves as the prefix directly. */
  function buildPrompt(currentLine: string): string {
    if (bufferContext()) {
      const tail = redactSecrets(readTerminalTail(term, MAX_BUFFER_LINES));
      if (tail) return tail;
    }
    const parts = recentCommands.slice(-6).map((c) => `$ ${c}`);
    parts.push(`$ ${currentLine}`);
    return parts.join("\n");
  }

  const engine = new CompletionEngine({
    fetcher: async (ctx, signal) => {
      const hist = historyRemainder(ctx.prefix);
      if (hist) return hist;
      await ensureModel();
      if (!model || signal.aborted) return null;
      // A touch more headroom when predicting a whole command (empty prefix).
      const numPredict = ctx.prefix.length === 0 ? 32 : 24;
      return fetchCompletion(connectionId, model, buildPrompt(ctx.prefix), "", signal, numPredict);
    },
    debounceMs: 140,
    minPrefix: 0, // empty-prompt prediction is handled by the length gates below
  });

  function clearGhost(): void {
    ghost = "";
    decoration?.dispose();
    marker?.dispose();
    decoration = null;
    marker = null;
  }

  function renderGhost(text: string): void {
    clearGhost();
    if (!text || disposed) return;
    const buffer = term.buffer.active;
    const cursorX = buffer.cursorX;
    const width = Math.max(1, Math.min(text.length, term.cols - cursorX));
    const m = term.registerMarker(0); // 0 = the cursor's current line
    if (!m) return;
    const d = term.registerDecoration({ marker: m, x: cursorX, width, layer: "top" });
    if (!d) {
      m.dispose();
      return;
    }
    d.onRender((el) => {
      el.textContent = text;
      el.style.color = "rgba(230, 230, 230, 0.4)";
      el.style.whiteSpace = "pre";
      el.style.pointerEvents = "none";
      el.style.overflow = "visible";
      el.style.width = "auto";
    });
    ghost = text;
    marker = m;
    decoration = d;
  }

  function refresh(): void {
    // A single typed char is noise — wait for more before asking.
    if (line.length === 1) {
      engine.cancel();
      clearGhost();
      return;
    }
    // Empty prompt: predict the next command, but only for the visible pane.
    if (line.length === 0 && !isActive()) {
      engine.cancel();
      clearGhost();
      return;
    }
    const asked = line;
    void engine
      .request({ scope: `term:${connectionId}`, prefix: asked, suffix: "", multiline: false })
      .then((text) => {
        if (disposed || asked !== line) return; // stale
        if (text) renderGhost(text);
        else clearGhost();
      });
  }

  function reset(): void {
    line = "";
    engine.cancel();
    clearGhost();
  }

  /** After a command runs, predict the next one at the fresh prompt once output
      has settled (the `=` new-command behaviour from smart-suggestion). */
  function schedulePredict(): void {
    if (predictTimer) clearTimeout(predictTimer);
    predictTimer = setTimeout(() => {
      predictTimer = null;
      if (disposed || line !== "" || !isActive()) return;
      refresh();
    }, 500);
  }

  function commit(): void {
    const cmd = line.trim();
    if (cmd) {
      recentCommands.push(cmd);
      if (recentCommands.length > 100) recentCommands.shift();
    }
    reset();
    schedulePredict();
  }

  // Track the user's forward typing. A second onData listener alongside the
  // component's own (xterm allows several); independent and non-destructive.
  const dataSub: IDisposable = term.onData((d) => {
    if (disposed) return;
    if (d === "\r" || d === "\n") return commit();
    if (d === "\x7f" || d === "\b") {
      line = line.slice(0, -1);
      refresh();
      return;
    }
    // Tab completion / any control or escape sequence makes our model unreliable.
    // eslint-disable-next-line no-control-regex
    if (d === "\t" || /[\x00-\x1f]/.test(d)) return reset();
    line += d;
    refresh();
  });

  function accept(): void {
    if (!ghost) return;
    sendInput(ghost);
    line += ghost;
    clearGhost();
  }

  function acceptWord(): void {
    if (!ghost) return;
    const word = nextWord(ghost);
    const rest = ghost.slice(word.length);
    sendInput(word);
    line += word;
    clearGhost();
    // Re-anchor the remainder once the shell has echoed the accepted word and
    // advanced the cursor — unless the user typed more in the meantime.
    if (rest) {
      const at = line;
      requestAnimationFrame(() => {
        if (!disposed && line === at) renderGhost(rest);
      });
    }
  }

  function handleKey(e: KeyboardEvent): boolean {
    if (disposed || !ghost) return false;
    const mod = e.metaKey || e.ctrlKey;
    if (e.key === "Tab" && !mod && !e.altKey && !e.shiftKey) {
      accept();
      return true;
    }
    if (e.key === "Escape" && !mod && !e.altKey && !e.shiftKey) {
      clearGhost();
      return true;
    }
    if (e.key === "ArrowRight" && mod) {
      acceptWord();
      return true;
    }
    return false;
  }

  return {
    handleKey,
    dispose() {
      disposed = true;
      if (predictTimer) clearTimeout(predictTimer);
      dataSub.dispose();
      engine.dispose();
      clearGhost();
    },
  };
}
