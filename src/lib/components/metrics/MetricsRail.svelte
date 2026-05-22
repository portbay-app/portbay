<!--
  MetricsRail — the three system metric cards stacked vertically. Mounts
  inside the layout's RightRail slot. Starts the metrics store on mount.

  Storage + network panels deliberately omitted per the card #11 outcome
  — they belong in Activity Monitor.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { metrics } from "$lib/stores/metrics.svelte";
  import CpuHistoryCard from "./CpuHistoryCard.svelte";
  import CpuLoadCard from "./CpuLoadCard.svelte";
  import MemoryCard from "./MemoryCard.svelte";

  onMount(() => {
    void metrics.start();
    return () => metrics.stop();
  });
</script>

<div class="space-y-4">
  <CpuLoadCard />
  <CpuHistoryCard />
  <MemoryCard />
</div>
