<!--
  MemoryCard — horizontal bar + numeric labels.
-->
<script lang="ts">
  import { DashboardCard } from "$lib/components/atoms";
  import { metrics } from "$lib/stores/metrics.svelte";

  const mem = $derived(metrics.value?.memory);
  const used = $derived(mem?.usedBytes ?? 0);
  const total = $derived(mem?.totalBytes ?? 1);
  const pct = $derived(Math.min(100, (used / total) * 100));

  function gb(bytes: number): string {
    return (bytes / 1_073_741_824).toFixed(1);
  }
</script>

<DashboardCard title="Memory" flush>
  {#if !mem}
    <p class="text-xs text-fg-subtle py-2">Sampling…</p>
  {:else}
    <div class="space-y-2">
      <div class="flex items-baseline justify-between text-xs">
        <span class="text-fg-muted">Used</span>
        <span class="text-fg font-mono tabular-nums">
          {gb(used)} / {gb(total)} GB
        </span>
      </div>
      <div class="h-2 rounded-full bg-surface-2 overflow-hidden">
        <div
          class="h-full bg-status-running transition-all duration-500"
          style:width="{pct}%"
        ></div>
      </div>
      <div class="text-[11px] text-fg-subtle tabular-nums">
        {pct.toFixed(1)}% in use
      </div>
    </div>
  {/if}
</DashboardCard>
