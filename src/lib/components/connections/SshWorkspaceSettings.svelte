<!--
  SshWorkspaceSettings — the workspace's Settings tab. Two honest groups:
  terminal preferences (local UI settings, persisted to localStorage and applied
  to new shells) and a shortcut into the host connection editor for the
  server-side details (host, auth, tags…). No fabricated server settings.
-->
<script lang="ts">
  import Icon from "$lib/components/atoms/Icon.svelte";
  import {
    compileRules,
    matchLine,
    patternError,
    HIGHLIGHT_PRESETS,
    MAX_HIGHLIGHT_RULES,
    type HighlightRenderMode,
  } from "$lib/ssh/terminalHighlight";
  import { terminalPrefs } from "$lib/stores/sshWorkspacePrefs.svelte";

  let { onEdit }: { onEdit: () => void } = $props();

  const p = $derived(terminalPrefs.value);
  const atCap = $derived(p.highlightRules.length >= MAX_HIGHLIGHT_RULES);

  const RENDER_MODES: { value: HighlightRenderMode; label: string }[] = [
    { value: "background", label: "Background" },
    { value: "underline", label: "Underline" },
    { value: "outline", label: "Outline" },
  ];

  // Live-test preview: compile the current rules and split a sample line into
  // styled / plain segments with the same matcher the real terminal uses.
  let testInput = $state(
    "2026-06-02 10:24:01 ERROR connection refused on 10.0.0.4:5432 — warning: deprecated flag",
  );
  const compiled = $derived(compileRules(p.highlightRules));

  interface Segment {
    text: string;
    color: string | null;
    mode: HighlightRenderMode | null;
  }
  function segmentize(text: string): Segment[] {
    const matches = matchLine(text, compiled);
    const segs: Segment[] = [];
    let cursor = 0;
    for (const m of matches) {
      if (m.start > cursor) segs.push({ text: text.slice(cursor, m.start), color: null, mode: null });
      segs.push({ text: text.slice(m.start, m.end), color: m.color, mode: m.renderMode });
      cursor = m.end;
    }
    if (cursor < text.length) segs.push({ text: text.slice(cursor), color: null, mode: null });
    return segs;
  }

  /** `#RRGGBB` → translucent fill, matching the terminal overlay alpha. */
  function tint(hex: string): string {
    let h = hex.trim().replace(/^#/, "");
    if (h.length === 3) h = h.split("").map((c) => c + c).join("");
    if (!/^[0-9a-fA-F]{6}$/.test(h)) return "transparent";
    const r = parseInt(h.slice(0, 2), 16);
    const g = parseInt(h.slice(2, 4), 16);
    const b = parseInt(h.slice(4, 6), 16);
    return `rgba(${r}, ${g}, ${b}, 0.32)`;
  }

  /** Inline style for a preview segment, mirroring the terminal render modes. */
  function previewStyle(seg: Segment): string {
    if (!seg.color || !seg.mode) return "";
    if (seg.mode === "underline") return `border-bottom: 2px solid ${seg.color}`;
    if (seg.mode === "outline") return `outline: 1px solid ${seg.color}; outline-offset: -1px`;
    return `background-color: ${tint(seg.color)}`;
  }

  // Preset dropdown (built like the host-actions menu: a fixed click-away
  // overlay behind an absolutely-positioned panel — no external dependency).
  let presetOpen = $state(false);
  function addPreset(preset: (typeof HIGHLIGHT_PRESETS)[number]) {
    presetOpen = false;
    const id = terminalPrefs.addHighlightRule(preset);
    if (id) expanded[id] = false; // collapsed: presets are ready to use as-is
  }

  // Which rules are expanded to the full editor (collapsed shows a summary).
  let expanded = $state<Record<string, boolean>>({});
  function addBlank() {
    const id = terminalPrefs.addHighlightRule();
    if (id) expanded[id] = true; // a blank rule needs editing, so open it
  }

  // Native drag-reorder (re-prioritise). The grip is the drag source; each row
  // is a drop target. WebKit only suppresses native drag on img/svg/a, so a
  // draggable div works here.
  let draggedIndex = $state<number | null>(null);
  let dragOverIndex = $state<number | null>(null);

  function onDragStart(e: DragEvent, index: number) {
    draggedIndex = index;
    e.dataTransfer?.setData("text/plain", String(index));
    if (e.dataTransfer) e.dataTransfer.effectAllowed = "move";
  }
  function onDrop(target: number) {
    if (draggedIndex !== null) terminalPrefs.reorderHighlightRule(draggedIndex, target);
    draggedIndex = null;
    dragOverIndex = null;
  }
</script>

<div class="h-full overflow-y-auto px-8 py-6">
  <div class="mx-auto max-w-2xl space-y-5">
    <!-- Terminal preferences -->
    <section class="rounded-xl border border-border/70 bg-surface px-5 py-4">
      <div class="flex items-center gap-2">
        <Icon name="terminal" size={14} class="text-fg-muted" />
        <h2 class="text-[13px] font-semibold text-fg">Terminal</h2>
      </div>
      <p class="mt-1 text-[11.5px] text-fg-subtle">
        Applied to new shells. Stored locally on this machine, shared across hosts.
      </p>

      <div class="mt-4 space-y-4">
        <label class="flex items-center justify-between gap-4">
          <span class="text-[12.5px] text-fg">Font size</span>
          <span class="flex items-center gap-2">
            <input
              type="range"
              min="9"
              max="24"
              value={p.fontSize}
              oninput={(e) => terminalPrefs.update({ fontSize: Number(e.currentTarget.value) })}
              class="accent-accent"
            />
            <span class="w-10 text-right font-mono text-[12px] text-fg-muted tabular-nums">{p.fontSize}px</span>
          </span>
        </label>

        <label class="flex items-center justify-between gap-4">
          <span class="text-[12.5px] text-fg">Scrollback (lines)</span>
          <input
            type="number"
            min="100"
            max="200000"
            step="1000"
            value={p.scrollback}
            onchange={(e) => terminalPrefs.update({ scrollback: Number(e.currentTarget.value) })}
            class="h-8 w-32 rounded-md border border-border bg-surface px-2 text-right font-mono text-[12px] text-fg outline-none focus:border-accent"
          />
        </label>

        <label class="flex items-center justify-between gap-4">
          <span class="text-[12.5px] text-fg">Cursor blink</span>
          <input
            type="checkbox"
            checked={p.cursorBlink}
            onchange={(e) => terminalPrefs.update({ cursorBlink: e.currentTarget.checked })}
            class="rounded border-border accent-accent"
          />
        </label>

        <label class="block">
          <span class="text-[12.5px] text-fg">Startup command</span>
          <span class="mt-0.5 block text-[11px] text-fg-subtle">
            Run once in each new interactive shell (e.g. <code class="font-mono">cd ~/project &amp;&amp; conda activate</code>).
          </span>
          <input
            value={p.startupCommand}
            onchange={(e) => terminalPrefs.update({ startupCommand: e.currentTarget.value })}
            placeholder="Optional"
            class="mt-1.5 h-8 w-full rounded-md border border-border bg-surface px-2 font-mono text-[12px] text-fg outline-none focus:border-accent"
          />
        </label>

        <label class="flex items-start justify-between gap-4">
          <span class="min-w-0">
            <span class="text-[12.5px] text-fg">Suggestions use terminal output</span>
            <span class="mt-0.5 block text-[11px] text-fg-subtle">
              Feed recent terminal output (not just typed commands) to the host's
              model for the inline next-command suggestion. Off by default — the
              buffer can hold secrets. When on it's capped and lightly redacted,
              and only ever sent to the host's own model over SSH.
            </span>
          </span>
          <input
            type="checkbox"
            checked={p.suggestBufferContext}
            onchange={(e) => terminalPrefs.update({ suggestBufferContext: e.currentTarget.checked })}
            class="mt-0.5 shrink-0 rounded border-border accent-accent"
          />
        </label>
      </div>

      <button
        type="button"
        onclick={() => terminalPrefs.reset()}
        class="mt-4 inline-flex items-center gap-1.5 h-8 px-3 rounded-md text-[12px] font-medium border border-border text-fg-muted hover:text-fg hover:bg-surface-2"
      >
        <Icon name="rotate-ccw" size={12} /> Reset to defaults
      </button>
    </section>

    <!-- Terminal highlight rules -->
    <section class="rounded-xl border border-border/70 bg-surface px-5 py-4">
      <div class="flex items-start justify-between gap-3">
        <div class="min-w-0">
          <div class="flex items-center gap-2">
            <Icon name="sliders-horizontal" size={14} class="text-fg-muted" />
            <h2 class="text-[13px] font-semibold text-fg">Highlight rules</h2>
          </div>
          <p class="mt-1 text-[11.5px] text-fg-subtle">
            Colour matching text in terminal &amp; log output. Earlier rules win an
            overlap — drag to reorder.
          </p>
        </div>
        <div class="flex shrink-0 items-center gap-1.5">
          <div class="relative">
            <button
              type="button"
              onclick={() => (presetOpen = !presetOpen)}
              disabled={atCap}
              class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md text-[12px] font-medium border border-border text-fg-muted hover:text-fg hover:bg-surface-2 disabled:opacity-50"
            >
              Presets <Icon name="chevron-down" size={12} />
            </button>
            {#if presetOpen}
              <button type="button" class="fixed inset-0 z-10 cursor-default" aria-label="Close menu" onclick={() => (presetOpen = false)}></button>
              <div class="absolute right-0 z-20 mt-1 max-h-72 w-52 overflow-y-auto rounded-lg border border-border bg-surface p-1 shadow-xl">
                {#each HIGHLIGHT_PRESETS as preset (preset.label)}
                  <button
                    type="button"
                    onclick={() => addPreset(preset)}
                    class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg"
                  >
                    <span class="h-3 w-3 shrink-0 rounded-sm" style="background-color: {preset.color}"></span>
                    {preset.label}
                  </button>
                {/each}
              </div>
            {/if}
          </div>
          <button
            type="button"
            onclick={addBlank}
            disabled={atCap}
            class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md text-[12px] font-medium border border-border text-fg-muted hover:text-fg hover:bg-surface-2 disabled:opacity-50"
          >
            <Icon name="plus" size={12} /> Add rule
          </button>
        </div>
      </div>

      {#if p.highlightRules.length === 0}
        <div class="mt-4 rounded-lg border border-dashed border-border/70 px-4 py-6 text-center text-[12px] text-fg-subtle">
          No highlight rules. Add one, or pick a preset, to colour matching output.
        </div>
      {:else}
        <ul class="mt-4 space-y-2">
          {#each p.highlightRules as rule, i (rule.id)}
            {@const err = patternError(rule.pattern, rule.isRegex)}
            {@const isOpen = expanded[rule.id]}
            <li
              role="listitem"
              ondragover={(e) => {
                e.preventDefault();
                dragOverIndex = i;
              }}
              ondrop={(e) => {
                e.preventDefault();
                onDrop(i);
              }}
              class="rounded-lg border bg-surface
                {dragOverIndex === i && draggedIndex !== null && draggedIndex !== i
                  ? 'border-accent'
                  : 'border-border/70'}
                {draggedIndex === i ? 'opacity-50' : ''}"
            >
              <!-- Collapsed header (always shown) -->
              <div class="flex items-center gap-2 px-2 py-1.5">
                <div
                  role="button"
                  tabindex="-1"
                  draggable="true"
                  ondragstart={(e) => onDragStart(e, i)}
                  ondragend={() => {
                    draggedIndex = null;
                    dragOverIndex = null;
                  }}
                  class="shrink-0 cursor-grab rounded p-1 text-fg-subtle hover:text-fg active:cursor-grabbing"
                  aria-label="Drag to reorder"
                >
                  <Icon name="chevrons-up-down" size={14} />
                </div>

                <span class="h-4 w-4 shrink-0 rounded-sm border border-border" style="background-color: {rule.color}"></span>

                <button
                  type="button"
                  onclick={() => (expanded[rule.id] = !isOpen)}
                  class="flex min-w-0 flex-1 items-center gap-2 text-left"
                  aria-expanded={isOpen}
                >
                  <Icon name={isOpen ? "chevron-down" : "chevron-right"} size={13} class="shrink-0 text-fg-subtle" />
                  <span class="truncate text-[12.5px] text-fg">
                    {rule.label.trim() || rule.pattern || "Untitled rule"}
                  </span>
                  {#if !rule.isRegex}
                    <span class="shrink-0 rounded border border-border px-1.5 py-px text-[10px] text-fg-subtle">literal</span>
                  {/if}
                  {#if err}
                    <span class="shrink-0 text-status-crashed" title={err}><Icon name="circle-alert" size={12} /></span>
                  {/if}
                </button>

                <label class="shrink-0 inline-flex items-center" title={rule.enabled ? "Enabled" : "Disabled"}>
                  <input
                    type="checkbox"
                    checked={rule.enabled}
                    onchange={(e) => terminalPrefs.updateHighlightRule(rule.id, { enabled: e.currentTarget.checked })}
                    class="rounded border-border accent-accent"
                    aria-label="Rule enabled"
                  />
                </label>
                <button
                  type="button"
                  onclick={() => terminalPrefs.removeHighlightRule(rule.id)}
                  class="shrink-0 rounded p-1 text-fg-subtle hover:bg-status-crashed/10 hover:text-status-crashed"
                  aria-label="Delete rule"
                >
                  <Icon name="trash-2" size={14} />
                </button>
              </div>

              <!-- Expanded editor -->
              {#if isOpen}
                <div class="space-y-2.5 border-t border-border/60 px-3 py-3">
                  <div class="grid grid-cols-[80px_1fr] items-center gap-2">
                    <span class="text-[11.5px] text-fg-subtle">Name</span>
                    <input
                      value={rule.label}
                      oninput={(e) => terminalPrefs.updateHighlightRule(rule.id, { label: e.currentTarget.value })}
                      placeholder="Optional"
                      class="h-7 w-full rounded-md border border-border bg-surface px-2 text-[12px] text-fg outline-none focus:border-accent"
                    />
                  </div>

                  <div class="grid grid-cols-[80px_1fr] items-start gap-2">
                    <span class="pt-1.5 text-[11.5px] text-fg-subtle">Pattern</span>
                    <div class="min-w-0">
                      <div class="relative">
                        <input
                          value={rule.pattern}
                          oninput={(e) => terminalPrefs.updateHighlightRule(rule.id, { pattern: e.currentTarget.value })}
                          placeholder={rule.isRegex ? "regex, e.g. \\berror\\b" : "text to match"}
                          spellcheck="false"
                          class="h-7 w-full rounded-md border bg-surface px-2 pr-7 font-mono text-[12px] text-fg outline-none focus:border-accent
                            {err ? 'border-status-crashed/60' : 'border-border'}"
                        />
                        {#if err}
                          <span class="pointer-events-none absolute right-1.5 top-1/2 -translate-y-1/2 text-status-crashed" title={err}>
                            <Icon name="circle-alert" size={13} />
                          </span>
                        {/if}
                      </div>
                      {#if err}
                        <p class="mt-1 text-[11px] text-status-crashed">{err}</p>
                      {/if}
                      <div class="mt-1.5 flex items-center gap-4">
                        <label class="inline-flex items-center gap-1.5 text-[11.5px] text-fg-muted">
                          <input type="checkbox" checked={rule.isRegex} onchange={(e) => terminalPrefs.updateHighlightRule(rule.id, { isRegex: e.currentTarget.checked })} class="rounded border-border accent-accent" />
                          Regex
                        </label>
                        <label class="inline-flex items-center gap-1.5 text-[11.5px] text-fg-muted">
                          <input type="checkbox" checked={rule.caseSensitive} onchange={(e) => terminalPrefs.updateHighlightRule(rule.id, { caseSensitive: e.currentTarget.checked })} class="rounded border-border accent-accent" />
                          Case-sensitive
                        </label>
                      </div>
                    </div>
                  </div>

                  <div class="grid grid-cols-[80px_1fr] items-center gap-2">
                    <span class="text-[11.5px] text-fg-subtle">Style</span>
                    <div class="flex items-center gap-2">
                      <input
                        type="color"
                        value={rule.color}
                        onchange={(e) => terminalPrefs.updateHighlightRule(rule.id, { color: e.currentTarget.value })}
                        class="h-7 w-9 shrink-0 cursor-pointer rounded border border-border bg-surface p-0.5"
                        aria-label="Highlight colour"
                      />
                      <select
                        value={rule.renderMode}
                        onchange={(e) => terminalPrefs.updateHighlightRule(rule.id, { renderMode: e.currentTarget.value as HighlightRenderMode })}
                        class="h-7 rounded-md border border-border bg-surface px-2 text-[12px] text-fg outline-none focus:border-accent"
                        aria-label="Render style"
                      >
                        {#each RENDER_MODES as m (m.value)}
                          <option value={m.value}>{m.label}</option>
                        {/each}
                      </select>
                    </div>
                  </div>
                </div>
              {/if}
            </li>
          {/each}
        </ul>
        <p class="mt-2 text-right text-[11px] text-fg-subtle">
          {p.highlightRules.length} / {MAX_HIGHLIGHT_RULES} rules
        </p>
      {/if}

      <!-- Live test -->
      <div class="mt-4">
        <span class="text-[11.5px] text-fg-subtle">Test</span>
        <input
          bind:value={testInput}
          spellcheck="false"
          class="mt-1 h-8 w-full rounded-md border border-border bg-surface px-2 font-mono text-[12px] text-fg outline-none focus:border-accent"
          placeholder="Type a sample line to preview…"
        />
        <div class="mt-1.5 overflow-x-auto rounded-md border border-border/60 bg-surface-2/40 px-2.5 py-2 font-mono text-[12px] leading-5 text-fg">
          {#if testInput}
            <span class="whitespace-pre-wrap break-all">{#each segmentize(testInput) as seg}{#if seg.color}<span class="rounded-[2px]" style={previewStyle(seg)}>{seg.text}</span>{:else}{seg.text}{/if}{/each}</span>
          {:else}
            <span class="text-fg-subtle">Preview appears here.</span>
          {/if}
        </div>
      </div>
    </section>

    <!-- Connection details -->
    <section class="rounded-xl border border-border/70 bg-surface px-5 py-4">
      <div class="flex items-center gap-2">
        <Icon name="server-cog" size={14} class="text-fg-muted" />
        <h2 class="text-[13px] font-semibold text-fg">Connection</h2>
      </div>
      <p class="mt-1 text-[11.5px] text-fg-subtle">
        Host, port, user, authentication, tags, and provider live on the saved
        connection.
      </p>
      <button
        type="button"
        onclick={onEdit}
        class="mt-3 inline-flex items-center gap-1.5 h-8 px-3 rounded-md text-[12px] font-medium bg-surface-2 text-fg hover:bg-surface-2/70"
      >
        <Icon name="pencil" size={12} /> Edit host connection
      </button>
    </section>
  </div>
</div>
