<!--
  OpenInButton — "Open in…" dropdown for a project.

  Lists installed editors, agents, and terminals (detected by the Rust
  side via `installed_dev_tools`) and launches the project folder in the
  chosen tool via `open_in_ide`. Disabled with a tooltip when nothing is
  detected.

  Two visual modes:
    - `variant="full"`  (default) — full-width button with chevron and
      "Open in" label. Used inside the right rail's Quick Actions grid.
    - `variant="icon"`           — 28×28 icon-only square. Used in
      the projects table row so the action surface stays compact.

  The menu is rendered with `position: fixed` and anchored to the
  trigger's bounding rect so it can escape table/row clip contexts.
-->
<script lang="ts">
  import { onMount } from "svelte";

  import Icon, { type IconName } from "$lib/components/atoms/Icon.svelte";
  import { safeInvoke } from "$lib/ipc";
  import { devTools } from "$lib/stores/devTools.svelte";
  import { errorBus } from "$lib/stores/errors.svelte";
  import type { DevToolInfo, DevToolKind } from "$lib/types/devTools";

  interface Props {
    projectId: string;
    variant?: "full" | "icon";
    /** Restrict the menu to these tool kinds. Defaults to all kinds. */
    kinds?: DevToolKind[];
    /** Trigger label (full variant only). Defaults to "Open in". */
    label?: string;
  }
  let {
    projectId,
    variant = "full",
    kinds = ["editor", "agent", "terminal", "file-manager"],
    label = "Open in",
  }: Props = $props();

  const KIND_ORDER: DevToolKind[] = ["editor", "agent", "terminal", "file-manager"];
  const KIND_LABEL: Record<DevToolKind, string> = {
    editor: "Editors",
    agent: "Agents",
    terminal: "Terminals",
    "file-manager": "File Manager",
  };
  const KIND_ICON: Record<DevToolKind, IconName> = {
    editor: "pencil",
    agent: "sparkles",
    terminal: "terminal",
    "file-manager": "folder",
  };

  const TOOL_ICONS: Record<string, string> = {
    vscode: "/apps/vscode.png",
    cursor: "/apps/cursor.png",
    phpstorm: "/apps/phpstorm.png",
    sublime: "/apps/sublime-text.png",
    zed: "/apps/zed.png",
    xcode: "/apps/xcode.png",
    "android-studio": "/apps/android-studio.png",
    "claude-code": "/apps/claude.png",
    "claude-desktop": "/apps/claude.png",
    codex: "/apps/codex.png",
    antigravity: "/apps/antigravity.png",
    warp: "/apps/warp.png",
    ghostty: "/apps/ghostty.png",
    iterm: "/apps/iterm2.png",
    terminal: "/apps/terminal.png",
    finder: "/apps/finder.png",
  };

  const MENU_WIDTH = 200;

  let open = $state<boolean>(false);
  let triggerEl: HTMLButtonElement | null = $state(null);
  let menuEl: HTMLDivElement | null = $state(null);
  let menuTop = $state<number>(0);
  let menuLeft = $state<number>(0);

  onMount(() => {
    void devTools.start();
  });

  // Tools in scope for this instance (after the optional `kinds` filter).
  const available = $derived(devTools.value.filter((t) => kinds.includes(t.kind)));

  const groupedTools = $derived.by<
    { kind: DevToolKind; items: DevToolInfo[] }[]
  >(() => {
    const groups = new Map<DevToolKind, DevToolInfo[]>();
    for (const t of available) {
      const list = groups.get(t.kind) ?? [];
      list.push(t);
      groups.set(t.kind, list);
    }
    return KIND_ORDER.filter((k) => groups.has(k)).map((k) => ({
      kind: k,
      items: groups.get(k) ?? [],
    }));
  });

  function recomputePosition() {
    if (!triggerEl) return;
    const r = triggerEl.getBoundingClientRect();
    menuTop = r.bottom + 4;
    menuLeft = Math.max(8, r.right - MENU_WIDTH);
  }

  function toggle(e: MouseEvent) {
    e.stopPropagation();
    if (available.length === 0) return;
    if (!open) recomputePosition();
    open = !open;
  }

  async function openInTool(e: MouseEvent, tool: DevToolInfo) {
    e.stopPropagation();
    open = false;
    try {
      await safeInvoke("open_in_ide", { id: projectId, ide: tool.id });
      errorBus.push({
        code: "OPEN_IN_TOOL",
        category: "lifecycle",
        whatHappened: `Opening project in ${tool.label}.`,
        whyItMatters:
          import.meta.env.PUBLIC_SIMULATOR === "true"
            ? "In the desktop app this launches the project folder in your installed tool."
            : "The project folder was handed to your installed tool.",
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });
    } catch {
      /* safeInvoke pushes its own toast */
    }
  }

  function onWindowClick(e: MouseEvent) {
    if (!open) return;
    const t = e.target as Node | null;
    if (
      triggerEl?.contains(t ?? null) ||
      menuEl?.contains(t ?? null)
    ) {
      return;
    }
    open = false;
  }

  function onWindowKey(e: KeyboardEvent) {
    if (open && e.key === "Escape") {
      open = false;
      triggerEl?.focus();
    }
  }

  function onScrollOrResize() {
    if (open) open = false;
  }

  const tooltip = $derived(
    available.length === 0
      ? "No supported tools detected"
      : `${label} an installed tool`,
  );
