<!--
  Tray popover — the rich-control alternative to the native NSMenu.
  Rendered into the `tray-panel` webview window (configured in
  tauri.conf.json: frameless, transparent, alwaysOnTop, hidden by
  default). The Rust tray module positions + shows this window when
  the user left-clicks the menu-bar icon; it auto-hides on blur.

  Layout (top to bottom):
    1. Header strip      — PortBay mark + aggregate status pill + cog
    2. Sticky action row — Start all · Stop all · Restart all
    3. Project list      — per-project status, name, hostname, CPU
                           sparkline, icon buttons (start/stop/restart/open)
    4. Footer            — aggregate CPU + memory bars and sparklines

  Background uses a frosted-glass treatment (semi-opaque + backdrop
  blur) so the transparent window reads cleanly over any wallpaper.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";

  import { safeInvoke } from "$lib/ipc";
  import { metrics } from "$lib/stores/metrics.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import { projectCpu } from "$lib/stores/projectCpu.svelte";
  import type { ProjectView } from "$lib/types/projects";
  import type { PortbayStatus } from "$lib/types/status";

  /** Live aggregate status used by the header pill and the tray icon colour. */
  const aggregate = $derived(computeAggregate(projects.value));

  /** Total of all per-project CPU% — capped to 100 for the bar width. */
  const aggregateCpu = $derived(
    projects.value.reduce((acc, p) => acc + (p.runtime?.cpuPercent ?? 0), 0),
  );

  /** Sum of per-project RSS, in MB. */
  const aggregateMemoryMb = $derived(
    projects.value.reduce(
      (acc, p) => acc + (p.runtime?.memBytes ?? 0) / (1024 * 1024),
      0,
    ),
  );

  /** System CPU sparkline source — rolling 60s of global CPU%. */
  const cpuHistory = $derived(metrics.cpuHistory);

  let busy = $state<Record<string, boolean>>({});

  onMount(() => {
    void projects.start();
    void metrics.start();
    projectCpu.start();

    return () => {
      // Don't fully stop projects/metrics — the main window may still
      // depend on them. Tear down only this view's listener.
      projectCpu.stop();
    };
  });

  function computeAggregate(items: ProjectView[]):
    | "idle"
    | "starting"
    | "running"
    | "error" {
    let anyStarting = false;
    let anyRunning = false;
    for (const p of items) {
      if (p.status === "crashed" || p.status === "port_conflict") return "error";
      if (p.status === "starting") anyStarting = true;
      if (p.status === "running" || p.status === "unhealthy") anyRunning = true;
    }
    if (anyStarting) return "starting";
    if (anyRunning) return "running";
    return "idle";
  }

  const aggregateLabel: Record<ReturnType<typeof computeAggregate>, string> = {
    idle: "Idle",
    starting: "Starting",
    running: "All healthy",
    error: "Needs attention",
  };

  const dotClass: Record<PortbayStatus, string> = {
    running: "bg-status-running",
    starting: "bg-status-starting",
    unhealthy: "bg-status-starting",
    crashed: "bg-status-crashed",
    port_conflict: "bg-status-crashed",
    stopped: "bg-fg-subtle/60",
  };

  async function startAll() {
    for (const p of projects.value) {
      if (p.status === "stopped" || p.status === "crashed") {
        void perProject(p.id, "start");
      }
    }
  }
  async function stopAll() {
    try {
      await safeInvoke("stop_all");
    } catch {
      /* toast already pushed */
    }
  }
  async function restartAll() {
    for (const p of projects.value) {
      if (
        p.status === "running" ||
        p.status === "unhealthy" ||
        p.status === "starting"
      ) {
        void perProject(p.id, "restart");
      }
    }
  }

  async function perProject(id: string, action: "start" | "stop" | "restart") {
    if (busy[id]) return;
    busy = { ...busy, [id]: true };
    try {
      await safeInvoke(`${action}_project`, { id });
    } catch {
      /* toast already pushed */
    } finally {
      busy = { ...busy, [id]: false };
    }
  }

  async function openProject(id: string) {
    try {
      await safeInvoke("open_project", { id });
    } catch {
      /* toast already pushed */
    }
  }

  async function showMainWindow(path?: string) {
    // Reveals the main window (creating it if it was destroyed) and
    // optionally pushes a route via the existing nav channel the tray
    // menu's "Preferences…" item uses.
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    try {
      // Find the main window from any window and show it.
      const { WebviewWindow } = await import("@tauri-apps/api/webviewWindow");
      const main = await WebviewWindow.getByLabel("main");
      if (main) {
        await main.unminimize();
        await main.show();
        await main.setFocus();
      }
      if (path) {
        const { emit } = await import("@tauri-apps/api/event");
        await emit("portbay://nav", path);
      }
      // Hide the popover so it doesn't linger over the main window.
      await getCurrentWindow().hide();
    } catch (e) {
      console.error("showMainWindow failed", e);
    }
  }

  /** Compact mm:ss-style memory label — kilobytes/megabytes/gigabytes. */
  function formatBytes(b: number): string {
    if (b < 1024) return `${b} B`;
    if (b < 1024 * 1024) return `${(b / 1024).toFixed(0)} KB`;
    if (b < 1024 * 1024 * 1024) return `${(b / (1024 * 1024)).toFixed(0)} MB`;
    return `${(b / (1024 * 1024 * 1024)).toFixed(1)} GB`;
  }

  /** SVG polyline points for a sparkline given a [0..N] history. */
  function sparkline(history: number[], width: number, height: number): string {
    if (history.length < 2) return "";
    const max = Math.max(1, ...history);
    const step = width / (history.length - 1);
    return history
      .map((v, i) => `${(i * step).toFixed(1)},${(height - (v / max) * height).toFixed(1)}`)
      .join(" ");
  }
