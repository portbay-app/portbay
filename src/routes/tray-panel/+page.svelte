<!--
  Tray popover — the rich-control surface behind the menu-bar icon.
  Rendered into the `tray-panel` webview window (configured in
  tauri.conf.json: frameless, transparent, alwaysOnTop, hidden by
  default). The Rust tray module positions + shows this window when the
  user left-clicks the menu-bar icon; it auto-hides on blur.

  Layout (top to bottom):
    1. Header        — Portbay wordmark + aggregate status + settings cog
    2. Nav grid      — Dashboard · Domains · Databases · Logs launchers
    3. Stat cards    — system CPU (sparkline) · Memory (ring) · Disk (ring)
    4. Running Now   — live list of up projects; click a host to open it
    5. Quick Controls— Start all · Stop all · Restart all
    6. Footer menu   — Preferences · Quit Portbay (⌘Q)

  Privileged window ops (reveal/focus the main window, navigate, quit)
  are routed through the `open_main_window` / `quit_app` app commands so
  this webview keeps a minimal capability surface — see
  src-tauri/capabilities/tray-panel.json.
-->
<script lang="ts">
  import { onMount } from "svelte";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import type { IconName } from "$lib/components/atoms/Icon.svelte";
  import LighthouseLogo from "$lib/components/atoms/LighthouseLogo.svelte";
  import { safeInvoke } from "$lib/ipc";
  import { metrics } from "$lib/stores/metrics.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import type { ProjectView } from "$lib/types/projects";
  import type { PortbayStatus } from "$lib/types/status";

  /** Live aggregate status used by the header dot + label. */
  const aggregate = $derived(computeAggregate(projects.value));

  /** Rolling system-CPU history (60 × 2s) for the CPU card sparkline. */
  const cpuHistory = $derived(metrics.cpuHistory);

  /** Projects currently up — the "Running Now" list and service count. */
  const runningProjects = $derived(projects.value.filter((p) => isUp(p.status)));

  let busy = $state(false);

  onMount(() => {
    void projects.start();
    void metrics.start();

    // Each Tauri webview has its own JS context. Status events fire to
    // every listener, but if the popover was hidden when state changed
    // in the main window it can open stale. Defensive: refresh on every
    // focus — one cheap `list_projects` IPC that guarantees the dots
    // match the dashboard.
    let unlistenFocus: (() => void) | null = null;
    void (async () => {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      const win = getCurrentWindow();
      unlistenFocus = await win.onFocusChanged(({ payload: focused }) => {
        if (focused) void projects.refresh();
      });
    })();

    return () => {
      // Don't fully stop projects/metrics — the main window may still
      // depend on them. Tear down only this view's listener.
      unlistenFocus?.();
    };
  });

  function isUp(status: PortbayStatus): boolean {
    return status === "running" || status === "unhealthy" || status === "starting";
  }

  function computeAggregate(
    items: ProjectView[],
  ): "idle" | "starting" | "running" | "error" {
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
    running: "Running",
    error: "Needs attention",
  };

  /** Header status dot + label colour, keyed off the aggregate. */
  const aggregateColor = $derived(
    aggregate === "running"
      ? "text-status-running"
      : aggregate === "starting"
        ? "text-status-starting"
        : aggregate === "error"
          ? "text-status-crashed"
          : "text-fg-subtle",
  );

  const dotClass: Record<PortbayStatus, string> = {
    running: "bg-status-running",
    starting: "bg-status-starting",
    unhealthy: "bg-status-starting",
    crashed: "bg-status-crashed",
    port_conflict: "bg-status-crashed",
    stopped: "bg-fg-subtle/60",
  };

  /** Launcher cards — each reveals the main window on its route. */
  const navItems: { label: string; icon: IconName; route: string; color: string }[] = [
    { label: "Dashboard", icon: "home", route: "/", color: "#818cf8" },
    { label: "Domains", icon: "globe", route: "/domains", color: "#4d9cff" },
    { label: "Databases", icon: "database", route: "/databases", color: "#2dd4bf" },
    { label: "Logs", icon: "terminal", route: "/logs", color: "#a78bfa" },
  ];

  async function startAll() {
    if (busy) return;
    busy = true;
    try {
      for (const p of projects.value) {
        if (p.status === "stopped" || p.status === "crashed") {
          void safeInvoke("start_project", { id: p.id });
        }
      }
    } finally {
      busy = false;
    }
  }

  async function stopAll() {
    if (busy) return;
    busy = true;
    try {
      await safeInvoke("stop_all");
    } catch {
      /* toast already pushed */
    } finally {
      busy = false;
    }
  }

  async function restartAll() {
    if (busy) return;
    busy = true;
    try {
      for (const p of projects.value) {
        if (isUp(p.status)) void safeInvoke("restart_project", { id: p.id });
      }
    } finally {
      busy = false;
    }
  }

  /** Open a project's URL in the default browser. */
  async function openProject(id: string) {
    try {
      await safeInvoke("open_project", { id });
    } catch {
      /* toast already pushed */
    }
  }

  /** Reveal the main window (optionally on a route) and dismiss the popover. */
  async function openMain(path?: string) {
    try {
      await safeInvoke("open_main_window", { path: path ?? null });
    } catch {
      /* toast already pushed */
    }
  }

  async function quitApp() {
    try {
      await safeInvoke("quit_app");
    } catch {
      /* toast already pushed */
    }
  }

  function onKeydown(e: KeyboardEvent) {
    // Honour the ⌘Q hint shown next to "Quit Portbay".
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "q") {
      e.preventDefault();
      void quitApp();
    }
  }

  /** Compact GB label — one decimal under 10 GB, whole numbers above. */
  function gb(bytes: number, whole = false): string {
    const g = bytes / 1024 ** 3;
    if (whole || g >= 10) return g.toFixed(0);
    return g.toFixed(1);
  }

  /** Fraction (0..1) of a used/total pair, clamped. */
  function frac(used: number, total: number): number {
    if (total <= 0) return 0;
    return Math.max(0, Math.min(1, used / total));
  }

  /** stroke-dasharray for a ring of radius `r` filled to `f` (0..1). */
  function ringDash(f: number, r: number): string {
    const c = 2 * Math.PI * r;
    return `${(f * c).toFixed(2)} ${c.toFixed(2)}`;
  }

  /** Polyline points for a sparkline over `history`, scaled to w×h. */
  function sparkPoints(history: number[], w: number, h: number): string {
    if (history.length < 2) return "";
    const max = Math.max(1, ...history);
    const step = w / (history.length - 1);
    return history
      .map((v, i) => `${(i * step).toFixed(1)},${(h - (v / max) * h).toFixed(1)}`)
      .join(" ");
  }

  /** Closed area path under the sparkline, for the gradient fill. */
  function sparkArea(history: number[], w: number, h: number): string {
    const pts = sparkPoints(history, w, h);
    if (!pts) return "";
    return `M0,${h} L${pts.split(" ").join(" L")} L${w},${h} Z`;
  }

  const cpuTotal = $derived(metrics.value?.cpu.total ?? 0);
  const memFrac = $derived(
    metrics.value ? frac(metrics.value.memory.usedBytes, metrics.value.memory.totalBytes) : 0,
  );
  const diskFrac = $derived(
    metrics.value ? frac(metrics.value.disk.usedBytes, metrics.value.disk.totalBytes) : 0,
  );
