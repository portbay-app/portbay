<!--
  /logs — inline live log viewer.

  Pick a project from the dropdown and its output streams in below: an
  initial `tail_logs` snapshot, then a `subscribe_logs` Channel<string> for
  live lines (like `tail -f`). Lines are level-tagged and colour-coded by the
  shared `ansi.ts` parser — the same parser the LogViewer modal uses, so the
  fetch/parse logic is unchanged here; only the page layout is new.

  The level tabs (All / Error / Warn / Info) and the search box are local
  filters over the parsed buffer. "Auto-scroll" sticks the view to the tail;
  scrolling up releases it. "Open in Terminal" hands the project folder to the
  user's installed terminal via `open_in_ide`.
-->
<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { Channel, invoke } from "@tauri-apps/api/core";

  import { Icon, ProjectAvatar, StatusDot, StatusPill } from "$lib/components/atoms";
  import { safeInvoke } from "$lib/ipc";
  import { devTools } from "$lib/stores/devTools.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import type { ProjectView } from "$lib/types/projects";
  import { parseLogLine, type LogLevel, type LogLine } from "$lib/components/logs/ansi";

  /** Cap on rendered lines — keeps the DOM bounded under chatty servers. */
  const MAX_LINES = 5_000;
  /** When over cap, drop this many from the head so we don't trim every line. */
  const TRIM_CHUNK = 1_000;

  type LevelFilter = "all" | "error" | "warn" | "info";
  const LEVEL_TABS: { value: LevelFilter; label: string }[] = [
    { value: "all", label: "All" },
    { value: "error", label: "Error" },
    { value: "warn", label: "Warn" },
    { value: "info", label: "Info" },
  ];

  let selectedId = $state<string | null>(null);
  let levelFilter = $state<LevelFilter>("all");
  let searchQuery = $state<string>("");
  let parsed = $state<LogLine[]>([]);
  let loading = $state<boolean>(false);
  let autoScroll = $state<boolean>(true);
  let copied = $state<boolean>(false);
  let pickerOpen = $state<boolean>(false);

  let scrollerEl: HTMLDivElement | undefined = $state();
  let pickerEl: HTMLDivElement | undefined = $state();
  /** Active follow channel — null when not streaming. */
  let followChannel: Channel<string> | null = null;

  const project = $derived<ProjectView | null>(
    selectedId === null
      ? null
      : (projects.value.find((p) => p.id === selectedId) ?? null),
  );

  /** First installed terminal, if any — target for "Open in Terminal". */
  const terminalTool = $derived(
    devTools.value.find((t) => t.kind === "terminal") ?? null,
  );

  // ---- filtering -------------------------------------------------------
  // "Info" folds in debug — the design has no Debug tab, and we never want a
  // line to vanish into a level with no home. Error / Warn stay exact.
  function matchesLevel(level: LogLevel): boolean {
    switch (levelFilter) {
      case "all":
        return true;
      case "error":
        return level === "error";
      case "warn":
        return level === "warn";
      case "info":
        return level === "info" || level === "debug";
    }
  }

  const visible = $derived.by(() => {
    const q = searchQuery.trim().toLowerCase();
    return parsed.filter(
      (pl) =>
        matchesLevel(pl.level) &&
        (q === "" || pl.text.toLowerCase().includes(q)),
    );
  });

  // ---- per-level styling (matches the mock) ----------------------------
  // The LEVEL token is always coloured. The message itself only turns red for
  // errors; everything else inherits the muted terminal foreground.
  function tokenClass(level: LogLevel): string {
    switch (level) {
      case "error":
        return "text-status-crashed";
      case "warn":
        return "text-status-unhealthy";
      case "debug":
        return "text-accent";
      default:
        return "text-status-running";
    }
  }

  // ---- data ------------------------------------------------------------
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
    }
  }

  function startFollow(id: string) {
    if (followChannel !== null) return;
    const ch = new Channel<string>();
    ch.onmessage = (line) => {
      parsed = parsed.concat(parseLogLine(line));
      if (parsed.length > MAX_LINES) parsed = parsed.slice(TRIM_CHUNK);
    };
    followChannel = ch;
    // Fire-and-forget: the backend task runs until the channel is dropped by
    // stopFollow / project switch / unmount.
    void invoke("subscribe_logs", { id, onLine: ch }).catch(() => {
      followChannel = null;
    });
  }

  function stopFollow() {
    if (followChannel !== null) {
      // Dropping the reference frees the Rust-side Channel on next tick; the
      // tail loop sees send() fail and exits. No explicit close() on the API.
      followChannel.onmessage = () => {};
      followChannel = null;
    }
  }

  function scrollToBottom() {
    if (scrollerEl) scrollerEl.scrollTop = scrollerEl.scrollHeight;
  }

  // Re-init when the selected project changes. Gated on the id (a string), not
  // the derived `project` object — the projects store mints fresh references
  // every status tick, which would otherwise wipe the buffer mid-stream.
  $effect(() => {
    const id = selectedId;
    untrack(() => stopFollow());
    if (id === null) {
      untrack(() => (parsed = []));
      return;
    }
    untrack(() => {
      parsed = [];
      autoScroll = true;
      void reload();
      startFollow(id);
    });
  });

  // Stick to the tail as new lines land (and when filters change), but only
  // while auto-scroll is engaged. Reading visible.length re-runs this on every
  // append; reading autoScroll re-runs it (and scrolls) when re-engaged.
  $effect(() => {
    void visible.length;
    if (autoScroll) requestAnimationFrame(scrollToBottom);
  });

  // Pick a sensible default project once the list loads: prefer a running one,
  // else the first. Guarded on `selectedId === null` so it fires only once.
  $effect(() => {
    if (selectedId !== null || projects.value.length === 0) return;
    const running = projects.value.find((p) => p.status === "running");
    selectedId = (running ?? projects.value[0]).id;
  });

  // Manual scroll-up releases auto-scroll; scrolling back to the bottom re-arms it.
  function onScroll() {
    if (!scrollerEl) return;
    autoScroll =
      scrollerEl.scrollHeight - scrollerEl.scrollTop - scrollerEl.clientHeight <
      40;
  }

  function selectProject(id: string) {
    selectedId = id;
    pickerOpen = false;
  }

  async function copyUrl() {
    if (!project) return;
    try {
      await navigator.clipboard.writeText(project.url);
      copied = true;
      setTimeout(() => (copied = false), 1500);
    } catch {
      /* clipboard blocked — silently ignore */
    }
  }

  async function openInTerminal() {
    if (!project || !terminalTool) return;
    await safeInvoke("open_in_ide", { id: project.id, ide: terminalTool.id });
  }

  // Close the picker on outside-click / Escape.
  function onWindowClick(e: MouseEvent) {
    if (pickerOpen && pickerEl && !pickerEl.contains(e.target as Node)) {
      pickerOpen = false;
    }
  }
  function onWindowKeydown(e: KeyboardEvent) {
    if (e.key === "Escape" && pickerOpen) pickerOpen = false;
  }

  onMount(() => {
    void devTools.start();
    return () => stopFollow();
  });