</script>

<svelte:window
  onclick={onWindowClick}
  onkeydown={onWindowKey}
  onscroll={onScrollOrResize}
  onresize={onScrollOrResize}
/>

{#if variant === "icon"}
  <button
    bind:this={triggerEl}
    type="button"
    onclick={toggle}
    disabled={available.length === 0}
    aria-haspopup="menu"
    aria-expanded={open}
    title={tooltip}
    aria-label="{label}…"
    class="inline-flex items-center justify-center w-7 h-7 rounded-md
           text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors
           disabled:opacity-40 disabled:cursor-not-allowed disabled:hover:bg-transparent"
  >
    <Icon name="external-link" size={13} />
  </button>
{:else}
  <button
    bind:this={triggerEl}
    type="button"
    onclick={toggle}
    disabled={available.length === 0}
    aria-haspopup="menu"
    aria-expanded={open}
    title={tooltip}
    class="w-full inline-flex items-center gap-2 px-3 py-2 rounded-md
           border border-border bg-surface hover:bg-surface-2
           text-[12px] text-fg-muted hover:text-fg transition-colors
           disabled:opacity-50 disabled:cursor-not-allowed disabled:hover:bg-surface"
  >
    <Icon name="external-link" size={13} />
    <span class="flex-1 text-left">{label}</span>
    <Icon name="chevron-down" size={11} class="opacity-70" />
  </button>
{/if}

{#if open}
  <div
    bind:this={menuEl}
    role="menu"
    aria-label="Open this project in…"
    style:position="fixed"
    style:top="{menuTop}px"
    style:left="{menuLeft}px"
    style:width="{MENU_WIDTH}px"
    class="z-50 py-1 rounded-md border border-border bg-surface
           shadow-2xl text-[12px]"
  >
    {#each groupedTools as group (group.kind)}
      <div
        class="px-2 pt-1.5 pb-1 text-[10px] uppercase tracking-wide
               text-fg-subtle flex items-center gap-1.5"
      >
        <Icon name={KIND_ICON[group.kind]} size={10} />
        {KIND_LABEL[group.kind]}
      </div>
      {#each group.items as tool (tool.id)}
        <button
          type="button"
          role="menuitem"
          onclick={(e) => openInTool(e, tool)}
          class="w-full text-left px-2 py-1.5 rounded
                 text-fg-muted hover:bg-surface-2 hover:text-fg
                 transition-colors flex items-center gap-2"
        >
          {#if TOOL_ICONS[tool.id]}
            <img
              src={TOOL_ICONS[tool.id]}
              alt=""
              class="w-4 h-4 rounded-[3px] object-cover flex-shrink-0"
            />
          {:else}
            <span class="w-4 h-4 flex-shrink-0"></span>
          {/if}
          {tool.label}
        </button>
      {/each}
    {/each}
  </div>
{/if}
