<!--
  CommandPalette — ⌘K spotlight-style search across every action in the app.

  Opens via `palette.show()` (TopBar pill + global ⌘K hotkey). Renders
  a centered modal with an autofocused input and a grouped result list
  (max 8 visible, scrollable). Up/Down navigates, Enter executes,
  Escape closes. Recents bubble to the top when the input is empty.

  Match algorithm: lowercase substring score across label + detail +
  keywords + group. Empty query → recents first, then everything
  grouped by their declared group; non-empty → ranked by score, no
  group headers (visual noise — the user already typed what they want).
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { trapFocus } from "$lib/actions/trapFocus";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import { collectCommands, executeCommand } from "$lib/commands";
  import { palette } from "$lib/stores/palette.svelte";
  import type { PaletteCommand, PaletteGroup } from "$lib/types/palette";

  let inputEl: HTMLInputElement | null = $state(null);
  let listEl: HTMLDivElement | null = $state(null);
  let selectedIndex = $state<number>(0);

  /** Global ⌘K / Ctrl-K opener + ESC-to-close. Lives at this scope so
   *  it works from any route without each page wiring its own listener. */
  function handleGlobalKey(e: KeyboardEvent) {
    // Open: ⌘K (mac) or Ctrl-K (linux/windows).
    if (
      (e.key === "k" || e.key === "K") &&
      (e.metaKey || e.ctrlKey) &&
      !e.altKey &&
      !e.shiftKey
    ) {
      e.preventDefault();
      palette.show();
      return;
    }
  }

  onMount(() => {
    window.addEventListener("keydown", handleGlobalKey);
    return () => window.removeEventListener("keydown", handleGlobalKey);
  });

  // Reset selection + autofocus the input every time the palette opens.
  $effect(() => {
    if (palette.isOpen) {
      selectedIndex = 0;
      queueMicrotask(() => inputEl?.focus());
    }
  });

  // Re-collect commands every render — store reads keep this reactive.
  const allCommands = $derived<PaletteCommand[]>(
    palette.isOpen ? collectCommands() : [],
  );

  /** Fuzzy / substring scoring. Higher score = better match. */
  function scoreCommand(cmd: PaletteCommand, q: string): number {
    if (!q) return 0;
    const haystack = (
      cmd.label +
      " " +
      (cmd.detail ?? "") +
      " " +
      (cmd.keywords ?? []).join(" ") +
      " " +
      cmd.group
    ).toLowerCase();

    let score = 0;
    // Whole-query substring is the strongest signal.
    if (haystack.includes(q)) score += 100;
    // Exact prefix of the label is even stronger.
    if (cmd.label.toLowerCase().startsWith(q)) score += 50;
    // Per-token substring picks up multi-word queries.
    const tokens = q.split(/\s+/).filter(Boolean);
    for (const tok of tokens) {
      if (haystack.includes(tok)) score += 20;
    }
    return score;
  }

  interface RankedItem {
    cmd: PaletteCommand;
    score: number;
  }

  /** Filter + sort the command set. Empty query → recents first
   *  followed by the unsorted list (preserving registration order
   *  so groups stay visually clustered). */
  const results = $derived.by<RankedItem[]>(() => {
    const q = palette.query.trim().toLowerCase();
    if (!q) {
      const byId = new Map(allCommands.map((c) => [c.id, c]));
      const recentCmds: PaletteCommand[] = [];
      for (const id of palette.recents) {
        const c = byId.get(id);
        if (c) {
          recentCmds.push(c);
          byId.delete(id);
        }
      }
      return [
        ...recentCmds.map((c) => ({ cmd: c, score: 0 })),
        ...Array.from(byId.values()).map((c) => ({ cmd: c, score: 0 })),
      ];
    }
    const scored = allCommands
      .map((cmd) => ({ cmd, score: scoreCommand(cmd, q) }))
      .filter((r) => r.score > 0);
    scored.sort((a, b) => b.score - a.score);
    return scored;
  });

  /** Whether to show group headers between rows. Only when the query
   *  is empty AND the recents block is rendered separately. */
  const showGroupHeaders = $derived(palette.query.trim() === "");

  /** Number of recent rows at the head of the list — we treat them as
   *  one synthetic "Recents" section. */
  const recentCount = $derived.by(() => {
    if (palette.query.trim() !== "") return 0;
    return palette.recents.filter((id) =>
      allCommands.some((c) => c.id === id),
    ).length;
  });

  function selectPrev() {
    if (results.length === 0) return;
    selectedIndex = (selectedIndex - 1 + results.length) % results.length;
    scrollSelectedIntoView();
  }

  function selectNext() {
    if (results.length === 0) return;
    selectedIndex = (selectedIndex + 1) % results.length;
    scrollSelectedIntoView();
  }

  function scrollSelectedIntoView() {
    queueMicrotask(() => {
      const el = listEl?.querySelector<HTMLElement>(`[data-palette-row="${selectedIndex}"]`);
      el?.scrollIntoView({ block: "nearest" });
    });
  }

  async function execute(index: number) {
    const item = results[index];
    if (!item) return;
    // Close BEFORE running so route changes / modals render under a
    // clean DOM. The command id is recorded after — markUsed is
    // sync localStorage write.
    palette.markUsed(item.cmd.id);
    palette.hide();
    await executeCommand(item.cmd);
  }

  function onInputKey(e: KeyboardEvent) {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      selectNext();
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      selectPrev();
    } else if (e.key === "Enter") {
      e.preventDefault();
      void execute(selectedIndex);
    } else if (e.key === "Escape") {
      e.preventDefault();
      palette.hide();
    }
  }

  /** Reset selection when the query changes — keeps highlight on the
   *  current best match instead of an off-screen leftover. */
  $effect(() => {
    const _q = palette.query;
    selectedIndex = 0;
  });

  /** Render group headers only at boundaries — for the empty-query
   *  view. Returns the group name to show above row `i`, or null when
   *  no header is needed (same group as the previous row). */
  function headerAt(i: number): string | null {
    if (!showGroupHeaders) return null;
    if (i < recentCount) {
      return i === 0 ? "Recent" : null;
    }
    const cur = results[i]?.cmd.group;
    const prev = results[i - 1]?.cmd.group;
    if (i === recentCount) return cur ?? null;
    if (cur && cur !== prev) return cur;
    return null;
  }

  const groupLabel: Record<PaletteGroup, string> = {
    Projects: "Projects",
    Groups: "Groups",
    Sidecars: "Sidecars",
    Navigation: "Navigation",
    PHP: "PHP",
    Tunnels: "Tunnels",
    App: "App",
  };
