<!--
  AgentDropdown — the composer-toolbar selector, a Svelte port of Void's
  VoidCustomDropdownBox (SidebarChat.tsx). A tight inline trigger "Name ▾" whose
  chevron touches the text, and a menu of rows with a leading check column, an
  optional per-row icon, the title, and (stacked beneath it) an optional detail.
  Used by the provider, model, and mode selectors.

  The menu is `position: fixed` and clamped to the viewport (Void uses
  floating-ui shift/flip) — the agent panel docks at the window's right edge, so
  a naively-positioned menu clips off-screen. It opens UPWARD from the trigger
  (the toolbar sits at the panel's bottom). Closes on outside-click or Escape.
-->
<script lang="ts" generics="T">
  import type { Snippet } from "svelte";

  interface Props {
    options: T[];
    selected: T | undefined;
    onChange: (value: T) => void;
    /** Trigger label (compact). */
    displayName: (option: T) => string;
    /** Row label in the menu; defaults to displayName. */
    dropdownName?: (option: T) => string;
    /** Optional detail, shown stacked under the row title. */
    detail?: (option: T) => string | undefined;
    equals?: (a: T, b: T) => boolean;
    /** Void's mode dropdown is bordered; the model dropdown is borderless. */
    bordered?: boolean;
    disabled?: boolean;
    title?: string;
    minWidth?: string;
    /** Optional glyph for the selected trigger + each row (e.g. provider icon). */
    optionIcon?: Snippet<[T]>;
  }

  let {
    options,
    selected,
    onChange,
    displayName,
    dropdownName,
    detail,
    equals = (a, b) => a === b,
    bordered = false,
    disabled = false,
    title,
    minWidth = "13rem",
    optionIcon,
  }: Props = $props();

  let open = $state(false);
  let root = $state<HTMLElement | null>(null);
  let triggerEl = $state<HTMLElement | null>(null);
  let menuEl = $state<HTMLElement | null>(null);

  // Fixed-position coordinates (viewport space), clamped so the menu never
  // clips off-screen. `placed` hides the menu for the first frame to avoid a
  // flash at (0,0) before we measure.
  let left = $state(0);
  let bottom = $state(0);
  let placed = $state(false);

  const rowName = (o: T) => (dropdownName ?? displayName)(o);

  function choose(o: T) {
    open = false;
    onChange(o);
  }

  // Render the menu under <body> so `position: fixed` is always viewport-anchored
  // (a transformed ancestor would otherwise become its containing block and the
  // viewport clamping would be wrong / clip).
  function portal(node: HTMLElement) {
    document.body.appendChild(node);
    return {
      destroy() {
        node.remove();
      },
    };
  }

  $effect(() => {
    if (!open) {
      placed = false;
      return;
    }
    const place = () => {
      if (!triggerEl) return;
      const t = triggerEl.getBoundingClientRect();
      const mw = menuEl?.offsetWidth ?? 208;
      left = Math.min(Math.max(8, t.left), window.innerWidth - mw - 8);
      bottom = window.innerHeight - t.top + 6; // open upward, 6px gap
      placed = true;
    };
    place();
    // Re-place once the menu has rendered its real width.
    requestAnimationFrame(place);
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, true);

    const onDown = (e: MouseEvent) => {
      const t = e.target as Node;
      // The menu is portaled out of `root`, so check it explicitly too.
      if (root && !root.contains(t) && menuEl && !menuEl.contains(t)) open = false;
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") open = false;
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);

    return () => {
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", place, true);
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  });
</script>

<div class="relative inline-block" bind:this={root}>
  <button
    type="button"
    bind:this={triggerEl}
    onclick={() => (open = !open)}
    {disabled}
    aria-haspopup="menu"
    aria-expanded={open}
    class="inline-flex h-6 max-w-[180px] items-center gap-1 whitespace-nowrap text-[12px] text-fg-muted hover:text-fg disabled:opacity-50
           {bordered ? 'rounded border border-border bg-surface px-1.5 py-0.5' : 'rounded px-0.5'}"
  >
    {#if optionIcon && selected !== undefined}{@render optionIcon(selected)}{/if}
    <span class="truncate">{selected !== undefined ? displayName(selected) : ""}</span>
    <!-- Void's chevron: size-3, touches the text. -->
    <svg class="size-3 shrink-0" viewBox="0 0 12 12" fill="none" aria-hidden="true">
      <path
        d="M2.5 4.5L6 8L9.5 4.5"
        stroke="currentColor"
        stroke-width="1.5"
        stroke-linecap="round"
        stroke-linejoin="round"
      />
    </svg>
  </button>

  {#if open}
    <div
      bind:this={menuEl}
      use:portal
      role="menu"
      class="fixed z-50 max-h-80 overflow-auto rounded-md border border-border bg-surface p-1 shadow-lg"
      style:left="{left}px"
      style:bottom="{bottom}px"
      style:min-width={minWidth}
      style:max-width="calc(100vw - 16px)"
      style:visibility={placed ? "visible" : "hidden"}
    >
      {#if title}
        <div class="px-2 py-1 text-[10.5px] uppercase tracking-wide text-fg-subtle">{title}</div>
      {/if}
      {#each options as option, i (i)}
        {@const isSelected = selected !== undefined && equals(option, selected)}
        {@const d = detail?.(option)}
        <button
          type="button"
          role="menuitem"
          onclick={() => choose(option)}
          class="flex w-full items-start gap-2 rounded px-2 py-1.5 text-left transition-colors
                 {isSelected ? 'bg-accent text-on-accent' : 'text-fg hover:bg-surface-2'}"
        >
          {#if optionIcon}
            <span class="mt-0.5 shrink-0">{@render optionIcon(option)}</span>
          {/if}
          <span class="min-w-0 flex-1">
            <span class="block truncate text-[12px]">{rowName(option)}</span>
            {#if d}
              <span class="block text-[11px] {isSelected ? 'text-on-accent/80' : 'text-fg-subtle'}">
                {d}
              </span>
            {/if}
          </span>
        </button>
      {/each}
    </div>
  {/if}
</div>
