<!--
  Popover — a small anchored editor panel, the pattern Planka uses for every
  card attribute. A `trigger` snippet renders the anchor (button/chip) and is
  given `(toggle, open)`; the `children` snippet renders the panel body and is
  given `(close)`. Closes on outside-click or Escape.

  The panel is measured on open and kept on-screen (floating-ui's shift/flip,
  same as AgentDropdown): it slides horizontally back into the viewport when
  the anchor sits near a window edge, and flips above/below the trigger when
  the preferred side would clip.
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

  // Viewport correction, measured when the panel mounts: a horizontal slide
  // (px) back on-screen, and a vertical flip when the preferred side clips.
  let dx = $state(0);
  let flip = $state(false);
  const openUp = $derived(flip ? !up : up);

  /** Action on the panel: measure once on open, shift/flip to stay visible. */
  function place(el: HTMLElement) {
    const pad = 8;
    const r = el.getBoundingClientRect();
    let shift = 0;
    if (r.right > window.innerWidth - pad) shift = window.innerWidth - pad - r.right;
    if (r.left + shift < pad) shift = pad - r.left;
    dx = shift;
    const anchor = root?.getBoundingClientRect();
    if (anchor) {
      if (!up && r.bottom > window.innerHeight - pad && anchor.top - r.height >= pad) {
        flip = true;
      } else if (up && r.top < pad && anchor.bottom + r.height <= window.innerHeight - pad) {
        flip = true;
      }
    }
    return {
      destroy() {
        dx = 0;
        flip = false;
      },
    };
  }

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
      use:place
      class="absolute z-30 rounded-lg border border-border bg-surface shadow-xl p-2.5
             {align === 'right' ? 'right-0' : 'left-0'} {openUp
        ? 'bottom-full mb-1'
        : 'top-full mt-1'}"
      style:width
      style:transform={dx ? `translateX(${dx}px)` : undefined}
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
