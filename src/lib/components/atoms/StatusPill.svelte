<!--
  StatusPill — dot + word, the canonical "what's this project doing right now"
  affordance. Used in the project list, detail panel, sidecar cards, and any
  inline location where a single project/service status matters.

  Lifted from Lerd (`StatusPill.svelte`, MIT) and adapted to PortBay's
  six-state taxonomy. See NOTICE.
-->
<script lang="ts">
  import type { DisplayStatus } from "$lib/types/status";
  import { displayStatusLabel } from "$lib/types/status";
  import StatusDot from "./StatusDot.svelte";

  interface Props {
    status: DisplayStatus;
    /** Override the default word. Use sparingly — taxonomy consistency matters. */
    label?: string;
    /** Compact mode hides the word, leaves only the dot + tooltip. */
    iconOnly?: boolean;
  }
  let { status, label, iconOnly = false }: Props = $props();

  const word = $derived(label ?? displayStatusLabel(status));

  // Subtle surface tint matches the status, kept low-contrast so the dot
  // and word do the visual work. `stopping` reuses the neutral stopped tint.
  const toneClass: Record<DisplayStatus, string> = {
    stopped: "bg-status-stopped/10 text-fg-muted",
    stopping: "bg-status-stopped/10 text-fg-muted",
    starting: "bg-status-starting/10 text-status-starting",
    running: "bg-status-running/10 text-status-running",
    unhealthy: "bg-status-unhealthy/10 text-status-unhealthy",
    crashed: "bg-status-crashed/10 text-status-crashed",
    port_conflict: "bg-status-port-conflict/10 text-status-port-conflict",
  };
</script>

{#if iconOnly}
  <span
    title={word}
    aria-label={word}
    class="inline-flex items-center justify-center w-5 h-5"
  >
    <StatusDot {status} size="md" />
  </span>
{:else}
  <span
    class="inline-flex items-center gap-1.5 text-xs font-medium px-2 py-0.5 rounded-full {toneClass[
      status
    ]}"
  >
    <StatusDot {status} size="sm" />
    {word}
  </span>
{/if}
