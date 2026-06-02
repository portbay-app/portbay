<script lang="ts">
  /**
   * TableDoc — browse rows for a specific table.
   * Header: table name + column/FK count + Refresh + Data⇄Chart toggle.
   * Body: DataGrid (Data view) or ResultChart (Chart view).
   */
  import { onMount } from "svelte";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import DataGrid from "$lib/components/databases/DataGrid.svelte";
  import ResultChart from "$lib/components/databases/ResultChart.svelte";

  import { safeInvoke } from "$lib/ipc";
  import type { DatabaseInstanceView, DbClientRows } from "$lib/types/databases";

  interface Props {
    instance: DatabaseInstanceView;
    schema: string | null | undefined;
    table: string;
  }

  let { instance, schema, table }: Props = $props();

  let rows = $state<DbClientRows | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let view = $state<"data" | "chart">("data");

  async function loadRows() {
    loading = true;
    error = null;
    try {
      rows = await safeInvoke<DbClientRows>("database_client_table_rows", {
        id: instance.id,
        schema: schema ?? null,
        table,
        limit: 100,
        offset: 0,
      });
    } catch (err) {
      rows = null;
      const msg = err instanceof Error ? err.message : (typeof err === "string" ? err : null);
      error = msg || "Could not load rows for this table.";
    } finally {
      loading = false;
    }
  }

  // Reload whenever instance or table identity changes.
  $effect(() => {
    void instance.id;
    void schema;
    void table;
    void loadRows();
  });

  const tableRef = $derived(schema ? `${schema}.${table}` : table);

  // Figure out FK count from the rows schema — we don't have DbClientTable here,
  // but we can derive it from rows.columns if available.
  // The column count comes from rows.columns.length.
  const columnCount = $derived(rows?.columns.length ?? 0);
</script>

<div class="h-full flex flex-col min-h-0">
  <!-- Header strip -->
  <div
    class="shrink-0 px-5 py-3 border-b border-border/60 flex items-center justify-between gap-3 bg-surface/60"
  >
    <div class="min-w-0">
      <h2 class="text-[14px] font-semibold text-fg truncate">{tableRef}</h2>
      {#if columnCount > 0}
        <p class="text-[11px] text-fg-subtle">{columnCount} columns</p>
      {/if}
    </div>

    <div class="flex items-center gap-2 shrink-0">
      <!-- Data ⇄ Chart toggle -->
      <div
        class="flex items-center rounded-md border border-border overflow-hidden text-[11px]"
      >
        <button
          type="button"
          onclick={() => (view = "data")}
          class="h-7 px-3 transition-colors
                 {view === 'data'
            ? 'bg-accent text-on-accent font-medium'
            : 'text-fg-muted hover:bg-surface-2 hover:text-fg'}"
        >
          Data
        </button>
        <button
          type="button"
          onclick={() => (view = "chart")}
          class="h-7 px-3 border-l border-border transition-colors
                 {view === 'chart'
            ? 'bg-accent text-on-accent font-medium'
            : 'text-fg-muted hover:bg-surface-2 hover:text-fg'}"
        >
          Chart
        </button>
      </div>

      <button
        type="button"
        onclick={loadRows}
        disabled={loading}
        title="Refresh rows"
        aria-label="Refresh rows"
        class="inline-flex items-center justify-center w-8 h-8 rounded-md
               border border-border bg-surface text-fg-muted hover:bg-surface-2
               hover:text-fg disabled:opacity-50 transition-colors"
      >
        <Icon name="refresh-cw" size={12} class={loading ? "animate-spin" : ""} />
      </button>
    </div>
  </div>

  <!-- Body -->
  <div class="flex-1 min-h-0 overflow-hidden">
    {#if view === "chart"}
      <div class="h-full overflow-auto p-4">
        <ResultChart {rows} />
      </div>
    {:else}
      <DataGrid
        {rows}
        {loading}
        {error}
        exportName={tableRef}
        emptyText="No rows returned."
        editable={{ instanceId: instance.id, schema: schema ?? null, table }}
        onApplied={loadRows}
      />
    {/if}
  </div>
</div>