</script>

<svelte:window onclick={onWindowClick} onkeydown={onWindowKeydown} />

<div class="flex flex-col h-full min-h-0">
  <header class="px-6 pt-6 pb-4 shrink-0 space-y-4">
    <div>
      <h1 class="text-2xl font-semibold text-fg">Logs</h1>
      <p class="text-[13px] text-fg-subtle mt-1">
        Inspect live output, errors, and runtime activity across your local
        projects.
      </p>
    </div>

    <!-- Toolbar: project picker · level tabs · search -->
    <div class="flex flex-wrap items-center gap-3">
      <!-- Project picker -->
      <div bind:this={pickerEl} class="relative">
        <button
          type="button"
          onclick={() => (pickerOpen = !pickerOpen)}
          disabled={projects.value.length === 0}
          aria-haspopup="listbox"
          aria-expanded={pickerOpen}
          class="flex items-center gap-2 h-9 w-56 px-2.5 rounded-lg bg-surface-2
                 border border-border text-left hover:border-border-strong
                 disabled:opacity-50 disabled:cursor-not-allowed transition-colors
                 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40"
        >
          {#if project}
            <ProjectAvatar id={project.id} name={project.name} size={20} />
            <span class="flex-1 truncate text-[13px] font-medium text-fg"
              >{project.name}</span
            >
          {:else}
            <span class="flex-1 truncate text-[13px] text-fg-subtle">
              {projects.value.length === 0 ? "No projects" : "Select a project"}
            </span>
          {/if}
          <Icon name="chevron-down" size={16} class="text-fg-subtle shrink-0" />
        </button>

        {#if pickerOpen}
          <div
            role="listbox"
            aria-label="Select a project"
            class="absolute z-30 mt-1.5 w-64 max-h-72 overflow-y-auto rounded-lg
                   bg-surface border border-border shadow-2xl p-1"
          >
            {#each projects.value as p (p.id)}
              <button
                type="button"
                role="option"
                aria-selected={p.id === selectedId}
                onclick={() => selectProject(p.id)}
                class="w-full flex items-center gap-2.5 px-2 py-1.5 rounded-md text-left
                       transition-colors {p.id === selectedId
                  ? 'bg-accent/10'
                  : 'hover:bg-surface-2'}"
              >
                <ProjectAvatar id={p.id} name={p.name} size={20} />
                <span class="flex-1 truncate text-[13px] text-fg">{p.name}</span>
                <StatusDot status={p.status} size="md" />
              </button>
            {/each}
          </div>
        {/if}
      </div>

      <!-- Level tabs -->
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
            class="px-3 h-7 rounded-md text-[12px] font-medium transition-colors
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

      <!-- Search -->
      <div class="relative flex-1 min-w-[180px] max-w-sm">
        <Icon
          name="search"
          size={14}
          class="absolute left-2.5 top-1/2 -translate-y-1/2 text-fg-subtle"
        />
        <input
          type="text"
          bind:value={searchQuery}
          placeholder="Search logs…"
          class="w-full h-9 text-[13px] bg-surface-2 border border-border rounded-lg
                 pl-8 pr-2.5 text-fg placeholder:text-fg-subtle
                 focus:outline-none focus:ring-2 focus:ring-accent/40"
        />
      </div>
    </div>
  </header>

  <!-- Log surface -->
  <div class="flex-1 min-h-0 px-6 pb-6">
    <div
      class="flex flex-col h-full min-h-0 bg-surface border border-border rounded-xl overflow-hidden"
    >
      {#if !project}
        <div
          class="flex-1 flex flex-col items-center justify-center text-center text-fg-subtle gap-2 py-20"
        >
          <Icon name="file-text" size={28} class="opacity-40" />
          <p class="text-[13px]">
            {projects.value.length === 0
              ? "No registered projects. Add one to see its logs here."
              : "Select a project to view its logs."}
          </p>
        </div>
      {:else}
        <!-- Card header: name · status · port -->
        <header
          class="shrink-0 flex items-center gap-3 px-4 py-3 border-b border-border"
        >
          <h2 class="text-[13px] font-semibold text-fg">{project.name}</h2>
          <StatusPill status={project.status} />
          {#if project.port}
            <button
              type="button"
              onclick={copyUrl}
              title="Copy URL ({project.url})"
              class="ml-auto inline-flex items-center gap-1.5 text-[12px] font-mono
                     text-fg-muted hover:text-fg transition-colors"
            >
              <span class="tabular-nums">PORT {project.port}</span>
              <Icon name={copied ? "check" : "copy"} size={13} />
            </button>
          {/if}
        </header>

        <!-- Terminal body -->
        <div
          bind:this={scrollerEl}
          onscroll={onScroll}
          class="flex-1 min-h-0 overflow-y-auto bg-bg py-2 font-mono text-[12px] leading-[1.5] text-fg-muted"
        >
          {#if visible.length === 0}
            <p class="text-[12px] text-fg-subtle italic px-4 py-4">
              {loading
                ? "Loading log…"
                : parsed.length === 0
                  ? "No log output yet."
                  : "No lines match the current filter."}
            </p>
          {:else}
            {#each visible as pl, i (i)}
              <div
                class="flex gap-3 px-4 whitespace-pre-wrap break-words
                       {pl.level === 'error' ? 'bg-status-crashed/5 text-status-crashed' : ''}"
              >
                <span
                  class="shrink-0 w-12 select-none {tokenClass(pl.level)}"
                  >{pl.level.toUpperCase()}</span
                >
                <span class="min-w-0 flex-1">{@html pl.html}</span>
              </div>
            {/each}
          {/if}
        </div>

        <!-- Footer: auto-scroll · open in terminal -->
        <footer
          class="shrink-0 flex items-center gap-3 px-4 py-2.5 border-t border-border"
        >
          <label
            class="inline-flex items-center gap-2 text-[12px] text-fg-muted cursor-pointer select-none"
          >
            <input type="checkbox" bind:checked={autoScroll} class="accent-accent" />
            Auto-scroll
          </label>

          <span class="text-[11px] text-fg-subtle tabular-nums">
            {visible.length}
            {#if visible.length !== parsed.length}/ {parsed.length}{/if}
            line{visible.length === 1 ? "" : "s"}
          </span>

          <button
            type="button"
            onclick={openInTerminal}
            disabled={!terminalTool}
            title={terminalTool
              ? `Open in ${terminalTool.label}`
              : "No terminal detected"}
            class="ml-auto inline-flex items-center gap-2 h-8 px-3 rounded-lg text-[12px]
                   text-fg-muted hover:text-fg bg-surface-2/60 hover:bg-surface-2
                   border border-border/60 transition-colors
                   disabled:opacity-50 disabled:cursor-not-allowed"
          >
            <Icon name="terminal" size={14} />
            Open in Terminal
          </button>
        </footer>
      {/if}
    </div>
  </div>
</div>
