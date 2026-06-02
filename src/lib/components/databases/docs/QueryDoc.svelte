<script lang="ts">
  /**
   * QueryDoc — SQL scratchpad.
   * Textarea + Run + Visual Explain + ANALYZE checkbox + recent-queries dropdown.
   * Results: Data⇄Chart toggle over DataGrid / ResultChart.
   * Read-only errors surface in the grid's error slot.
   */
  import { onMount } from "svelte";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import SqlEditor from "$lib/components/atoms/SqlEditor.svelte";
  import DataGrid from "$lib/components/databases/DataGrid.svelte";
  import ResultChart from "$lib/components/databases/ResultChart.svelte";

  import { safeInvoke } from "$lib/ipc";
  import { dbWorkspace } from "$lib/stores/dbWorkspace.svelte";
  import type { DatabaseInstanceView, DbClientRows } from "$lib/types/databases";
  import type { SQLNamespace } from "@codemirror/lang-sql";

  interface Props {
    instance: DatabaseInstanceView;
    schema: string | null | undefined;
    initialSql?: string;
  }

  let { instance, schema, initialSql = "" }: Props = $props();

  // The scratchpad editor owns its own sql state seeded from initialSql once on mount.
  // The tab is keyed by id so it remounts on navigation — no reactive follow needed.
  let sql = $state<string>("");

  onMount(() => {
    sql = initialSql;
    void dbWorkspace.loadSchema(instance.id);
  });

  // Build a table → columns map for SQL autocompletion from the cached schema.
  const sqlSchema = $derived.by<SQLNamespace>(() => {
    const entry = dbWorkspace.schemaEntry(instance.id);
    const ns: Record<string, string[]> = {};
    for (const t of entry.schema?.tables ?? []) {
      ns[t.name] = t.columns.map((c) => c.name);
    }
    return ns;
  });
  let rows = $state<DbClientRows | null>(null);
  let running = $state(false);
  let error = $state<string | null>(null);
  let analyzeExplain = $state(false);
  let view = $state<"data" | "chart">("data");

  // Last 8 distinct queries.
  let queryHistory = $state<string[]>([]);

  function rememberQuery(q: string) {
    const trimmed = q.trim();
    if (!trimmed) return;
    queryHistory = [trimmed, ...queryHistory.filter((e) => e !== trimmed)].slice(0, 8);
  }

  async function runQuery() {
    if (!sql.trim() || running) return;
    running = true;
    error = null;
    try {
      rows = await safeInvoke<DbClientRows>("database_client_query", {
        id: instance.id,
        schema: schema ?? null,
        sql,
        limit: 100,
      });
      rememberQuery(sql);
    } catch (err) {
      rows = null;
      const msg = err instanceof Error ? err.message : (typeof err === "string" ? err : null);
      error = msg || "The embedded client only allows read-only inspection queries.";
    } finally {
      running = false;
    }
  }

  function openExplain() {
    if (!sql.trim()) return;
    dbWorkspace.openExplain(instance.id, sql, schema ?? null);
  }
</script>

<div class="h-full flex flex-col min-h-0">
  <!-- Editor area (shrink-0; does not scroll) -->
  <div class="shrink-0 px-4 py-3 border-b border-border/60 bg-surface/60 space-y-2">
    <div class="flex items-start gap-2">
      <div class="flex-1 min-w-0">
        <SqlEditor
          value={sql}
          oninput={(v) => (sql = v)}
          onRun={runQuery}
          schema={sqlSchema}
        />
      </div>
      <button
        type="button"
        onclick={runQuery}
        disabled={running || !sql.trim()}
        title="Run query (⌘↵)"
        class="self-stretch inline-flex items-center justify-center w-10 rounded-md
               bg-accent text-on-accent hover:brightness-110
               disabled:opacity-50 disabled:cursor-not-allowed transition"
      >
        <Icon
          name={running ? "refresh-cw" : "play"}
          size={13}
          class={running ? "animate-spin" : ""}
        />
      </button>
    </div>

    <div class="flex flex-wrap items-center gap-2">
      <button
        type="button"
        onclick={openExplain}
        disabled={!sql.trim()}
        class="inline-flex items-center gap-1.5 h-7 px-2.5 rounded-md
               border border-border text-[11px] text-fg-muted hover:bg-surface-2
               hover:text-fg disabled:opacity-50 transition-colors"
      >
        <Icon name="activity" size={11} />
        Visual Explain
      </button>
      <label
        class="inline-flex items-center gap-1.5 text-[11px] text-fg-subtle
               select-none cursor-pointer"
        title="Run EXPLAIN ANALYZE for real timings (PostgreSQL)"
      >
        <input type="checkbox" bind:checked={analyzeExplain} class="accent-accent" />
        ANALYZE
      </label>

      {#if queryHistory.length > 0}
        <select
          aria-label="Query history"
          class="h-7 max-w-[220px] rounded-md border border-border bg-surface
                 px-2 text-[11px] text-fg-muted focus:outline-none
                 focus:ring-1 focus:ring-accent/50 ml-auto"
          onchange={(e) => {
            const val = e.currentTarget.value;
            if (val) sql = val;
            e.currentTarget.value = "";
          }}
        >
          <option value="">Recent queries</option>
          {#each queryHistory as entry (entry)}
            <option value={entry}>{entry.length > 60 ? `${entry.slice(0, 60)}…` : entry}</option>
          {/each}
        </select>
      {/if}

      <!-- Data ⇄ Chart toggle -->
      <div
        class="flex items-center rounded-md border border-border overflow-hidden
               text-[11px] {queryHistory.length === 0 ? 'ml-auto' : ''}"
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
    </div>
  </div>

  <!-- Results body -->
  <div class="flex-1 min-h-0 overflow-hidden">
    {#if view === "chart"}
      <div class="h-full overflow-auto p-4">
        <ResultChart {rows} />
      </div>
    {:else}
      <DataGrid
        {rows}
        loading={running}
        {error}
        exportName="query-result"
        emptyText="Run a query to see results."
      />
    {/if}
  </div>
</div>
