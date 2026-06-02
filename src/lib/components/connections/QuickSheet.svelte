<!--
  QuickSheet — shared VS Code-style top-dropdown shell used by the three SSH
  credential prompts. Descends from the top of the window (12vh) like VS Code's
  Quick Input.

  A transparent full-screen click-catcher sits behind the panel (z-[120]).
  There is deliberately NO background dim — these read as top dropdowns, not
  modal pop-ups.

  Props:
    open          — controlled visibility
    heading       — bold title in the header
    hostLabel?    — secondary label shown after the heading (host/port, etc.)
    icon?         — Icon name for the header badge (default "key")
    iconClass?    — Tailwind class for the icon colour (default "text-accent")
    ondismiss     — called on Esc keydown and on backdrop click
    body          — snippet rendered inside px-3.5 py-3
    footer?       — optional snippet rendered after the body wrapper
    headerExtra?  — optional snippet rendered right-aligned inside the header row
-->
<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "$lib/components/atoms/Icon.svelte";
  import type { IconName } from "$lib/components/atoms/Icon.svelte";

  interface Props {
    open: boolean;
    heading: string;
    hostLabel?: string;
    icon?: IconName;
    iconClass?: string;
    ondismiss: () => void;
    body: Snippet;
    footer?: Snippet;
    headerExtra?: Snippet;
  }

  let {
    open,
    heading,
    hostLabel,
    icon = "key",
    iconClass = "text-accent",
    ondismiss,
    body,
    footer,
    headerExtra,
  }: Props = $props();

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      ondismiss();
    }
  }
</script>

{#if open}
  <!-- Transparent full-screen click-catcher — no dim, just a dismiss target. -->
  <div
    class="fixed inset-0 z-[120]"
    role="presentation"
    onclick={ondismiss}
  ></div>

  <!-- Top-anchored panel, centered horizontally like VS Code's Quick Input. -->
  <div
    class="fixed left-1/2 top-[12vh] z-[121] w-[min(92vw,440px)] -translate-x-1/2
           rounded-lg border border-border bg-surface shadow-2xl"
    role="dialog"
    aria-modal="true"
    aria-label={heading}
    tabindex={-1}
    onkeydown={onKeydown}
  >
    <!-- Header -->
    <div class="flex items-center gap-2 border-b border-border/60 px-3.5 py-2.5">
      <Icon name={icon} size={14} class={iconClass} />
      <span class="text-[12.5px] font-semibold text-fg">{heading}</span>
      {#if hostLabel}
        <span class="ml-1 min-w-0 truncate text-[11.5px] text-fg-subtle">{hostLabel}</span>
      {/if}
      {#if headerExtra}
        <span class="ml-auto shrink-0">
          {@render headerExtra()}
        </span>
      {/if}
    </div>

    <!-- Body slot -->
    <div class="px-3.5 py-3">
      {@render body()}
    </div>

    {#if footer}
      {@render footer()}
    {/if}
  </div>
{/if}