</script>

<!--
  Frosted glass panel — let the OS wallpaper bleed through faintly
  while keeping content readable. The outer transparent webview shows
  the panel as a floating card with a subtle drop shadow (handled by
  Tauri's `shadow: true` in tauri.conf.json).
-->
<div
  class="h-full w-full overflow-hidden rounded-xl border border-border/40
         bg-bg/85 backdrop-blur-xl text-fg flex flex-col"
>
  <!-- 1. Header strip -->
  <header class="flex items-center justify-between px-4 py-3 border-b border-border/40">
    <div class="flex items-center gap-2">
      <span
        class="inline-block h-2 w-2 rounded-full transition-colors
               {aggregate === 'running'
          ? 'bg-status-running'
          : aggregate === 'starting'
            ? 'bg-status-starting'
            : aggregate === 'error'
              ? 'bg-status-crashed'
              : 'bg-fg-subtle/60'}"
        aria-hidden="true"
      ></span>
      <span class="text-sm font-semibold">PortBay</span>
      <span class="text-xs text-fg-muted">— {aggregateLabel[aggregate]}</span>
    </div>
    <button
      type="button"
      class="text-fg-muted hover:text-fg p-1 rounded transition-colors"
      onclick={() => showMainWindow("/settings")}
      aria-label="Open settings"
      title="Open settings"
    >
      <svg width="14" height="14" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">
        <path
          fill-rule="evenodd"
          d="M9.405 1.05a1 1 0 0 0-2.81 0l-.213 1.281a6 6 0 0 0-1.296.756L3.84 2.611a1 1 0 0 0-1.39 1.39l.476 1.246a6 6 0 0 0-.756 1.296L.89 6.756a1 1 0 0 0 0 2.488l1.281.213a6 6 0 0 0 .756 1.296l-.476 1.246a1 1 0 0 0 1.39 1.39l1.246-.476a6 6 0 0 0 1.296.756l.213 1.281a1 1 0 0 0 2.488 0l.213-1.281a6 6 0 0 0 1.296-.756l1.246.476a1 1 0 0 0 1.39-1.39l-.476-1.246a6 6 0 0 0 .756-1.296l1.281-.213a1 1 0 0 0 0-2.488l-1.281-.213a6 6 0 0 0-.756-1.296l.476-1.246a1 1 0 0 0-1.39-1.39l-1.246.476a6 6 0 0 0-1.296-.756L9.405 1.05ZM8 11a3 3 0 1 0 0-6 3 3 0 0 0 0 6Z"
        />
      </svg>
    </button>
  </header>

  <!-- 2. Sticky action row -->
  <div class="flex gap-1.5 px-4 py-2.5 border-b border-border/40 bg-bg/40">
    <button
      type="button"
      onclick={startAll}
      disabled={projects.value.length === 0}
      class="flex-1 flex items-center justify-center gap-1.5 px-3 py-1.5 rounded-md
             text-xs font-medium text-status-running bg-status-running/10
             hover:bg-status-running/20 disabled:opacity-40 disabled:cursor-not-allowed
             transition-colors"
    >
      <svg width="10" height="10" viewBox="0 0 10 10" fill="currentColor" aria-hidden="true">
        <path d="M2 1.5v7l6-3.5L2 1.5z" />
      </svg>
      Start all
    </button>
    <button
      type="button"
      onclick={stopAll}
      disabled={projects.value.length === 0}
      class="flex-1 flex items-center justify-center gap-1.5 px-3 py-1.5 rounded-md
             text-xs font-medium text-status-crashed bg-status-crashed/10
             hover:bg-status-crashed/20 disabled:opacity-40 disabled:cursor-not-allowed
             transition-colors"
    >
      <svg width="10" height="10" viewBox="0 0 10 10" fill="currentColor" aria-hidden="true">
        <rect x="2" y="2" width="6" height="6" rx="0.5" />
      </svg>
      Stop all
    </button>
    <button
      type="button"
      onclick={restartAll}
      disabled={projects.value.length === 0}
      class="flex-1 flex items-center justify-center gap-1.5 px-3 py-1.5 rounded-md
             text-xs font-medium text-fg-muted bg-surface-2/60
             hover:bg-surface-2 hover:text-fg transition-colors
             disabled:opacity-40 disabled:cursor-not-allowed"
    >
      <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.4" aria-hidden="true">
        <path d="M1.5 5a3.5 3.5 0 1 1 1 2.5" />
        <path d="M1 5V3M1 5h2" />
      </svg>
      Restart all
    </button>
  </div>

  <!-- 3. Project list -->
  <div class="flex-1 min-h-0 overflow-y-auto px-2 py-2 space-y-0.5">
    {#if projects.value.length === 0}
      <div class="px-3 py-6 text-center">
        <p class="text-xs text-fg-muted">No projects yet.</p>
        <button
          type="button"
          onclick={() => showMainWindow("/")}
          class="mt-2 text-xs text-accent hover:text-accent-hover"
        >
          Open dashboard to add one →
        </button>
      </div>
    {:else}
      {#each projects.value as project (project.id)}
        {@const history = projectCpu.historyFor(project.id)}
        {@const cpu = project.runtime?.cpuPercent ?? 0}
        {@const mem = project.runtime?.memBytes ?? 0}
        {@const isUp =
          project.status === "running" ||
          project.status === "unhealthy" ||
          project.status === "starting"}
        <div
          class="group flex items-center gap-2 px-2 py-1.5 rounded-md
                 hover:bg-surface-2/60 transition-colors"
        >
          <span
            class="inline-block h-1.5 w-1.5 rounded-full shrink-0 {dotClass[project.status]}"
            aria-hidden="true"
          ></span>
          <div class="min-w-0 flex-1">
            <div class="text-xs font-medium text-fg truncate">{project.name}</div>
            <div class="text-[10px] font-mono text-fg-subtle truncate">
              {project.hostname}
              {#if isUp}
                <span class="ml-1.5 text-fg-muted">· {cpu.toFixed(0)}% CPU</span>
                {#if mem > 0}
                  <span class="text-fg-muted">· {formatBytes(mem)}</span>
                {/if}
              {/if}
            </div>
          </div>
          {#if isUp && history.length >= 2}
            <svg
              width="36"
              height="14"
              viewBox="0 0 36 14"
              class="text-status-running/70 shrink-0"
              aria-hidden="true"
            >
              <polyline
                fill="none"
                stroke="currentColor"
                stroke-width="1"
                stroke-linejoin="round"
                stroke-linecap="round"
                points={sparkline(history, 36, 12)}
              />
            </svg>
          {/if}
          <div class="flex items-center gap-0.5 shrink-0 opacity-0 group-hover:opacity-100 transition-opacity">
            {#if isUp}
              <button
                type="button"
                onclick={() => perProject(project.id, "restart")}
                disabled={busy[project.id]}
                title="Restart"
                class="p-1 rounded text-fg-muted hover:text-fg hover:bg-surface-2 disabled:opacity-40 transition-colors"
              >
                <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.4" aria-hidden="true">
                  <path d="M1.5 5a3.5 3.5 0 1 1 1 2.5" />
                  <path d="M1 5V3M1 5h2" />
                </svg>
              </button>
              <button
                type="button"
                onclick={() => perProject(project.id, "stop")}
                disabled={busy[project.id]}
                title="Stop"
                class="p-1 rounded text-status-crashed/80 hover:text-status-crashed hover:bg-status-crashed/10 disabled:opacity-40 transition-colors"
              >
                <svg width="10" height="10" viewBox="0 0 10 10" fill="currentColor" aria-hidden="true">
                  <rect x="2" y="2" width="6" height="6" rx="0.5" />
                </svg>
              </button>
            {:else}
              <button
                type="button"
                onclick={() => perProject(project.id, "start")}
                disabled={busy[project.id]}
                title="Start"
                class="p-1 rounded text-status-running/80 hover:text-status-running hover:bg-status-running/10 disabled:opacity-40 transition-colors"
              >
                <svg width="10" height="10" viewBox="0 0 10 10" fill="currentColor" aria-hidden="true">
                  <path d="M2 1.5v7l6-3.5L2 1.5z" />
                </svg>
              </button>
            {/if}
            <button
              type="button"
              onclick={() => openProject(project.id)}
              disabled={!isUp}
              title="Open in browser"
              class="p-1 rounded text-fg-muted hover:text-fg hover:bg-surface-2 disabled:opacity-30 transition-colors"
            >
              <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.4" aria-hidden="true">
                <path d="M3 1h5v5M8 1L4 5M5 2H1.5v6.5H8V6" />
              </svg>
            </button>
          </div>
        </div>
      {/each}
    {/if}
  </div>

  <!-- 4. System load footer -->
  <footer class="px-4 py-3 border-t border-border/40 bg-bg/50 space-y-2.5">
    <!-- Aggregate CPU -->
    <div class="flex items-center gap-2">
      <span class="text-[10px] font-mono uppercase tracking-wider text-fg-subtle w-9">CPU</span>
      <div class="flex-1 h-1.5 rounded-full bg-surface-2/60 overflow-hidden">
        <div
          class="h-full bg-status-running/80 transition-all duration-500"
          style:width={`${Math.min(100, aggregateCpu)}%`}
        ></div>
      </div>
      <span class="text-[10px] font-mono text-fg-muted w-10 text-right">
        {aggregateCpu.toFixed(0)}%
      </span>
      <svg width="40" height="14" viewBox="0 0 40 14" class="text-status-running/70 shrink-0" aria-hidden="true">
        {#if cpuHistory.length >= 2}
          <polyline
            fill="none"
            stroke="currentColor"
            stroke-width="1"
            stroke-linejoin="round"
            stroke-linecap="round"
            points={sparkline(cpuHistory, 40, 12)}
          />
        {/if}
      </svg>
    </div>

    <!-- Aggregate memory -->
    <div class="flex items-center gap-2">
      <span class="text-[10px] font-mono uppercase tracking-wider text-fg-subtle w-9">MEM</span>
      <div class="flex-1 h-1.5 rounded-full bg-surface-2/60 overflow-hidden">
        <div
          class="h-full bg-accent/70 transition-all duration-500"
          style:width={`${
            metrics.value
              ? Math.min(
                  100,
                  (metrics.value.memory.usedBytes / metrics.value.memory.totalBytes) * 100,
                )
              : 0
          }%`}
        ></div>
      </div>
      <span class="text-[10px] font-mono text-fg-muted w-16 text-right">
        {#if aggregateMemoryMb > 0}
          {aggregateMemoryMb.toFixed(0)} MB
        {:else if metrics.value}
          {formatBytes(metrics.value.memory.usedBytes)}
        {:else}
          —
        {/if}
      </span>
      <span class="w-10"></span>
    </div>
  </footer>
</div>

<style>
  /*
    Make the body backdrop transparent so the Tauri webview's
    transparent: true setting can show through to the OS desktop.
    Scoped via :global() — only applies when this page is mounted.
  */
  :global(html),
  :global(body) {
    background: transparent !important;
  }
</style>
