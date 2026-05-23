<!--
  Sparkline — compact area-fill line over a series of numeric samples.

  Designed for status-card use: 100 × 32 by default, no axes, no labels.
  The line + soft fill share a colour token so the spark reads as one
  element. When the series has fewer than two points the component
  renders nothing (the caller renders its own "sampling…" hint).
-->
<script lang="ts">
  interface Props {
    /** Sample values. Plotted left-to-right; the last sample is the latest. */
    data: number[];
    /** Optional explicit max. Defaults to max(data); falls back to 1 so
        a flat zero series doesn't collapse to a single line. */
    max?: number;
    width?: number;
    height?: number;
    /** Stroke + fill colour token. CSS variable expected. */
    color?: string;
    /** Label for screen readers. */
    label?: string;
  }
  let {
    data,
    max,
    width = 100,
    height = 32,
    color = "var(--color-accent)",
    label = "Recent values",
  }: Props = $props();

  const computedMax = $derived.by(() => {
    if (max !== undefined) return max;
    const m = data.length === 0 ? 1 : Math.max(...data, 1);
    return m === 0 ? 1 : m;
  });

  const linePoints = $derived.by(() => {
    if (data.length < 2) return "";
    const step = width / Math.max(1, data.length - 1);
    return data
      .map((v, i) => `${i * step},${height - (v / computedMax) * height}`)
      .join(" ");
  });

  const areaPoints = $derived.by(() => {
    if (data.length < 2) return "";
    return `0,${height} ${linePoints} ${width},${height}`;
  });
</script>

{#if data.length >= 2}
  <svg
    viewBox="0 0 {width} {height}"
    class="w-full h-full"
    preserveAspectRatio="none"
    aria-label={label}
  >
    <polyline points={areaPoints} fill={color} opacity="0.15" />
    <polyline
      points={linePoints}
      stroke={color}
      stroke-width="1.5"
      fill="none"
      stroke-linejoin="round"
    />
  </svg>
{/if}
