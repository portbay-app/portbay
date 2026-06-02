<!--
  Popover — a small anchored editor panel, the pattern Planka uses for every
  card attribute. A `trigger` snippet renders the anchor (button/chip) and is
  given `(toggle, open)`; the `children` snippet renders the panel body and is
  given `(close)`. Closes on outside-click or Escape.
-->
<script lang="ts">
  import type { Snippet } from "svelte";

  import Icon from "$lib/components/atoms/Icon.svelte";

  interface Props {
    /** Renders the anchor. Receives a toggle fn and the open state. */
    trigger: Snippet<[() => void, boolean]>;
    /** Renders the panel body. Receives a close fn. */
    children: Snippet<[() => void]>;
    title?: string;
    align?: "left" | "right";
    /** Open above the trigger instead of below (for items low on screen). */
    up?: boolean;
    width?: string;
    /** Extra classes for the relative root wrapper (e.g. `h-full` so a
     *  full-height trigger can stretch). */
    rootClass?: string;
    /** Two-way bindable open state. Lets a parent observe the panel opening —
     *  e.g. to keep a hover-revealed trigger visible while the menu is up,
     *  since WebKit doesn't focus buttons on click so `focus-within` can't. */
    open?: boolean;
  }
  let {
    trigger,
    children,
    title,
    align = "right",
    up = false,
    width = "17rem",
    rootClass = "",
    open = $bindable(false),
  }: Props = $props();

  let root = $state<HTMLElement | null>(null);

  function toggle() {
    open = !open;
  }
  function close() {
    open = false;
  }

  $effect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (root && !root.contains(e.target as Node)) close();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") close();
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  });
</script>

<div class="relative {rootClass}" bind:this={root}>
  {@render trigger(toggle, open)}
  {#if open}
    <div
      class="absolute z-30 rounded-lg border border-border bg-surface shadow-xl p-2.5
             {align === 'right' ? 'right-0' : 'left-0'} {up ? 'bottom-full mb-1' : 'top-full mt-1'}"
      style:width
      role="dialog"
    >
      {#if title}
        <div class="flex items-center justify-between mb-2">
          <span class="text-[11px] font-semibold uppercase tracking-wide text-fg-subtle">{title}</span>
          <button
            type="button"
            onclick={close}
            aria-label="Close"
            class="text-fg-subtle hover:text-fg transition-colors"
          >
            <Icon name="x" size={12} />
          </button>
        </div>
      {/if}
      {@render children(close)}
    </div>
  {/if}
</div>