</script>

{#if palette.isOpen}
  <div
    class="fixed inset-0 z-[60] flex items-start justify-center pt-[12vh] px-4 bg-black/40 backdrop-blur-sm"
    onclick={(e) => {
      if (e.target === e.currentTarget) palette.hide();
    }}
    role="presentation"
  >
    <div
      use:trapFocus
      role="dialog"
      aria-label="Command palette"
      aria-modal="true"
      tabindex={-1}
      class="w-full max-w-[600px] bg-bg border border-border rounded-xl shadow-2xl
             overflow-hidden flex flex-col"
    >
      <div class="flex items-center gap-2 px-4 py-3 border-b border-border">
        <Icon name="search" size={14} class="text-fg-subtle" />
        <input
          bind:this={inputEl}
          type="text"
          value={palette.query}
          oninput={(e) =>
            palette.setQuery((e.currentTarget as HTMLInputElement).value)}
          onkeydown={onInputKey}
          placeholder="Type a command or project name…"
          class="flex-1 bg-transparent text-sm outline-none text-fg
                 placeholder-fg-subtle"
          aria-autocomplete="list"
          aria-controls="palette-results"
          spellcheck="false"
        />
        <kbd
          class="text-[10px] font-mono px-1.5 py-0.5 rounded
                 border border-border text-fg-subtle"
        >
          ESC
        </kbd>
      </div>

      {#if results.length === 0}
        <div class="px-4 py-8 text-center text-sm text-fg-muted">
          {palette.query.trim()
            ? `No matches for "${palette.query}".`
            : "Type to search."}
        </div>
      {:else}
        <div
          bind:this={listEl}
          id="palette-results"
          role="listbox"
          aria-label="Command results"
          class="max-h-[60vh] overflow-y-auto py-1"
        >
          {#each results as item, i (item.cmd.id)}
            {@const header = headerAt(i)}
            {#if header}
              <div
                class="px-4 pt-2 pb-1 text-[10px] uppercase tracking-wide
                       text-fg-subtle font-medium"
              >
                {header === "Recent" ? "Recent" : groupLabel[header as PaletteGroup]}
              </div>
            {/if}
            <button
              type="button"
              role="option"
              aria-selected={i === selectedIndex}
              data-palette-row={i}
              onmousemove={() => (selectedIndex = i)}
              onclick={() => execute(i)}
              class="w-full flex items-center gap-2.5 px-4 py-2 text-left
                     text-sm transition-colors"
              class:bg-accent={i === selectedIndex}
              class:text-on-accent={i === selectedIndex}
              class:text-fg={i !== selectedIndex}
              class:hover:bg-surface-2={i !== selectedIndex}
            >
              {#if item.cmd.icon}
                <Icon name={item.cmd.icon} size={13} />
              {:else}
                <span class="w-[13px]"></span>
              {/if}
              <span class="flex-1 min-w-0 truncate">{item.cmd.label}</span>
              {#if item.cmd.detail}
                <span
                  class="text-[11px] truncate max-w-[40%]"
                  class:text-on-accent={i === selectedIndex}
                  class:text-fg-subtle={i !== selectedIndex}
                  title={item.cmd.detail}
                >
                  {item.cmd.detail}
                </span>
              {/if}
              {#if item.cmd.shortcut}
                <kbd
                  class="text-[10px] font-mono px-1.5 py-0.5 rounded
                         border"
                  class:border-on-accent={i === selectedIndex}
                  class:text-on-accent={i === selectedIndex}
                  class:border-border={i !== selectedIndex}
                  class:text-fg-subtle={i !== selectedIndex}
                >
                  {item.cmd.shortcut}
                </kbd>
              {/if}
            </button>
          {/each}
        </div>
      {/if}

      <footer
        class="px-4 py-2 border-t border-border flex items-center justify-between
               text-[10px] text-fg-subtle"
      >
        <div class="flex items-center gap-3">
          <span class="inline-flex items-center gap-1">
            <kbd
              class="font-mono px-1 py-0.5 rounded border border-border"
            >↑</kbd>
            <kbd
              class="font-mono px-1 py-0.5 rounded border border-border"
            >↓</kbd>
            navigate
          </span>
          <span class="inline-flex items-center gap-1">
            <kbd
              class="font-mono px-1 py-0.5 rounded border border-border"
            >↵</kbd>
            run
          </span>
        </div>
        <span class="font-mono">
          {results.length} command{results.length === 1 ? "" : "s"}
        </span>
      </footer>
    </div>
  </div>
{/if}
