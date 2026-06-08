<!--
  SshTerminalTabs — the Terminal tab's content: a strip of shell tabs on one
  host, where each tab can be split into multiple side-by-side / stacked panes
  (VS Code-style). Multiple concurrent shells are the researcher/cluster workflow
  (run a job in one pane, watch `nvidia-smi -l` in another, edit in a third).

  Every pane of every tab stays mounted; inactive tabs are hidden (not destroyed)
  so background shells keep streaming and never re-authenticate. Splits within the
  active tab are all visible at once, resizable by dragging the dividers between
  them.

  Keyboard (routed up from the focused terminal so they never leak to the remote
  shell): Cmd/Ctrl+Shift+T new tab, +W close pane/tab, +E split side-by-side,
  +D split stacked, Cmd/Ctrl+1–9 jump, Cmd/Ctrl+Tab cycle.
-->
<script lang="ts">
  import { onMount } from "svelte";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import SshTerminalSession from "$lib/components/connections/SshTerminalSession.svelte";
  import { terminalLaunch } from "$lib/stores/terminalLaunch.svelte";
  import { terminalPrefs } from "$lib/stores/sshWorkspacePrefs.svelte";
  import type { TerminalShortcut } from "$lib/ssh/pty";

  let { connectionId, label }: { connectionId: string; label: string } = $props();

  /** A single shell pane. */
  interface Pane {
    key: number;
    title: string;
    exited: boolean;
    /** Run a program under the pty instead of a login shell (e.g. tmux attach). */
    command?: string;
  }
  /** A tab: one or more panes laid out in a direction, sized by percentage. */
  interface Tab {
    key: number;
    direction: "row" | "col";
    panes: Pane[];
    /** Per-pane flex weights (proportional); same length as `panes`. */
    sizes: number[];
    /** The focused pane's key within this tab. */
    activePane: number;
  }

  /** A tab never holds more panes than this, matching the reference IDE. */
  const MAX_PANES = 4;
  const FONT_MIN = 9;
  const FONT_MAX = 24;

  let tabs = $state<Tab[]>([]);
  let activeTabKey = $state<number | null>(null);
  let nextKey = 0;

  // Non-reactive map of pane key → session instance, for clear()/focus().
  const paneRefs: Record<number, SshTerminalSession | undefined> = {};

  const activeTab = $derived(tabs.find((t) => t.key === activeTabKey) ?? null);

  function mkPane(): Pane {
    return { key: nextKey++, title: label, exited: false };
  }

  function evenSizes(n: number): number[] {
    return Array.from({ length: n }, () => 100 / n);
  }

  function newTab() {
    const pane = mkPane();
    const tab: Tab = {
      key: nextKey++,
      direction: "row",
      panes: [pane],
      sizes: [100],
      activePane: pane.key,
    };
    tabs = [...tabs, tab];
    activeTabKey = tab.key;
  }

  /** Open a new tab whose single pane runs `command` under the pty (instead of a
      login shell) — used by the Jobs panel to attach a tmux/screen session or
      wrap a shell in a fresh tmux. The pane is seeded with a recognizable title
      until the program reports its own. */
  function launchCommandTab(command: string, title: string) {
    const pane: Pane = { key: nextKey++, title, exited: false, command };
    const tab: Tab = {
      key: nextKey++,
      direction: "row",
      panes: [pane],
      sizes: [100],
      activePane: pane.key,
    };
    tabs = [...tabs, tab];
    activeTabKey = tab.key;
  }

  // Consume launch requests addressed to this host's workspace. The store bumps
  // `seq` on every launch (so the same command twice still fires); we track the
  // last seq we handled to open each request exactly once. The request also
  // brings the Terminal panel forward (see terminalLaunch.launch).
  let lastLaunchSeq = 0;
  $effect(() => {
    const req = terminalLaunch.request;
    if (!req || req.connectionId !== connectionId || req.seq === lastLaunchSeq) return;
    lastLaunchSeq = req.seq;
    launchCommandTab(req.command, req.title);
    terminalLaunch.clear();
  });

  /** Split the active tab in `direction`, adding one pane. */
  function splitActive(direction: "row" | "col") {
    const t = activeTab;
    if (!t) {
      newTab();
      return;
    }
    if (t.panes.length >= MAX_PANES) return;
    const pane = mkPane();
    const panes = [...t.panes, pane];
    tabs = tabs.map((tab) =>
      tab.key === t.key
        ? { ...tab, direction, panes, sizes: evenSizes(panes.length), activePane: pane.key }
        : tab,
    );
  }

  function selectTab(key: number) {
    activeTabKey = key;
  }

  function setActivePane(tabKey: number, paneKey: number) {
    tabs = tabs.map((t) => (t.key === tabKey ? { ...t, activePane: paneKey } : t));
    if (tabKey !== activeTabKey) activeTabKey = tabKey;
  }

  function closeTab(key: number) {
    const idx = tabs.findIndex((t) => t.key === key);
    if (idx === -1) return;
    for (const p of tabs[idx].panes) delete paneRefs[p.key];
    tabs = tabs.filter((t) => t.key !== key);
    if (activeTabKey === key) {
      const neighbour = tabs[idx - 1] ?? tabs[idx] ?? null;
      activeTabKey = neighbour ? neighbour.key : null;
    }
  }

  function closePane(tabKey: number, paneKey: number) {
    const t = tabs.find((tab) => tab.key === tabKey);
    if (!t) return;
    if (t.panes.length <= 1) {
      closeTab(tabKey);
      return;
    }
    const idx = t.panes.findIndex((p) => p.key === paneKey);
    if (idx === -1) return;
    delete paneRefs[paneKey];
    const panes = t.panes.filter((p) => p.key !== paneKey);
    // Active pane falls back to the previous (else next) sibling.
    const activePane =
      t.activePane === paneKey ? (panes[idx - 1] ?? panes[idx] ?? panes[0]).key : t.activePane;
    tabs = tabs.map((tab) =>
      tab.key === tabKey ? { ...tab, panes, sizes: evenSizes(panes.length), activePane } : tab,
    );
  }

  /** Cmd/Ctrl+Shift+W: close the focused pane, or the whole tab if it's alone. */
  function closeActive() {
    const t = activeTab;
    if (!t) return;
    if (t.panes.length > 1) closePane(t.key, t.activePane);
    else closeTab(t.key);
  }

  function cycleTab(delta: number) {
    if (tabs.length === 0) return;
    const idx = tabs.findIndex((t) => t.key === activeTabKey);
    const next = (idx + delta + tabs.length) % tabs.length;
    activeTabKey = tabs[next].key;
  }

  function jumpTab(index: number) {
    if (index < tabs.length) activeTabKey = tabs[index].key;
  }

  function setTitle(tabKey: number, paneKey: number, title: string) {
    tabs = tabs.map((t) =>
      t.key === tabKey
        ? { ...t, panes: t.panes.map((p) => (p.key === paneKey ? { ...p, title } : p)) }
        : t,
    );
  }

  function markExited(tabKey: number, paneKey: number) {
    tabs = tabs.map((t) =>
      t.key === tabKey
        ? { ...t, panes: t.panes.map((p) => (p.key === paneKey ? { ...p, exited: true } : p)) }
        : t,
    );
  }

  function handleShortcut(shortcut: TerminalShortcut) {
    switch (shortcut.action) {
      case "new":
        newTab();
        break;
      case "close":
        closeActive();
        break;
      case "next":
        cycleTab(1);
        break;
      case "prev":
        cycleTab(-1);
        break;
      case "jump":
        jumpTab(shortcut.index);
        break;
      case "split":
        splitActive(shortcut.direction);
        break;
    }
  }

  /** Label shown on a tab button = its focused pane's title. */
  function tabTitle(t: Tab): string {
    return (t.panes.find((p) => p.key === t.activePane) ?? t.panes[0])?.title ?? label;
  }

  function tabExited(t: Tab): boolean {
    return t.panes.every((p) => p.exited);
  }

  function clearActivePane() {
    const t = activeTab;
    if (!t) return;
    paneRefs[t.activePane]?.clear();
  }

  function nudgeFont(delta: number) {
    const next = Math.min(FONT_MAX, Math.max(FONT_MIN, terminalPrefs.value.fontSize + delta));
    terminalPrefs.update({ fontSize: next });
  }

  // Drag a divider between pane `idx` and `idx+1`, adjusting their weights.
  function startResize(t: Tab, idx: number, e: PointerEvent) {
    e.preventDefault();
    const container = (e.currentTarget as HTMLElement).parentElement;
    if (!container) return;
    const isRow = t.direction === "row";
    const rect = container.getBoundingClientRect();
    const total = isRow ? rect.width : rect.height;
    if (total <= 0) return;
    const startPos = isRow ? e.clientX : e.clientY;
    const a = t.sizes[idx];
    const b = t.sizes[idx + 1];
    const MIN = 10;

    const move = (ev: PointerEvent) => {
      const pos = isRow ? ev.clientX : ev.clientY;
      const deltaPct = ((pos - startPos) / total) * 100;
      let na = a + deltaPct;
      let nb = b - deltaPct;
      if (na < MIN) {
        nb -= MIN - na;
        na = MIN;
      }
      if (nb < MIN) {
        na -= MIN - nb;
        nb = MIN;
      }
      t.sizes[idx] = na;
      t.sizes[idx + 1] = nb;
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      document.body.style.removeProperty("cursor");
      document.body.style.removeProperty("user-select");
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    document.body.style.cursor = isRow ? "col-resize" : "row-resize";
    document.body.style.userSelect = "none";
  }

  onMount(() => {
    newTab();
  });
</script>

<div class="flex h-full min-h-0 flex-col bg-surface">
  <!-- Tab strip + controls -->
  <div class="flex items-center gap-1 border-b border-border/60 bg-surface-2/30 px-2 py-1.5">
    <div class="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto">
      {#each tabs as t, i (t.key)}
        {@const isActive = t.key === activeTabKey}
        <div
          class="group flex shrink-0 items-center gap-1.5 rounded-md border px-2 py-1 text-[12px] transition-colors
                 {isActive
            ? 'border-border bg-surface text-fg'
            : 'border-transparent text-fg-muted hover:bg-surface/60 hover:text-fg'}"
        >
          <button
            type="button"
            onclick={() => selectTab(t.key)}
            class="flex min-w-0 items-center gap-1.5"
            title={`Tab ${i + 1}: ${tabTitle(t)}`}
          >
            <!-- No status dot — the icon itself dims when the shell has exited. -->
            <Icon name="terminal" size={12} class="shrink-0 {tabExited(t) ? 'text-status-stopped' : 'text-fg-subtle'}" />
            <span class="max-w-[160px] truncate font-mono text-[11.5px]">{tabTitle(t)}</span>
            {#if t.panes.length > 1}
              <span class="rounded bg-surface-2 px-1 text-[9px] font-semibold tabular-nums text-fg-subtle">
                {t.panes.length}
              </span>
            {/if}
          </button>
          <button
            type="button"
            onclick={() => closeTab(t.key)}
            class="shrink-0 rounded p-0.5 text-fg-subtle opacity-0 hover:bg-surface-2 hover:text-fg group-hover:opacity-100 {isActive ? 'opacity-100' : ''}"
            aria-label="Close tab"
          >
            <Icon name="x" size={12} />
          </button>
        </div>
      {/each}
    </div>

    <!-- Right-hand controls -->
    <div class="flex shrink-0 items-center gap-0.5">
      <button
        type="button"
        onclick={() => splitActive("row")}
        disabled={!activeTab || activeTab.panes.length >= MAX_PANES}
        class="grid h-7 w-7 place-items-center rounded-md text-fg-muted hover:bg-surface hover:text-fg disabled:opacity-40"
        aria-label="Split terminal"
        title="Split (⌘⇧E side-by-side · ⌘⇧D stacked)"
      >
        <Icon name="grid-2x2" size={14} />
      </button>
      <div class="mx-0.5 inline-flex items-center rounded-md border border-border/60">
        <button
          type="button"
          onclick={() => nudgeFont(-1)}
          disabled={terminalPrefs.value.fontSize <= FONT_MIN}
          class="grid h-6 w-6 place-items-center rounded-l-md text-[13px] font-semibold text-fg-muted hover:bg-surface hover:text-fg disabled:opacity-40"
          aria-label="Decrease font size"
          title="Decrease font size"
        >
          <Icon name="minus" size={13} />
        </button>
        <button
          type="button"
          onclick={() => nudgeFont(1)}
          disabled={terminalPrefs.value.fontSize >= FONT_MAX}
          class="grid h-6 w-6 place-items-center rounded-r-md text-[13px] font-semibold text-fg-muted hover:bg-surface hover:text-fg disabled:opacity-40"
          aria-label="Increase font size"
          title="Increase font size"
        >
          <Icon name="plus" size={13} />
        </button>
      </div>
      <button
        type="button"
        onclick={clearActivePane}
        disabled={!activeTab}
        class="grid h-7 w-7 place-items-center rounded-md text-fg-muted hover:bg-surface hover:text-fg disabled:opacity-40"
        aria-label="Clear terminal"
        title="Clear terminal"
      >
        <Icon name="eraser" size={14} />
      </button>
      <button
        type="button"
        onclick={newTab}
        class="grid h-7 w-7 place-items-center rounded-md text-fg-muted hover:bg-surface hover:text-fg"
        aria-label="New tab"
        title="New tab (⌘⇧T)"
      >
        <Icon name="plus" size={14} />
      </button>
    </div>
  </div>

  <!-- Tab bodies — all mounted; inactive ones hidden so they keep streaming. -->
  <div class="relative min-h-0 flex-1">
    {#if tabs.length === 0}
      <div class="flex h-full items-center justify-center">
        <button
          type="button"
          onclick={newTab}
          class="inline-flex items-center gap-2 rounded-lg border border-border px-3.5 py-2 text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg"
        >
          <Icon name="plus" size={14} /> New terminal
        </button>
      </div>
    {:else}
      {#each tabs as t (t.key)}
        {@const tabActive = t.key === activeTabKey}
        <div class="absolute inset-0" class:hidden={!tabActive}>
          <div class="flex h-full w-full {t.direction === 'row' ? 'flex-row' : 'flex-col'}">
            {#each t.panes as pane, i (pane.key)}
              {@const paneActive = tabActive && pane.key === t.activePane}
              <div
                class="group relative min-h-0 min-w-0"
                style="flex: {t.sizes[i]} 1 0"
              >
                <div
                  class="absolute inset-0 {t.panes.length > 1 && paneActive
                    ? 'ring-1 ring-inset ring-accent/70'
                    : ''}"
                >
                  <SshTerminalSession
                    bind:this={paneRefs[pane.key]}
                    {connectionId}
                    {label}
                    command={pane.command}
                    active={paneActive}
                    onTitle={(title) => setTitle(t.key, pane.key, title)}
                    onExit={() => markExited(t.key, pane.key)}
                    onShortcut={handleShortcut}
                    onFocus={() => setActivePane(t.key, pane.key)}
                  />
                </div>
                {#if t.panes.length > 1}
                  <button
                    type="button"
                    onclick={() => closePane(t.key, pane.key)}
                    class="absolute right-1.5 top-1.5 z-10 grid h-5 w-5 place-items-center rounded bg-surface/80 text-fg-subtle opacity-0 hover:bg-surface-2 hover:text-fg focus:opacity-100 group-hover:opacity-100 {paneActive ? 'opacity-70' : ''}"
                    aria-label="Close pane"
                    title="Close pane (⌘⇧W)"
                  >
                    <Icon name="x" size={12} />
                  </button>
                {/if}
              </div>
              {#if i < t.panes.length - 1}
                <!-- Divider between panes -->
                <div
                  role="separator"
                  aria-orientation={t.direction === "row" ? "vertical" : "horizontal"}
                  onpointerdown={(e) => startResize(t, i, e)}
                  class="group relative shrink-0 bg-border/60 hover:bg-accent/60
                    {t.direction === 'row' ? 'w-px cursor-col-resize' : 'h-px cursor-row-resize'}"
                >
                  <span
                    class="absolute {t.direction === 'row'
                      ? 'inset-y-0 -left-1 -right-1'
                      : 'inset-x-0 -top-1 -bottom-1'}"
                  ></span>
                </div>
              {/if}
            {/each}
          </div>
        </div>
      {/each}
    {/if}
  </div>
</div>
