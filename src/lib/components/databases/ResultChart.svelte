<script lang="ts">
  import { BarChart, LineChart, PieChart } from "layerchart";

  import type { DbClientRows } from "$lib/types/databases";
  import {
    canRenderChart,
    getNumericColumns,
    getLabelColumns,
    buildDefaultConfig,
    transformToChartData,
    paletteColor,
    type ChartConfig,
    type ChartRow,
  } from "./chartData";

  interface Props {
    rows: DbClientRows | null;
  }

  let { rows }: Props = $props();

  // ── config: re-initialise when rows identity changes ─────────────────────
  // Initialise without capturing the prop reference in $state() itself.
  let config = $state<ChartConfig | null>(null);

  $effect(() => {
    const r = rows; // track identity
    config = r ? buildDefaultConfig(r) : null;
  });

  // ── derived ───────────────────────────────────────────────────────────────
  const renderable = $derived(canRenderChart(rows));

  const numericCols = $derived.by((): string[] => (rows ? getNumericColumns(rows) : []));
  const labelCols = $derived.by((): string[] => (rows ? getLabelColumns(rows) : []));

  const chartData = $derived.by((): ChartRow[] => {
    if (!rows || !config || !renderable) return [];
    return transformToChartData(rows, config);
  });

  const isEmpty = $derived(chartData.length === 0);

  // ── series definitions for bar/line ──────────────────────────────────────
  const series = $derived.by(() => {
    if (!config) return [];
    return config.valueColumns.map((col, i) => ({
      key: col,
      label: col,
      value: col,
      color: paletteColor(i),
    }));
  });

  // ── pie data ──────────────────────────────────────────────────────────────
  const pieData = $derived.by(() => {
    if (!config || isEmpty) return [];
    const valueCol = config.valueColumns[0];
    return chartData.map((row, i) => ({
      key: row.label || `Row ${i + 1}`,
      label: row.label || `Row ${i + 1}`,
      value: (row[valueCol] as number) ?? 0,
    }));
  });

  const pieColors = $derived.by(() => pieData.map((_, i) => paletteColor(i)));

  // ── legend show/hide ──────────────────────────────────────────────────────
  const showLegend = $derived((config?.valueColumns.length ?? 0) > 1);

  // ── config mutation helpers ───────────────────────────────────────────────
  function setType(t: ChartConfig["type"]) {
    if (!config) return;
    config = { ...config, type: t };
  }

  function setLabel(col: string) {
    if (!config) return;
    config = { ...config, labelColumn: col };
  }

  function toggleValue(col: string) {
    if (!config) return;
    const has = config.valueColumns.includes(col);
    if (has && config.valueColumns.length <= 1) return;
    const valueColumns = has
      ? config.valueColumns.filter((c) => c !== col)
      : [...config.valueColumns, col];
    config = { ...config, valueColumns };
  }
</script>

<!-- ── empty / no-chart state ─────────────────────────────────────────────── -->
{#if !renderable || !config || isEmpty}
  <div
    class="rounded-lg border border-border/60 bg-surface-2/30 px-4 py-6 text-[12px] text-fg-subtle"
  >
    {#if !rows || rows.rows.length === 0}
      Run a query with results to chart them.
    {:else}
      This result cannot be charted — at least one numeric and one non-numeric column are required.
    {/if}
  </div>
{:else}
  <!-- ── toolbar ─────────────────────────────────────────────────────────────── -->
  <div
    class="mb-2 flex flex-wrap items-center gap-x-3 gap-y-1.5 rounded-lg border border-border/60 bg-surface-2/30 px-3 py-2"
  >
    <!-- chart type pills -->
    <div class="flex items-center gap-1">
      <span class="mr-1 text-[10px] font-semibold uppercase tracking-wide text-fg-muted"
        >Chart</span
      >
      {#each (["bar", "line", "pie"] as const) as t (t)}
        <button
          onclick={() => setType(t)}
          class="rounded px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide transition-colors
            {config.type === t
            ? 'bg-accent/10 border border-accent/50 text-accent'
            : 'border border-border/60 text-fg-muted hover:border-border hover:text-fg'}"
        >
          {t}
        </button>
      {/each}
    </div>

    <!-- label column selector -->
    {#if labelCols.length > 0}
      <div class="flex items-center gap-1">
        <span class="text-[10px] font-semibold uppercase tracking-wide text-fg-muted">Label</span>
        <select
          class="rounded border border-border/60 bg-surface px-1.5 py-0.5 text-[10px] text-fg focus:outline-none"
          value={config.labelColumn}
          onchange={(e) => setLabel((e.currentTarget as HTMLSelectElement).value)}
        >
          {#each labelCols as col (col)}
            <option value={col}>{col}</option>
          {/each}
        </select>
      </div>
    {/if}

    <!-- value column toggles -->
    {#if numericCols.length > 0}
      <div class="flex items-center gap-1">
        <span class="text-[10px] font-semibold uppercase tracking-wide text-fg-muted">Values</span>
        {#each numericCols as col, i (col)}
          <button
            onclick={() => toggleValue(col)}
            class="rounded px-2 py-0.5 text-[10px] font-medium transition-colors
              {config.valueColumns.includes(col)
              ? 'border text-white'
              : 'border border-border/60 text-fg-muted hover:border-border hover:text-fg'}"
            style={config.valueColumns.includes(col)
              ? `background-color:${paletteColor(i)};border-color:${paletteColor(i)}`
              : ""}
          >
            {col}
          </button>
        {/each}
      </div>
    {/if}
  </div>

  <!-- ── chart area ───────────────────────────────────────────────────────────── -->
  <div class="h-[250px] w-full p-2">
    {#if config.type === "bar"}
      <BarChart
        data={chartData}
        x="label"
        {series}
        seriesLayout={series.length > 1 ? "group" : "overlap"}
        legend={showLegend}
        props={{
          bars: { radius: 3, strokeWidth: 0 },
          yAxis: { ticks: 4 },
        }}
      />
    {:else if config.type === "line"}
      <LineChart
        data={chartData}
        x="label"
        {series}
        legend={showLegend}
        points
        props={{
          yAxis: { ticks: 4 },
        }}
      />
    {:else if config.type === "pie"}
      <PieChart
        data={pieData}
        key="key"
        label="label"
        value="value"
        c="key"
        cRange={pieColors}
        legend
      />
    {/if}
  </div>
{/if}
