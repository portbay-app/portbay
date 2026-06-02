<!--
  SshTerminalSession — one interactive remote shell: an xterm.js terminal bound
  to one backend pty. Output streams in over a Tauri Channel; keystrokes and
  resizes go back through the pty commands. The connect runs through
  connectWithPrompt, so a password/passphrase host is asked once (one-shot
  secret, never stored), exactly like the file browser.

  Designed to stay mounted while hidden (the tab strip keeps background shells
  alive), so xterm is created once and only re-fit + re-focused when this
  session becomes active. xterm + addons are dynamically imported in onMount so
  nothing terminal-related touches SSR or the main bundle.

  Theme is sampled from the app's own design tokens (bg-surface / text-fg /
  text-accent) so the terminal matches light/dark without hard-coded colors.
-->
<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import type { Terminal } from "@xterm/xterm";
  import type { FitAddon } from "@xterm/addon-fit";
  import type { SearchAddon } from "@xterm/addon-search";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import {
    openPty,
    ptyClose,
    ptyInput,
    ptyResize,
    type PtyEvent,
    type TerminalShortcut,
  } from "$lib/ssh/pty";
  import { compileRules, type CompiledRule } from "$lib/ssh/terminalHighlight";
  import { attachHighlightEngine, type HighlightEngine } from "$lib/ssh/terminalHighlightEngine";
  import { terminalPrefs } from "$lib/stores/sshWorkspacePrefs.svelte";

  let {
    connectionId,
    label,
    active = false,
    /** Run one program under the pty instead of a login shell (Logs tab). */
    command,
    /** Read-only view (no keystrokes sent) — for streamed log follows. */
    disableInput = false,
    onTitle,
    onExit,
    onShortcut,
    onFocus,
  }: {
    connectionId: string;
    label: string;
    active?: boolean;
    command?: string;
    disableInput?: boolean;
    onTitle?: (title: string) => void;
    onExit?: (code: number | null) => void;
    onShortcut?: (shortcut: TerminalShortcut) => void;
    /** Fired when this pane gains focus (so a split layout can track it). */
    onFocus?: () => void;
  } = $props();

  let host = $state<HTMLDivElement | null>(null);
  let status = $state<"connecting" | "open" | "closed" | "error">("connecting");
  let errorMsg = $state<string | null>(null);

  // Find bar (Cmd/Ctrl+F over the scrollback).
  let findOpen = $state(false);
  let findQuery = $state("");
  let findEl = $state<HTMLInputElement | null>(null);

  // Non-reactive engine handles (no need to trigger Svelte updates on these).
  let term: Terminal | null = null;
  let fit: FitAddon | null = null;
  let search: SearchAddon | null = null;
  let ptyId: string | null = null;
  let ro: ResizeObserver | null = null;
  let disposed = false;

  // Regex highlight rules, compiled once per change and read by the decoration
  // engine on every repaint (so live edits in Settings apply without re-attach).
  let highlightEngine: HighlightEngine | null = null;
  let compiledRules: CompiledRule[] = [];

  /** Read a computed color off a throwaway element styled with our tokens. */
  function sample(cls: string, prop: "color" | "backgroundColor"): string {
    const el = document.createElement("span");
    el.className = cls;
    el.style.display = "none";
    document.body.appendChild(el);
    const value = getComputedStyle(el)[prop];
    el.remove();
    return value || "";
  }

  function sampleFont(): string {
    const el = document.createElement("span");
    el.className = "font-mono";
    el.style.display = "none";
    document.body.appendChild(el);
    const family = getComputedStyle(el).fontFamily;
    el.remove();
    return family || "monospace";
  }

  /** Translucent accent for the selection highlight. */
  function translucent(rgb: string, alpha: number): string {
    const m = rgb.match(/rgba?\(([^)]+)\)/);
    if (!m) return rgb;
    const [r, g, b] = m[1].split(",").map((s) => s.trim());
    return `rgba(${r}, ${g}, ${b}, ${alpha})`;
  }

  /** Fit the terminal to its container — but only when actually visible, since a
      hidden (display:none) container has zero size and would corrupt the grid. */
  function fitNow() {
    if (!fit || !term || !host) return;
    if (host.clientWidth === 0 || host.clientHeight === 0) return;
    try {
      fit.fit();
    } catch {
      /* container mid-layout; the ResizeObserver will fit again */
    }
  }

  function runSearch(forward: boolean) {
    if (!search || !findQuery) return;
    if (forward) search.findNext(findQuery);
    else search.findPrevious(findQuery);
  }

  function closeFind() {
    findOpen = false;
    findQuery = "";
    term?.focus();
  }

  onMount(() => {
    void (async () => {
      const [{ Terminal }, { FitAddon }, { WebLinksAddon }, { SearchAddon }, { Unicode11Addon }] =
        await Promise.all([
          import("@xterm/xterm"),
          import("@xterm/addon-fit"),
          import("@xterm/addon-web-links"),
          import("@xterm/addon-search"),
          import("@xterm/addon-unicode11"),
        ]);
      await import("@xterm/xterm/css/xterm.css");
      if (disposed || !host) return;

      const fg = sample("text-fg", "color") || "#e6e6e6";
      const bg = sample("bg-surface", "backgroundColor") || "#0b0b0b";
      const accent = sample("text-accent", "color") || fg;

      const prefs = terminalPrefs.value;
      term = new Terminal({
        fontFamily: sampleFont(),
        fontSize: prefs.fontSize,
        lineHeight: 1.2,
        cursorBlink: !disableInput && prefs.cursorBlink,
        scrollback: prefs.scrollback,
        disableStdin: disableInput,
        allowProposedApi: true,
        macOptionIsMeta: true,
        theme: {
          background: bg,
          foreground: fg,
          cursor: accent,
          cursorAccent: bg,
          selectionBackground: translucent(accent, 0.3),
        },
      });

      fit = new FitAddon();
      search = new SearchAddon();
      const unicode = new Unicode11Addon();
      term.loadAddon(fit);
      term.loadAddon(search);
      term.loadAddon(unicode);
      term.loadAddon(new WebLinksAddon());
      term.unicode.activeVersion = "11";

      term.open(host);
      fitNow();

      // Paint user-defined regex highlight rules over output. The engine reads
      // `compiledRules` on each repaint; the $effect below keeps it current.
      compiledRules = compileRules(terminalPrefs.value.highlightRules);
      highlightEngine = attachHighlightEngine(term, () => compiledRules);

      // Intercept window/tab chords before xterm forwards them to the shell.
      // Cmd (mac) or Ctrl+Shift (Linux/Win) to avoid clobbering readline's
      // Ctrl-key bindings; Cmd/Ctrl+F opens the find bar. Returning false keeps
      // xterm from sending the keystroke to the remote pty.
      term.attachCustomKeyEventHandler((e) => {
        if (e.type !== "keydown") return true;
        const mod = e.metaKey || e.ctrlKey;
        if (!mod) return true;
        const key = e.key.toLowerCase();
        if (key === "f" && !e.shiftKey) {
          e.preventDefault();
          findOpen = true;
          requestAnimationFrame(() => findEl?.focus());
          return false;
        }
        if (key === "t" && e.shiftKey) {
          e.preventDefault();
          onShortcut?.({ action: "new" });
          return false;
        }
        if (key === "w" && e.shiftKey) {
          e.preventDefault();
          onShortcut?.({ action: "close" });
          return false;
        }
        // Split the active tab: Cmd/Ctrl+Shift+E side-by-side, +D stacked
        // (matching the reference IDE's split chords).
        if (key === "e" && e.shiftKey) {
          e.preventDefault();
          onShortcut?.({ action: "split", direction: "row" });
          return false;
        }
        if (key === "d" && e.shiftKey) {
          e.preventDefault();
          onShortcut?.({ action: "split", direction: "col" });
          return false;
        }
        if (e.key === "Tab") {
          e.preventDefault();
          onShortcut?.({ action: e.shiftKey ? "prev" : "next" });
          return false;
        }
        if (!e.shiftKey && e.key >= "1" && e.key <= "9") {
          e.preventDefault();
          onShortcut?.({ action: "jump", index: Number(e.key) - 1 });
          return false;
        }
        return true;
      });

      // Keystrokes / resizes / window title back to the backend + tab strip.
      term.onData((data) => {
        if (ptyId) ptyInput(ptyId, data);
      });
      term.onResize(({ cols, rows }) => {
        if (ptyId) ptyResize(ptyId, cols, rows);
      });
      term.onTitleChange((title) => {
        if (title) onTitle?.(title);
      });

      const onEvent = (event: PtyEvent) => {
        if (!term) return;
        if (event.type === "data") {
          term.write(Uint8Array.from(event.bytes));
        } else {
          status = "closed";
          term.write(
            `\r\n\x1b[2m[process exited${
              event.code != null && event.code !== 0 ? ` — code ${event.code}` : ""
            }]\x1b[0m\r\n`,
          );
          onExit?.(event.code);
        }
      };

      try {
        ptyId = await openPty(connectionId, label, term.cols, term.rows, onEvent, command);
        if (disposed) {
          ptyClose(ptyId);
          return;
        }
        status = "open";
        // Re-assert size now that the shell is live (it opened at 80×24).
        fitNow();
        // A configured startup command runs once in a fresh interactive shell
        // (not for one-shot `command` sessions like Logs).
        if (!command && prefs.startupCommand.trim()) {
          ptyInput(ptyId, `${prefs.startupCommand.trim()}\r`);
        }
        if (active && !disableInput) term.focus();
      } catch (e) {
        status = "error";
        errorMsg =
          e && typeof e === "object" && "whatHappened" in e
            ? String((e as { whatHappened: unknown }).whatHappened)
            : "Couldn't open the shell.";
        return;
      }

      // Keep the grid matched to the container as the pane resizes.
      ro = new ResizeObserver(() => fitNow());
      ro.observe(host);
    })();
  });

  onDestroy(() => {
    disposed = true;
    ro?.disconnect();
    highlightEngine?.dispose();
    if (ptyId) ptyClose(ptyId);
    term?.dispose();
  });

  // Recompile + repaint when the rule set changes (live edits in Settings).
  $effect(() => {
    compiledRules = compileRules(terminalPrefs.value.highlightRules);
    highlightEngine?.refresh();
  });

  // When this session becomes the active tab, its container gains size — fit and
  // focus on the next frame so the grid is correct and typing lands here.
  $effect(() => {
    if (active && term) {
      requestAnimationFrame(() => {
        fitNow();
        if (!disableInput) term?.focus();
      });
    }
  });

  // Live font-size: the toolbar's A−/A+ buttons write the global pref; every
  // mounted terminal re-renders at the new size and refits its grid.
  $effect(() => {
    const fontSize = terminalPrefs.value.fontSize;
    if (term) {
      term.options.fontSize = fontSize;
      requestAnimationFrame(() => fitNow());
    }
  });

  /** Clear the scrollback + viewport of this pane (toolbar Clear button). */
  export function clear() {
    term?.clear();
  }

  /** Focus this pane's terminal (used when a split pane is selected). */
  export function focusTerm() {
    if (!disableInput) term?.focus();
  }

  // Cmd/Ctrl+F is intercepted inside xterm (see attachCustomKeyEventHandler);
  // here we only need Escape to dismiss the find bar when its input has focus.
  function onKeydown(ev: KeyboardEvent) {
    if (ev.key === "Escape" && findOpen) {
      ev.preventDefault();
      closeFind();
    }
  }
