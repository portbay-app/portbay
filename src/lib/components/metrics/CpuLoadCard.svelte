<!--
  CpuLoadCard — half-circle CPU gauge.

  Custom SVG, no charting library. Gradient stops at 40% / 75% / 90% map
  to green → amber → red, matching the screenshot's CPU dial.
-->
<script lang="ts">
  import { DashboardCard } from "$lib/components/atoms";
  import { metrics } from "$lib/stores/metrics.svelte";

  const cpu = $derived(metrics.value?.cpu.total ?? 0);
  // The needle's angle: 0% → -90° (left), 100% → +90° (right).
  const angle = $derived(-90 + (cpu / 100) * 180);

  const colorClass = $derived.by(() => {
    if (cpu >= 90) return "text-status-crashed";
    if (cpu >= 75) return "text-status-unhealthy";
    if (cpu >= 40) return "text-status-starting";
    return "text-status-running";
  });
</script>

<DashboardCard title="CPU Load" flush>
  <div class="flex flex-col items-center">
    <svg
      viewBox="0 0 200 110"
      class="w-full max-w-xs"
      aria-label="CPU load gauge"
    >
      <!-- Arc backgrounds: green / amber / red bands -->
      <path
        d="M 20 100 A 80 80 0 0 1 92 22"
        stroke="var(--color-status-running)"
        stroke-width="14"
        fill="none"
        opacity="0.4"
        stroke-linecap="round"
      />
      <path
        d="M 92 22 A 80 80 0 0 1 152 36"
        stroke="var(--color-status-unhealthy)"
        stroke-width="14"
        fill="none"
        opacity="0.4"
        stroke-linecap="round"
      />
      <path
        d="M 152 36 A 80 80 0 0 1 180 100"
        stroke="var(--color-status-crashed)"
        stroke-width="14"
        fill="none"
        opacity="0.4"
        stroke-linecap="round"
      />

      <!-- Needle -->
      <g transform="translate(100 100)">
        <line
          x1="0"
          y1="0"
          x2="0"
          y2="-70"
          stroke="currentColor"
          stroke-width="3"
          stroke-linecap="round"
          transform="rotate({angle})"
          class={colorClass}
          style="transition: transform 0.5s ease, color 0.3s"
        />
        <circle r="6" fill="var(--color-surface-2)" stroke="currentColor" stroke-width="2" class={colorClass} />
      </g>

      <!-- Tick labels -->
      <text x="10" y="105" font-size="10" fill="var(--color-fg-subtle)" text-anchor="middle">0</text>
      <text x="100" y="18" font-size="10" fill="var(--color-fg-subtle)" text-anchor="middle">50</text>
      <text x="190" y="105" font-size="10" fill="var(--color-fg-subtle)" text-anchor="middle">100</text>
    </svg>

    <div class="text-2xl font-semibold tabular-nums {colorClass} mt-1">
      {cpu.toFixed(1)}%
    </div>
  </div>
</DashboardCard>
