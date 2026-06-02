<!--
  LogViewer — full-screen modal log tail for one project.

  Two modes:
    - Static tail (default): one-shot snapshot from `tail_logs`.
      Refresh button re-fetches.
    - Follow: a Channel<string> from `subscribe_logs` streams new
      lines as they're written. The channel is dropped on
      unmount / toggle-off so the Rust task exits cleanly.
-->
<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { trapFocus } from "$lib/actions/trapFocus";
  import { Channel, invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";

  import { Icon, StatusPill } from "$lib/components/atoms";
  import { safeInvoke } from "$lib/ipc";
  import { logViewer } from "$lib/stores/logViewer.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import type { ProjectView } from "$lib/types/projects";
  import {
    parseLogLine,
    eventLogLine,
    levelClass,
    type LogLevel,
    type LogLine,
  } from "./ansi";

  /** PortBay-authored lifecycle line pushed over `portbay://proc-log`. */
  type ProcLogEvent = { id: string; level: string; message: string };

  /** Cap on rendered lines. Keeps DOM size bounded under chatty servers. */
  const MAX_LINES = 5_000;
  /** When trimming, drop this many from the front so we don't trim every line. */
  const TRIM_CHUNK = 1_000;
  const project = $derived<ProjectView | null>(
    logViewer.id === null
      ? null
      : (projects.value.find((p) => p.id === logViewer.id) ?? null),
  );

  // `parsed` is the render/search/copy source: each raw line is unwrapped from
  // Process Compose's JSON envelope, ANSI-rendered, and tagged with a level.
  // Parsed once on arrival (here + in follow) rather than re-derived, so a
  // chatty follow stream stays O(1) per line instead of re-parsing the buffer.
  let parsed = $state<LogLine[]>([]);
  let loading = $state<boolean>(false);
  let follow = $state<boolean>(false);
  let searchQuery = $state<string>("");
  let matchIndex = $state<number>(0);
  let autoScroll = $state<boolean>(true);
  let scrollerEl: HTMLDivElement | undefined = $state();
  /** Active follow channel — null when not following. */
  let followChannel: Channel<string> | null = null;

  // ----- level filter -----
  // Mirrors the /logs page so the modal and the full page filter identically.
  // "Info" folds in debug — there's no Debug tab and a line should never vanish
  // into a level with no home. Error / Warn stay exact.
  type LevelFilter = "all" | "error" | "warn" | "info";
  const LEVEL_TABS: { value: LevelFilter; label: string }[] = [
    { value: "all", label: "All" },
    { value: "error", label: "Error" },
    { value: "warn", label: "Warn" },
    { value: "info", label: "Info" },
  ];
  let levelFilter = $state<LevelFilter>("all");

  function matchesLevel(level: LogLevel): boolean {
    switch (levelFilter) {
      case "all":
        return true;
      case "error":
        return level === "error";
      case "warn":
        return level === "warn";
      case "info":
        // PortBay lifecycle lines ride along with Info so they never vanish
        // into a tab with no home when the user narrows the filter.
        return level === "info" || level === "debug" || level === "system";
    }
  }

  // The rendered set: level-filtered. Search highlights/jumps within it (it
  // doesn't filter), so match indices below are indices into `visible`.
  const visible = $derived(parsed.filter((pl) => matchesLevel(pl.level)));

  async function reload() {
    if (!project) return;
    loading = true;
    try {
      const raw = await safeInvoke<string[]>("tail_logs", {
        id: project.id,
        limit: 1000,
      });
      parsed = raw.map(parseLogLine);
    } catch {
      parsed = [];
    } finally {
      loading = false;
      if (autoScroll) requestAnimationFrame(scrollToBottom);
    }
  }

  function scrollToBottom() {
    if (!scrollerEl) return;
    scrollerEl.scrollTop = scrollerEl.scrollHeight;
  }

  // Incoming lines are buffered and committed once per animation frame
  // rather than one reactive update per line. The backend now delivers
  // lines the instant they're written (FS-event driven), so a chatty
  // server can fire a burst in a single frame; coalescing keeps that to
  // one array rebuild + one render + one scroll instead of N, which is
  // what keeps fast streams smooth instead of janky.
  let pending: LogLine[] = [];
  let flushHandle: number | null = null;

  function flushPending() {
    flushHandle = null;
    if (pending.length === 0) return;
    let next = parsed.concat(pending);
    pending = [];
    // Trim the head when over cap so the DOM stays bounded. Keep the last
    // (MAX_LINES - TRIM_CHUNK) so a big burst can't blow past the cap and
    // we don't re-slice every frame.
    if (next.length > MAX_LINES) {
      next = next.slice(next.length - (MAX_LINES - TRIM_CHUNK));
    }
    parsed = next;
    if (autoScroll) scrollToBottom();
  }

  function startFollow() {
    if (followChannel !== null || !project) return;
    const id = project.id;
    const ch = new Channel<string>();
    ch.onmessage = (line) => {
      pending.push(parseLogLine(line));
      if (flushHandle === null) {
        flushHandle = requestAnimationFrame(flushPending);
      }
    };
    followChannel = ch;
    // Fire-and-forget; the backend task runs until the channel is
    // dropped by stopFollow / unmount.
    void invoke("subscribe_logs", { id, onLine: ch }).catch(() => {
      // Backend refused (sidecar down, registry mismatch). Toast was
      // already pushed by the safeInvoke wrapper if invoked through it;
      // here we just unwind the follow toggle.
      followChannel = null;
      follow = false;
    });
  }

  function stopFollow() {
    if (followChannel !== null) {
      // Dropping the channel reference frees the Rust-side Channel<String>
      // on next tick; the spawn_blocking tail loop sees send() fail and
      // exits. There's no explicit close() on the Tauri Channel API.
      followChannel.onmessage = () => {};
      followChannel = null;
    }
    // Drop any lines buffered for the next frame so a re-follow / project
    // switch can't flush a previous stream's tail into the new buffer.
    if (flushHandle !== null) {
      cancelAnimationFrame(flushHandle);
      flushHandle = null;
    }
    pending = [];
  }

  // PortBay lifecycle lines (Starting / command echo / port-conflict) arrive
  // over an app-global event keyed by project id — independent of the Follow
  // toggle, so pressing Play always shows immediate feedback even before the
  // file tail has any output. Tied to which project the viewer is open on.
  let procUnlisten: UnlistenFn | null = null;

  async function startProcLog(id: string) {
    stopProcLog();
    const un = await listen<ProcLogEvent>("portbay://proc-log", (e) => {
      if (e.payload.id !== id) return;
      pending.push(eventLogLine(e.payload.message, e.payload.level as LogLevel));
      if (flushHandle === null) {
        flushHandle = requestAnimationFrame(flushPending);
      }
    });
    // The await above can resolve after the user switched projects; if so,
    // detach immediately instead of leaking a stale listener.
    if (untrack(() => openedId) === id) procUnlisten = un;
    else un();
  }

  function stopProcLog() {
    if (procUnlisten !== null) {
      procUnlisten();
      procUnlisten = null;
    }
  }

  // Re-init only when the viewer opens for a *different* project.
  // We gate on the project id (a string) rather than the derived
  // `project` object — the projects store mints new object references
  // on every 1.5 s status tick, which would otherwise re-trigger this
  // effect and wipe the log/reset Follow mode mid-stream.
  const openedId = $derived(logViewer.id);
  $effect(() => {
    const id = openedId;
    if (id === null) {
      stopFollow();
      stopProcLog();
      return;
    }
    untrack(() => {
      parsed = [];
      searchQuery = "";
      matchIndex = 0;
      levelFilter = "all";
      autoScroll = true;
      follow = false;
      void reload();
      void startProcLog(id);
    });
  });

  // Follow toggle wires up / tears down the poll.
  $effect(() => {
    if (follow) startFollow();
    else stopFollow();
  });

  // ----- search -----
  const matches = $derived.by(() => {
    const q = searchQuery.trim().toLowerCase();
    if (!q) return [] as number[];
    const found: number[] = [];
    for (let i = 0; i < visible.length; i++) {
      if (visible[i].text.toLowerCase().includes(q)) found.push(i);
    }
    return found;
  });

  function jumpToMatch(direction: 1 | -1) {
    if (matches.length === 0) return;
    matchIndex =
      (matchIndex + direction + matches.length) % matches.length;
    scrollToLine(matches[matchIndex]);
  }

  function scrollToLine(idx: number) {
    if (!scrollerEl) return;
    const lineEl = scrollerEl.querySelector(
      `[data-line="${idx}"]`,
    ) as HTMLElement | null;
    if (lineEl) {
      lineEl.scrollIntoView({ block: "center", behavior: "smooth" });
    }
  }

  // Terminal-style `clear`: wipe the *visible buffer* only. Like a real
  // terminal's `clear`, it does not touch the on-disk `<id>.log` — new lines
  // keep streaming in afterward, and Follow / auto-scroll state is preserved.
  function clearView() {
    parsed = [];
    pending = [];
    searchQuery = "";
    matchIndex = 0;
  }

  async function copyAll() {
    // No notification — copying is self-evident. Quietly ignore a missing perm.
    try {
      await navigator.clipboard.writeText(parsed.map((p) => p.text).join("\n"));
    } catch {
      /* silently fail */
    }
  }

  function onKeydown(e: KeyboardEvent) {
    if (logViewer.id === null) return;
    if (e.key === "Escape") {
      logViewer.hide();
      return;
    }
    // `/` focuses search (only when not already in an input).
    if (
      e.key === "/" &&
      !(e.target instanceof HTMLInputElement)
    ) {
      e.preventDefault();
      (document.getElementById("logviewer-search") as HTMLInputElement)?.focus();
      return;
    }
    if (e.target instanceof HTMLInputElement) return;
    if (e.key === "n") jumpToMatch(1);
    else if (e.key === "N") jumpToMatch(-1);
  }

  onMount(() => () => {
    stopFollow();
    stopProcLog();
  });

  // Detect manual scroll-up; turn off autoScroll when not at bottom.
  function onScroll() {
    if (!scrollerEl) return;
    const atBottom =
      scrollerEl.scrollHeight - scrollerEl.scrollTop - scrollerEl.clientHeight < 40;
    autoScroll = atBottom;
  }
</script>

<svelte:window onkeydown={onKeydown} />

{#if project}
  <!-- Backdrop closes only on a direct click (target === backdrop), so a
       click inside the dialog doesn't bubble out and dismiss it — no inner
       stopPropagation needed. Escape (window handler) covers keyboard. -->
  <div
    class="fixed inset-0 z-50 bg-bg/70 backdrop-blur-sm flex items-center justify-center p-6"
    onclick={(e) => {
      if (e.target === e.currentTarget) logViewer.hide();
    }}
    role="presentation"
  >
    <div
      use:trapFocus
      class="w-[1100px] max-w-[95vw] h-[85vh] bg-surface border border-border rounded-xl shadow-2xl flex flex-col overflow-hidden"
      role="dialog"
      aria-label="Log viewer"
      aria-modal="true"
      tabindex="-1"
    >
      <!-- Header -->
      <header
        class="shrink-0 flex flex-wrap items-center gap-x-3 gap-y-2 px-4 py-3 border-b border-border"
      >
        <Icon name="terminal" size={16} class="text-fg-muted" />
        <h2 class="text-sm font-semibold text-fg">{project.name}</h2>
        <StatusPill status={project.status} />

        <!-- Level tabs — same filter as the /logs page -->
        <div
          role="group"
          aria-label="Filter by level"
          class="flex items-center gap-1 bg-surface-2 border border-border rounded-lg p-1"
        >
          {#each LEVEL_TABS as tab (tab.value)}
            {@const active = levelFilter === tab.value}
            <button
              type="button"
              onclick={() => (levelFilter = tab.value)}
              aria-pressed={active}
              class="px-2.5 h-6 rounded-md text-[11px] font-medium transition-colors
                     focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40
                     {active
                ? tab.value === 'all'
                  ? 'bg-accent text-on-accent'
                  : tab.value === 'error'
                    ? 'bg-status-crashed/15 text-status-crashed'
                    : tab.value === 'warn'
                      ? 'bg-status-unhealthy/15 text-status-unhealthy'
                      : 'bg-accent/15 text-accent'
                : tab.value === 'error'
                  ? 'text-status-crashed/70 hover:text-status-crashed'
                  : tab.value === 'warn'
                    ? 'text-status-unhealthy/70 hover:text-status-unhealthy'
                    : tab.value === 'info'
                      ? 'text-accent/70 hover:text-accent'
                      : 'text-fg-muted hover:text-fg'}"
            >
              {tab.label}
            </button>
          {/each}
        </div>

        <!-- Clear — wipes the visible buffer only (terminal `clear`), placed
             immediately before the Follow control. Does not delete the log. -->
        <button
          type="button"
          onclick={clearView}
          title="Clear the view (does not delete the log file)"
          class="ml-auto flex items-center gap-1.5 px-2 h-6 rounded-md text-xs text-fg-muted
                 hover:text-fg hover:bg-surface-2 transition-colors"
        >
          <Icon name="eraser" size={12} />
          Clear
        </button>

        <!-- Follow toggle -->
        <label
          class="flex items-center gap-1.5 text-xs text-fg-muted cursor-pointer"
          title="Live tail — new log lines stream in as the project writes them. Like `tail -f`."
        >
          <input
            type="checkbox"
            bind:checked={follow}
            class="accent-accent"
          />
          Follow
        </label>

        <!-- Search -->
        <div
          class="flex items-center w-56 h-7 rounded-md bg-bg border border-border focus-within:border-accent/60 transition-colors"
        >
          <span class="pl-2 text-fg-subtle">
            <Icon name="search" size={12} />
          </span>
          <input
            id="logviewer-search"
            type="text"
            bind:value={searchQuery}
            oninput={() => (matchIndex = 0)}
            placeholder="Search (/)"
            class="flex-1 bg-transparent text-xs pl-2 pr-2 outline-none text-fg placeholder-fg-subtle"
          />
          {#if matches.length > 0}
            <span class="px-2 text-[11px] text-fg-subtle tabular-nums">
              {matchIndex + 1}/{matches.length}
            </span>
          {/if}
        </div>

        {#if matches.length > 0}
          <button
            type="button"
            onclick={() => jumpToMatch(-1)}
            title="Previous match (N)"
            class="p-1 rounded-md text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
          >
            <Icon name="chevron-down" size={12} class="rotate-180" />
          </button>
          <button
            type="button"
            onclick={() => jumpToMatch(1)}
            title="Next match (n)"
            class="p-1 rounded-md text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
          >
            <Icon name="chevron-down" size={12} />
          </button>
        {/if}

        <button
          type="button"
          onclick={() => void reload()}
          title="Reload"
          class="p-1 rounded-md text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
          class:animate-spin={loading}
        >
          <Icon name="refresh-cw" size={12} />
        </button>
        <button
          type="button"
          onclick={copyAll}
          title="Copy entire log"
          class="p-1 rounded-md text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
        >
          <Icon name="link" size={12} />
        </button>
        <button
          type="button"
          onclick={() => logViewer.hide()}
          title="Close"
          aria-label="Close log viewer"
          class="p-1 rounded-md text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
        >
          <Icon name="x" size={14} />
        </button>
      </header>

      <!--
        Body. The previous implementation wrapped each line in a
        <div> *inside* a <pre>, which preserved every newline /
        indent from the source template as rendered whitespace — the
        cause of the huge gaps the user reported. We use a plain
        container with `whitespace-pre` on each row so ANSI-rendered
        spaces still align, without the source-template noise.
      -->
      <div
        bind:this={scrollerEl}
        onscroll={onScroll}
        class="flex-1 min-h-0 overflow-y-auto bg-bg py-2 font-mono text-[12px] leading-[1.4] text-fg-muted"
      >
        {#if visible.length === 0}
          <p class="text-xs text-fg-subtle italic px-4 py-4">
            {loading
              ? "Loading log…"
              : parsed.length === 0
                ? "No log output yet."
                : "No lines match the current filter."}
          </p>
        {:else}
          {#each visible as pl, i (i)}
            <div
              data-line={i}
              class="px-4 whitespace-pre-wrap break-words {levelClass(pl.level)}
                     {pl.level === 'error' ? 'bg-status-crashed/5' : ''}
                     {matches.includes(i) ? 'bg-accent/10 text-fg' : ''}
                     {matches[matchIndex] === i ? 'ring-1 ring-accent' : ''}"
            >{@html pl.html}</div>
          {/each}
        {/if}
      </div>

      <!-- Footer hint -->
      <footer
        class="shrink-0 px-4 py-2 border-t border-border flex items-center gap-3 text-[11px] text-fg-subtle"
      >
        <span>
          {visible.length}{#if visible.length !== parsed.length} / {parsed.length}{/if}
          line{visible.length === 1 ? "" : "s"}
        </span>
        {#if follow}
          <span class="text-status-running">● following (live stream)</span>
        {/if}
        <span class="ml-auto">
          ESC close · / search · n / N next / prev · click outside to close
        </span>
      </footer>
    </div>
  </div>
{/if}
