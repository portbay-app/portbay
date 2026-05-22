<!--
  RightRail — slot container for the right column.

  Hidden when the viewport is narrow AND the density is compact, per the
  card #3 spec. Cards #5 (sidecar pills variant) and #11 (system metrics
  rail) populate this slot.
-->
<script lang="ts">
  import { MetricsRail } from "$lib/components/metrics";
  import { density } from "$lib/stores/density";
  import type { Snippet } from "svelte";

  let { children }: { children?: Snippet } = $props();
</script>

<aside
  class="h-full overflow-y-auto bg-bg border-l border-border p-4 space-y-4"
  class:hidden={density.value === "compact"}
  aria-label="Status panel"
>
  {#if children}
    {@render children()}
  {:else}
    <MetricsRail />
  {/if}
</aside>
