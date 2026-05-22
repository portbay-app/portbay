<!--
  DashboardCard — surface container with a title row, body slot, and
  optional footer slot.

  Lifted from Lerd (`tabs/dashboard/DashboardCard.svelte`, MIT) and
  restyled to PortBay's dark-surface palette. See NOTICE.

  Three tones: `default`, `critical` (red left accent), `warn` (amber).
  Tones are used sparingly — the screenshot's NGINX/MySQL cards do this
  to signal stopped/needing-action sidecars at a glance.
-->
<script lang="ts" module>
  export type CardTone = "default" | "critical" | "warn";
</script>

<script lang="ts">
  import type { Snippet } from "svelte";

  interface Props {
    title?: string;
    tone?: CardTone;
    /** Right-side header decoration (status pill, badge, action button). */
    badge?: Snippet;
    footer?: Snippet;
    /** Drop the body's max-height + scroll — used when the card is short. */
    flush?: boolean;
    children: Snippet;
  }
  let {
    title,
    tone = "default",
    badge,
    footer,
    flush = false,
    children,
  }: Props = $props();

  const accent: Record<CardTone, string> = {
    default: "",
    critical: "border-l-4 border-l-status-crashed",
    warn: "border-l-4 border-l-status-unhealthy",
  };
</script>

<div
  class="flex flex-col bg-surface border border-border rounded-xl overflow-hidden {accent[
    tone
  ]} {flush ? '' : 'max-h-[340px]'}"
>
  {#if title || badge}
    <div
      class="shrink-0 flex items-center justify-between gap-3 px-4 py-2.5 border-b border-border"
    >
      {#if title}
        <span class="text-sm font-semibold text-fg">{title}</span>
      {/if}
      {#if badge}{@render badge()}{/if}
    </div>
  {/if}
  <div
    class="flex-1 min-h-0 px-4 py-3 space-y-2.5 {flush
      ? ''
      : 'overflow-y-auto'}"
  >
    {@render children()}
  </div>
  {#if footer}
    <div class="shrink-0 px-4 py-2.5 border-t border-border">
      {@render footer()}
    </div>
  {/if}
</div>