</script>

<svelte:window onkeydown={onKeydown} />

<!--
  Frosted-glass panel — the transparent webview lets the OS wallpaper
  bleed through faintly while content stays readable. The drop shadow
  is drawn by Tauri (`shadow: true`).
-->
<div
  class="flex h-full w-full flex-col overflow-hidden rounded-2xl border border-border/50
         bg-bg/90 text-fg backdrop-blur-2xl"
>
  <!-- 1. Header -->
  <header class="flex items-center justify-between px-5 pt-5 pb-4">
    <div class="flex items-center gap-2.5">
      <LighthouseLogo size={30} />
      <h1
        class="text-[22px] font-extrabold leading-none tracking-tight text-fg"
        style="font-family: 'Nunito', Inter, ui-sans-serif"
      >
        Portbay
      </h1>
      <span class="flex items-center gap-1.5 {aggregateColor}">
        <span class="inline-block h-2 w-2 rounded-full bg-current"></span>
        <span class="text-sm font-medium">{aggregateLabel[aggregate]}</span>
      </span>
    </div>
    <button
      type="button"
      onclick={() => openMain("/settings")}
      aria-label="Open settings"
      title="Settings"
      class="flex h-9 w-9 items-center justify-center rounded-lg border border-border/60
             bg-surface/40 text-fg-muted transition-colors hover:border-border-strong
             hover:bg-surface-2 hover:text-fg"
    >
      <Icon name="settings" size={17} />
    </button>
  </header>

  <!-- 2. Nav grid -->
  <nav class="grid grid-cols-4 gap-2.5 px-5">
    {#each navItems as item (item.route)}
      <button
        type="button"
        onclick={() => openMain(item.route)}
        class="group relative flex flex-col items-center gap-2 overflow-hidden rounded-xl
               border border-border/60 bg-surface/40 py-3.5 transition-colors
               hover:border-accent/50 hover:bg-surface-2"
      >
        <span
          class="absolute inset-x-5 top-0 h-0.5 rounded-full bg-accent opacity-0
                 transition-opacity group-hover:opacity-100"
        ></span>
        <!-- lucide strokes with currentColor; tint per item via `color`. -->
        <span class="transition-transform group-hover:scale-110" style:color={item.color}>
          <Icon name={item.icon} size={22} strokeWidth={1.75} />
        </span>
        <span class="text-xs font-medium text-fg-muted group-hover:text-fg">{item.label}</span>
      </button>
    {/each}
  </nav>

  <!-- 3. Stat cards -->
  <section class="grid grid-cols-3 gap-2.5 px-5 pt-3">
    <!-- CPU -->
    <div class="relative flex h-[86px] flex-col rounded-xl border border-border/60 bg-surface/40 p-3">
      <div class="flex items-center justify-between">
        <span class="text-xs font-medium text-fg-muted">CPU</span>
        <span class="inline-block h-1.5 w-1.5 rounded-full bg-status-running"></span>
      </div>
      <svg
        class="absolute inset-x-0 bottom-7 h-9 w-full"
        viewBox="0 0 100 36"
        preserveAspectRatio="none"
        aria-hidden="true"
      >
        <defs>
          <linearGradient id="cpuGrad" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stop-color="#2ee36b" stop-opacity="0.4" />
            <stop offset="100%" stop-color="#2ee36b" stop-opacity="0" />
          </linearGradient>
        </defs>
        {#if cpuHistory.length >= 2}
          <path d={sparkArea(cpuHistory, 100, 36)} fill="url(#cpuGrad)" />
          <polyline
            points={sparkPoints(cpuHistory, 100, 36)}
            fill="none"
            stroke="#2ee36b"
            stroke-width="1.5"
            stroke-linejoin="round"
            stroke-linecap="round"
            vector-effect="non-scaling-stroke"
          />
        {/if}
      </svg>
      <span class="mt-auto self-end text-lg font-semibold tabular-nums text-fg">
        {cpuTotal.toFixed(0)}%
      </span>
    </div>

    <!-- Memory -->
    <div class="flex h-[86px] flex-col rounded-xl border border-border/60 bg-surface/40 p-3">
      <span class="text-xs font-medium text-fg-muted">Memory</span>
      <div class="mt-auto flex items-center gap-2">
        <svg width="30" height="30" viewBox="0 0 38 38" class="-rotate-90 shrink-0" aria-hidden="true">
          <defs>
            <linearGradient id="memGrad" x1="0" y1="0" x2="1" y2="1">
              <stop offset="0%" stop-color="#4d9cff" />
              <stop offset="100%" stop-color="#a78bfa" />
            </linearGradient>
          </defs>
          <circle cx="19" cy="19" r="14" fill="none" class="text-surface-2" stroke="currentColor" stroke-width="4" />
          <circle
            cx="19"
            cy="19"
            r="14"
            fill="none"
            stroke="url(#memGrad)"
            stroke-width="4"
            stroke-linecap="round"
            stroke-dasharray={ringDash(memFrac, 14)}
          />
        </svg>
        <div class="min-w-0 leading-tight">
          {#if metrics.value}
            <div class="flex items-baseline gap-0.5 whitespace-nowrap text-fg">
              <span class="text-sm font-semibold tabular-nums">{gb(metrics.value.memory.usedBytes)}</span>
              <span class="text-[10px] font-medium text-fg-muted">GB</span>
            </div>
            <div class="whitespace-nowrap text-[10px] tabular-nums text-fg-subtle">/ {gb(metrics.value.memory.totalBytes, true)} GB</div>
          {:else}
            <div class="text-sm font-semibold text-fg-subtle">—</div>
          {/if}
        </div>
      </div>
    </div>

    <!-- Disk -->
    <div class="flex h-[86px] flex-col rounded-xl border border-border/60 bg-surface/40 p-3">
      <span class="text-xs font-medium text-fg-muted">Disk</span>
      <div class="mt-auto flex items-center gap-2">
        <svg width="30" height="30" viewBox="0 0 38 38" class="-rotate-90 shrink-0 text-status-running" aria-hidden="true">
          <circle cx="19" cy="19" r="14" fill="none" class="text-surface-2" stroke="currentColor" stroke-width="4" />
          <circle
            cx="19"
            cy="19"
            r="14"
            fill="none"
            stroke="currentColor"
            stroke-width="4"
            stroke-linecap="round"
            stroke-dasharray={ringDash(diskFrac, 14)}
          />
        </svg>
        <div class="min-w-0 leading-tight">
          {#if metrics.value}
            <div class="flex items-baseline gap-0.5 whitespace-nowrap text-fg">
              <span class="text-sm font-semibold tabular-nums">{gb(metrics.value.disk.usedBytes, true)}</span>
              <span class="text-[10px] font-medium text-fg-muted">GB</span>
            </div>
            <div class="whitespace-nowrap text-[10px] tabular-nums text-fg-subtle">/ {gb(metrics.value.disk.totalBytes, true)} GB</div>
          {:else}
            <div class="text-sm font-semibold text-fg-subtle">—</div>
          {/if}
        </div>
      </div>
    </div>
  </section>

  <!-- 4. Running Now -->
  <section class="mx-5 mt-4 flex min-h-0 flex-1 flex-col rounded-xl border border-border/60 bg-surface/40">
    <div class="flex items-center justify-between px-4 pt-3 pb-2">
      <h2 class="text-sm font-semibold text-fg">Running Now</h2>
      <span class="flex items-center gap-1.5 text-xs text-fg-muted">
        {runningProjects.length}
        {runningProjects.length === 1 ? "Service" : "Services"}
        <span
          class="inline-block h-1.5 w-1.5 rounded-full {runningProjects.length > 0
            ? 'bg-status-running'
            : 'bg-fg-subtle/50'}"
        ></span>
      </span>
    </div>
    <div class="min-h-0 flex-1 overflow-y-auto px-2 pb-2">
      {#if runningProjects.length === 0}
        <div class="px-2 py-5 text-center">
          <p class="text-xs text-fg-muted">No services running.</p>
          <button
            type="button"
            onclick={startAll}
            disabled={projects.value.length === 0 || busy}
            class="mt-1.5 text-xs font-medium text-accent transition-colors hover:text-accent-hover
                   disabled:opacity-40"
          >
            {projects.value.length === 0 ? "Add a project →" : "Start all →"}
          </button>
        </div>
      {:else}
        {#each runningProjects as project (project.id)}
          <div class="flex items-center gap-2.5 rounded-lg px-2 py-1.5">
            <span
              class="inline-block h-1.5 w-1.5 shrink-0 rounded-full {dotClass[project.status]}"
              aria-hidden="true"
            ></span>
            <button
              type="button"
              onclick={() => openProject(project.id)}
              title="Open {project.hostname} in browser"
              class="min-w-0 flex-1 truncate text-left text-[13px] text-fg transition-colors
                     hover:text-accent hover:underline"
            >
              {project.hostname}
            </button>
            {#if project.port}
              <span class="shrink-0 text-[13px] tabular-nums text-fg-muted">{project.port}</span>
            {/if}
          </div>
        {/each}
      {/if}
    </div>
  </section>

  <!-- 5. Quick Controls -->
  <section class="px-5 pt-4">
    <h2 class="mb-2 text-sm font-semibold text-fg">Quick Controls</h2>
    <div class="grid grid-cols-3 gap-2.5">
      <button
        type="button"
        onclick={startAll}
        disabled={projects.value.length === 0 || busy}
        class="flex items-center justify-center gap-1.5 rounded-xl border border-status-running/30
               bg-status-running/10 py-2.5 text-xs font-medium text-status-running transition-colors
               hover:bg-status-running/20 disabled:cursor-not-allowed disabled:opacity-40"
      >
        <Icon name="play" size={13} />
        Start All
      </button>
      <button
        type="button"
        onclick={stopAll}
        disabled={runningProjects.length === 0 || busy}
        class="flex items-center justify-center gap-1.5 rounded-xl border border-status-crashed/30
               bg-status-crashed/10 py-2.5 text-xs font-medium text-status-crashed transition-colors
               hover:bg-status-crashed/20 disabled:cursor-not-allowed disabled:opacity-40"
      >
        <Icon name="square" size={13} />
        Stop All
      </button>
      <button
        type="button"
        onclick={restartAll}
        disabled={runningProjects.length === 0 || busy}
        class="flex items-center justify-center gap-1.5 rounded-xl border border-accent/30
               bg-accent/10 py-2.5 text-xs font-medium text-accent transition-colors
               hover:bg-accent/20 disabled:cursor-not-allowed disabled:opacity-40"
      >
        <Icon name="refresh-cw" size={13} />
        Restart All
      </button>
    </div>
  </section>

  <!-- 6. Footer menu -->
  <footer class="mt-3 border-t border-border/50 px-2 py-2">
    <button
      type="button"
      onclick={() => openMain("/settings")}
      class="flex w-full items-center gap-3 rounded-lg px-3 py-2 text-fg-muted
             transition-colors hover:bg-surface-2 hover:text-fg"
    >
      <Icon name="sliders-horizontal" size={16} class="shrink-0" />
      <span class="flex-1 text-left text-[13px]">Preferences</span>
      <Icon name="chevron-right" size={15} class="text-fg-subtle" />
    </button>
    <button
      type="button"
      onclick={quitApp}
      class="flex w-full items-center gap-3 rounded-lg px-3 py-2 text-fg-muted
             transition-colors hover:bg-surface-2 hover:text-fg"
    >
      <Icon name="power" size={16} class="shrink-0" />
      <span class="flex-1 text-left text-[13px]">Quit Portbay</span>
      <span class="text-[11px] tabular-nums text-fg-subtle">⌘Q</span>
    </button>
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
