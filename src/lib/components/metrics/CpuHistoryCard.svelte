<!--
  CpuHistoryCard — sparkline of the last 60 CPU samples (2 minutes).
-->
<script lang="ts">
  import { DashboardCard } from "$lib/components/atoms";
  import { metrics } from "$lib/stores/metrics";

  const points = $derived.by(() => {
    const data = metrics.cpuHistory;
    if (data.length < 2) return "";
    const w = 280;
    const h = 70;
    const max = 100;
    const step = w / Math.max(1, data.length - 1);
    return data
      .map((v, i) => `${i * step},${h - (v / max) * h}`)
      .join(" ");
  });

  const areaPoints = $derived.by(() => {
    const data = metrics.cpuHistory;
    if (data.length < 2) return "";
    const w = 280;
    const h = 70;
    const step = w / Math.max(1, data.length - 1);
    const top = data
      .map((v, i) => `${i * step},${h - (v / 100) * h}`)
      .join(" ");
    return `0,${h} ${top} ${w},${h}`;
  });
</script>

<DashboardCard title="CPU history" flush>
  {#if metrics.cpuHistory.length < 2}
    <p class="text-xs text-fg-subtle py-2">Sampling…</p>
  {:else}
    <svg
      viewBox="0 0 280 70"
      class="w-full"
      preserveAspectRatio="none"
      aria-label="CPU usage history sparkline"
    >
      <polyline
        points={areaPoints}
        fill="var(--color-status-running)"
        opacity="0.15"
      />
      <polyline
        points={points}
        stroke="var(--color-status-running)"
        stroke-width="1.5"
        fill="none"
        stroke-linejoin="round"
      />
    </svg>
  {/if}
</DashboardCard>
