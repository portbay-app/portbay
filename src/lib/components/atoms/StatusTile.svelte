<!--
  StatusTile — the calm-rectangle tile used by the dashboard's four-up
  status row (Projects / Local Access / Services / Local AI).

  Shape: icon top-left, title + subtitle stacked beside it, then a value
  row at the bottom — value content on the left, an optional flourish
  (sparkline, trust/health badge) on the right. No header rule, unlike
  `DashboardCard` (which is a title-header + scrollable-body container for
  the larger panels). Kept as its own atom so the four tiles stop
  hand-rolling identical container + header markup.
-->
<script lang="ts">
  import type { Snippet } from "svelte";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import type { IconName } from "$lib/components/atoms/Icon.svelte";

  interface Props {
    /** Lucide icon name for the top-left chip. */
    icon: IconName;
    /** Tailwind classes for the icon chip's tint, e.g.
     *  "bg-status-running/10 text-status-running". */
    iconClass: string;
    title: string;
    subtitle: string;
    /** The value column (left of the bottom row): the big number + caption. */
    children: Snippet;
    /** Right of the bottom row: a sparkline or a status badge. Optional —
     *  tiles like Projects/Local AI hide it when nothing is running. */
    flourish?: Snippet;
  }
  let { icon, iconClass, title, subtitle, children, flourish }: Props =
    $props();
</script>

<div
  class="bg-surface border border-border rounded-2xl p-4
         flex flex-col gap-3 min-h-[112px]"
>
  <div class="flex items-start justify-between gap-2">
    <div class="flex items-center gap-2.5 min-w-0">
      <span
        class="inline-flex items-center justify-center w-8 h-8 rounded-lg shrink-0 {iconClass}"
      >
        <Icon name={icon} size={15} />
      </span>
      <div class="min-w-0 leading-tight">
        <p class="text-[13px] font-semibold text-fg truncate">{title}</p>
        <p class="text-[11px] text-fg-subtle truncate">{subtitle}</p>
      </div>
    </div>
  </div>
  <div class="flex items-end justify-between gap-2">
    <div class="leading-tight min-w-0">
      {@render children()}
    </div>
    {#if flourish}{@render flourish()}{/if}
  </div>
</div>
