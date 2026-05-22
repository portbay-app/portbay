<!--
  Badge — small neutral chip for tags, framework labels, version strings,
  and any non-status secondary info.

  Lifted from Lerd (`Badge.svelte`, MIT) and adapted to PortBay's tone
  set. See NOTICE.

  Status-tinted variants are NOT here on purpose — for live status, use
  StatusPill, which keeps the dot+word pairing canonical.
-->
<script lang="ts" module>
  export type BadgeTone = "neutral" | "info" | "success" | "warning" | "danger";
</script>

<script lang="ts">
  import type { Snippet } from "svelte";

  interface Props {
    tone?: BadgeTone;
    title?: string;
    onclick?: (e: MouseEvent) => void;
    children: Snippet;
  }
  let { tone = "neutral", title, onclick, children }: Props = $props();

  const toneClass: Record<BadgeTone, string> = {
    neutral: "text-fg-muted bg-surface-2 border border-border",
    info: "text-status-starting bg-status-starting/10 border border-status-starting/30",
    success:
      "text-status-running bg-status-running/10 border border-status-running/30",
    warning:
      "text-status-unhealthy bg-status-unhealthy/10 border border-status-unhealthy/30",
    danger:
      "text-status-crashed bg-status-crashed/10 border border-status-crashed/30",
  };

  const base =
    "inline-flex items-center gap-1 text-xs font-medium px-2 py-0.5 rounded-md transition-colors";
</script>

{#if onclick}
  <button
    {onclick}
    {title}
    class="{base} {toneClass[tone]} hover:brightness-110"
  >
    {@render children()}
  </button>
{:else}
  <span {title} class="{base} {toneClass[tone]}">
    {@render children()}
  </span>
{/if}