</script>

<div
  class="relative h-full w-full bg-surface"
  role="presentation"
  onkeydown={onKeydown}
  onfocusin={() => onFocus?.()}
  onpointerdown={() => onFocus?.()}
>
  {#if status === "error"}
    <div class="flex h-full items-center justify-center p-6">
      <div class="max-w-sm rounded-lg border border-status-crashed/40 bg-status-crashed/10 p-4 text-center">
        <Icon name="circle-alert" size={18} class="mx-auto text-status-crashed" />
        <p class="mt-2 text-[12.5px] text-fg">{errorMsg}</p>
      </div>
    </div>
  {/if}

  <!-- Find bar -->
  {#if findOpen}
    <div class="absolute right-3 top-3 z-10 flex items-center gap-1 rounded-lg border border-border bg-surface px-2 py-1.5 shadow-xl">
      <Icon name="search" size={13} class="text-fg-subtle" />
      <input
        bind:this={findEl}
        bind:value={findQuery}
        oninput={() => runSearch(true)}
        onkeydown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            runSearch(!e.shiftKey);
          }
        }}
        placeholder="Find"
        class="h-6 w-40 bg-transparent text-[12px] text-fg outline-none placeholder:text-fg-subtle"
      />
      <button type="button" onclick={() => runSearch(false)} class="rounded p-1 text-fg-muted hover:bg-surface-2 hover:text-fg" aria-label="Previous match">
        <Icon name="chevron-up" size={13} />
      </button>
      <button type="button" onclick={() => runSearch(true)} class="rounded p-1 text-fg-muted hover:bg-surface-2 hover:text-fg" aria-label="Next match">
        <Icon name="chevron-down" size={13} />
      </button>
      <button type="button" onclick={closeFind} class="rounded p-1 text-fg-muted hover:bg-surface-2 hover:text-fg" aria-label="Close find">
        <Icon name="x" size={13} />
      </button>
    </div>
  {/if}

  <!-- xterm mounts here. Padding via the wrapper so the fit addon measures a
       clean rectangle. -->
  <div bind:this={host} class="h-full w-full px-2 py-1.5"></div>
</div>
