<script lang="ts">
  import Icon from "$lib/components/atoms/Icon.svelte";
  import { safeInvoke } from "$lib/ipc";
  import type {
    DatabaseInstanceView,
    DbExplainPlan,
    DbClientRows,
    DbClientSchema,
    DbClientTable,
  } from "$lib/types/databases";
  import ExplainView from "./ExplainView.svelte";
  import JsonTree from "./JsonTree.svelte";
  import QueryBuilder from "./QueryBuilder.svelte";
  import ResultChart from "./ResultChart.svelte";
  import SchemaDiagram from "./SchemaDiagram.svelte";

  interface Props {
    instance: DatabaseInstanceView;
    initialSchema?: string | null;
    compact?: boolean;
  }

  let { instance, initialSchema = null, compact = false }: Props = $props();

  let loadingSchema = $state<boolean>(false);
  let loadingRows = $state<boolean>(false);
  let runningQuery = $state<boolean>(false);
  let explaining = $state<boolean>(false);
  let schemaError = $state<string | null>(null);
  let rowsError = $state<string | null>(null);
  let explainError = $state<string | null>(null);
  let schema = $state<DbClientSchema | null>(null);
  let selectedKey = $state<string | null>(null);
  let rows = $state<DbClientRows | null>(null);
  let explainPlan = $state<DbExplainPlan | null>(null);
  let sql = $state<string>("");
  let analyzeExplain = $state<boolean>(false);
  let inspected = $state<{ label: string; value: unknown } | null>(null);
  let rowFilter = $state<string>("");
  let sortColumn = $state<string | null>(null);
  let sortDirection = $state<"asc" | "desc">("asc");
  let page = $state<number>(0);
  let pageSize = $state<number>(25);
  let queryHistory = $state<string[]>([]);
  let view = $state<"data" | "diagram" | "builder" | "explain" | "chart">("data");
  const tabs: Array<{ id: typeof view; label: string }> = [
    { id: "data", label: "Data" },
    { id: "diagram", label: "ERD" },
    { id: "builder", label: "Builder" },
    { id: "explain", label: "Explain" },
    { id: "chart", label: "Chart" },
  ];
  const pageSizes = [25, 50, 100];

  const supported = $derived(
    ["mysql", "mariadb", "postgres", "sqlite"].includes(instance.engine),
  );

  const selectedTable = $derived.by(() => {
    if (!schema || !selectedKey) return null;
    return schema.tables.find((table) => tableKey(table) === selectedKey) ?? null;
  });

  const groupedTables = $derived.by(() => {
    const groups = new Map<string, DbClientTable[]>();
    for (const table of schema?.tables ?? []) {
      const group = table.schema ?? "main";
      groups.set(group, [...(groups.get(group) ?? []), table]);
    }
    return Array.from(groups.entries());
  });

  const filteredRows = $derived.by(() => {
    if (!rows) return [];
    const query = rowFilter.trim().toLowerCase();
    const sortIndex = sortColumn
      ? rows.columns.findIndex((column) => column.name === sortColumn)
      : -1;
    let nextRows = query
      ? rows.rows.filter((row) =>
          row.some((value) => cellText(value).toLowerCase().includes(query)),
        )
      : [...rows.rows];

    if (sortIndex >= 0) {
      nextRows = nextRows.toSorted((left, right) => {
        const result = compareCellValues(left[sortIndex], right[sortIndex]);
        return sortDirection === "asc" ? result : -result;
      });
    }
    return nextRows;
  });

  const pageCount = $derived(Math.max(1, Math.ceil(filteredRows.length / pageSize)));
  const visibleRows = $derived(
    filteredRows.slice(page * pageSize, page * pageSize + pageSize),
  );
  const pageStart = $derived(filteredRows.length === 0 ? 0 : page * pageSize + 1);
  const pageEnd = $derived(Math.min(filteredRows.length, (page + 1) * pageSize));

  function tableKey(table: DbClientTable): string {
    return `${table.schema ?? ""}.${table.name}`;
  }

  function tableLabel(table: DbClientTable): string {
    return table.schema ? `${table.schema}.${table.name}` : table.name;
  }

  function isInspectable(value: unknown): boolean {
    return Array.isArray(value) || (typeof value === "object" && value !== null);
  }

  function cellText(value: unknown): string {
    if (value === null || value === undefined) return "NULL";
    if (typeof value === "string") return value;
    if (typeof value === "number" || typeof value === "boolean") return String(value);
    return JSON.stringify(value);
  }

  function compareCellValues(left: unknown, right: unknown): number {
    if (left === right) return 0;
    if (left === null || left === undefined) return 1;
    if (right === null || right === undefined) return -1;
    if (typeof left === "number" && typeof right === "number") return left - right;
    return cellText(left).localeCompare(cellText(right), undefined, {
      numeric: true,
      sensitivity: "base",
    });
  }

  function resetRowView() {
    inspected = null;
    rowFilter = "";
    sortColumn = null;
    sortDirection = "asc";
    page = 0;
  }

  function toggleSort(column: string) {
    if (sortColumn === column) {
      sortDirection = sortDirection === "asc" ? "desc" : "asc";
    } else {
      sortColumn = column;
      sortDirection = "asc";
    }
    page = 0;
  }

  function rememberQuery(query: string) {
    const trimmed = query.trim();
    if (!trimmed) return;
    queryHistory = [trimmed, ...queryHistory.filter((entry) => entry !== trimmed)].slice(
      0,
      8,
    );
  }

  function exportCsv() {
    if (!rows || rows.columns.length === 0) return;
    const names = rows.columns.map((column) => column.name);
    const csv = [names, ...filteredRows]
      .map((line) => line.map((value) => csvCell(value)).join(","))
      .join("\n");
    const blob = new Blob([csv], { type: "text/csv;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = `${selectedTable?.name ?? "query"}-rows.csv`;
    link.click();
    URL.revokeObjectURL(url);
  }

  function csvCell(value: unknown): string {
    const text = cellText(value);
    return /[",\n\r]/.test(text) ? `"${text.replaceAll('"', '""')}"` : text;
  }

  function setSqlFor(table: DbClientTable) {
    const ref = table.schema ? `${table.schema}.${table.name}` : table.name;
    sql = `SELECT * FROM ${ref} LIMIT 100`;
  }

  async function loadSchema() {
    if (!supported) return;
    loadingSchema = true;
    schemaError = null;
    rowsError = null;
    rows = null;
    inspected = null;
    try {
      schema = await safeInvoke<DbClientSchema>("database_client_schema", {
        id: instance.id,
      });
      const preferred =
        schema.tables.find((table) => initialSchema && table.schema === initialSchema) ??
        schema.tables[0] ??
        null;
      selectedKey = preferred ? tableKey(preferred) : null;
      if (preferred) {
        setSqlFor(preferred);
        await loadRows(preferred);
      }
    } catch {
      schema = null;
      selectedKey = null;
      schemaError = "Could not inspect this database.";
    } finally {
      loadingSchema = false;
    }
  }

  async function loadRows(table = selectedTable) {
    if (!table || loadingRows) return;
    loadingRows = true;
    rowsError = null;
    inspected = null;
    try {
      rows = await safeInvoke<DbClientRows>("database_client_table_rows", {
        id: instance.id,
        schema: table.schema ?? null,
        table: table.name,
        limit: 100,
        offset: 0,
      });
      resetRowView();
    } catch (err) {
      rows = null;
      const msg = err instanceof Error ? err.message : (typeof err === "string" ? err : null);
      rowsError = msg || "Could not load rows for this table.";
    } finally {
      loadingRows = false;
    }
  }

  async function selectTable(table: DbClientTable) {
    selectedKey = tableKey(table);
    setSqlFor(table);
    view = "data";
    await loadRows(table);
  }

  async function runQuery(table = selectedTable, query = sql) {
    if (!table || runningQuery) return;
    runningQuery = true;
    rowsError = null;
    try {
      rows = await safeInvoke<DbClientRows>("database_client_query", {
        id: instance.id,
        schema: table.schema ?? null,
        sql: query,
        limit: 100,
      });
      rememberQuery(query);
      resetRowView();
    } catch (err) {
      const msg = err instanceof Error ? err.message : (typeof err === "string" ? err : null);
      rowsError = msg || "The embedded client only allows read-only inspection queries.";
    } finally {
      runningQuery = false;
    }
  }

  async function runExplain() {
    if (!selectedTable || explaining) return;
    explaining = true;
    explainError = null;
    view = "explain";
    try {
      explainPlan = await safeInvoke<DbExplainPlan>("database_client_explain", {
        id: instance.id,
        schema: selectedTable.schema ?? null,
        sql,
        analyze: analyzeExplain,
      });
    } catch (err) {
      explainPlan = null;
      const msg = err instanceof Error ? err.message : (typeof err === "string" ? err : null);
      explainError = msg || "Could not explain this query.";
    } finally {
      explaining = false;
    }
  }

  async function applyBuiltQuery(nextSql: string, table: DbClientTable) {
    selectedKey = tableKey(table);
    sql = nextSql;
    view = "data";
    await runQuery(table, nextSql);
  }

  $effect(() => {
    const id = instance.id;
    const schemaName = initialSchema;
    queueMicrotask(() => {
      if (instance.id === id && initialSchema === schemaName) void loadSchema();
    });
  });

  $effect(() => {
    if (page >= pageCount) page = Math.max(0, pageCount - 1);
  });
</script>

{#if !supported}
  <article class="border border-border/70 rounded-lg bg-surface px-4 py-3">
    <p class="text-[12px] text-fg-muted">
      Embedded browsing is available for SQLite, MySQL, MariaDB, and PostgreSQL.
    </p>
  </article>
{:else}
  <article
    class="h-full flex flex-col border border-border/70 bg-surface overflow-hidden {compact
      ? 'rounded-lg'
      : 'rounded-xl'}"
  >
    <header
      class="shrink-0 px-4 py-3 border-b border-border/60 flex items-center justify-between gap-3"
    >
      <div class="min-w-0">
        <h3 class="text-[13px] font-semibold text-fg">Data Workbench</h3>
        <p class="text-[11px] text-fg-subtle truncate">
          Browse tables and run read-only SQL inside PortBay.
        </p>
      </div>
      <button
        type="button"
        onclick={loadSchema}
        disabled={loadingSchema}
        title="Refresh schema"
        aria-label="Refresh schema"
        class="shrink-0 inline-flex items-center justify-center w-8 h-8 rounded-md
               border border-border bg-surface text-fg-muted hover:bg-surface-2
               hover:text-fg disabled:opacity-50 transition-colors"
      >
        <Icon name="refresh-cw" size={12} class={loadingSchema ? "animate-spin" : ""} />
      </button>
    </header>

    {#if schemaError}
      <div class="px-4 py-4 text-[12px] text-status-crashed">{schemaError}</div>
    {:else if loadingSchema && !schema}
      <div class="px-4 py-4 text-[12px] text-fg-subtle">Inspecting schema...</div>
    {:else if schema && schema.tables.length === 0}
      <div class="px-4 py-4 text-[12px] text-fg-subtle">No user tables found.</div>
    {:else if schema}
      <div class="flex-1 min-h-0 {compact ? 'grid grid-cols-1' : 'grid grid-cols-[240px,1fr]'}">
        <aside
          class="{compact
            ? 'border-b'
            : 'border-r'} border-border/60 bg-surface/40 overflow-y-auto min-h-0"
        >
          {#each groupedTables as [group, tables] (group)}
            <div class="px-2 py-2">
              <div class="px-2 pb-1 text-[10px] uppercase tracking-wide text-fg-subtle">
                {group}
              </div>
              <div class="space-y-1">
                {#each tables as table (tableKey(table))}
                  <button
                    type="button"
                    onclick={() => selectTable(table)}
                    class="w-full flex items-center gap-2 px-2 py-1.5 rounded-md text-left
                           text-[12px] transition-colors {tableKey(table) === selectedKey
                      ? 'bg-accent/10 text-accent'
                      : 'text-fg-muted hover:bg-surface-2 hover:text-fg'}"
                  >
                    <Icon name="database" size={12} />
                    <span class="truncate">{table.name}</span>
                    <span class="ml-auto text-[10px] text-fg-subtle">
                      {table.columns.length}
                    </span>
                  </button>
                {/each}
              </div>
            </div>
          {/each}
        </aside>

        <section class="min-w-0 flex flex-col min-h-0">
          {#if selectedTable}
            <!-- Editor + table context (fixed height; never scrolls away) -->
            <div class="shrink-0 px-4 py-3 border-b border-border/60">
              <div class="flex items-start justify-between gap-3">
                <div class="min-w-0">
                  <h4 class="text-[13px] font-medium text-fg truncate">
                    {tableLabel(selectedTable)}
                  </h4>
                  <p class="text-[11px] text-fg-subtle">
                    {selectedTable.columns.length} columns
                    {#if selectedTable.foreignKeys.length > 0}
                       ·  {selectedTable.foreignKeys.length} foreign keys
                    {/if}
                  </p>
                </div>
                <div class="flex items-center gap-2 shrink-0">
                  {#if queryHistory.length > 0}
                    <select
                      aria-label="Query history"
                      class="h-7 max-w-[200px] rounded-md border border-border bg-surface
                             px-2 text-[11px] text-fg-muted focus:outline-none
                             focus:ring-1 focus:ring-accent/50"
                      onchange={(event) => {
                        const value = event.currentTarget.value;
                        if (value) sql = value;
                      }}
                    >
                      <option value="">Recent queries</option>
                      {#each queryHistory as entry (entry)}
                        <option value={entry}>{entry}</option>
                      {/each}
                    </select>
                  {/if}
                  <button
                    type="button"
                    onclick={() => loadRows(selectedTable)}
                    disabled={loadingRows}
                    class="inline-flex items-center gap-1.5 h-7 px-2 rounded-md
                           border border-border text-[11px] text-fg-muted
                           hover:bg-surface-2 hover:text-fg disabled:opacity-50"
                  >
                    <Icon name="refresh-cw" size={11} class={loadingRows ? "animate-spin" : ""} />
                    Rows
                  </button>
                </div>
              </div>

              <div class="mt-3 flex gap-2">
                <textarea
                  bind:value={sql}
                  spellcheck="false"
                  aria-label="SQL query"
                  rows="2"
                  class="flex-1 min-h-16 max-h-40 resize-y rounded-md border border-border bg-surface-2/60
                         px-3 py-2 font-mono text-[12px] text-fg focus:outline-none
                         focus:ring-1 focus:ring-accent/50"
                ></textarea>
                <button
                  type="button"
                  onclick={() => runQuery()}
                  disabled={runningQuery || !sql.trim()}
                  title="Run read-only query"
                  class="self-stretch inline-flex items-center justify-center w-10 rounded-md
                         bg-accent text-on-accent hover:brightness-110 disabled:opacity-50
                         disabled:cursor-not-allowed"
                >
                  <Icon name="play" size={13} class={runningQuery ? "animate-pulse" : ""} />
                </button>
              </div>

              <div class="mt-2 flex flex-wrap items-center gap-2">
                <button
                  type="button"
                  onclick={runExplain}
                  disabled={explaining || !sql.trim()}
                  class="inline-flex items-center gap-1.5 h-7 px-2.5 rounded-md
                         border border-border text-[11px] text-fg-muted hover:bg-surface-2
                         hover:text-fg disabled:opacity-50"
                >
                  <Icon name="activity" size={11} class={explaining ? "animate-pulse" : ""} />
                  Visual Explain
                </button>
                <label
                  class="inline-flex items-center gap-1.5 text-[11px] text-fg-subtle select-none cursor-pointer"
                  title="Run EXPLAIN ANALYZE for real timings (PostgreSQL)"
                >
                  <input type="checkbox" bind:checked={analyzeExplain} class="accent-accent" />
                  ANALYZE
                </label>
                {#if selectedTable.foreignKeys.length > 0}
                  <div class="ml-auto flex flex-wrap items-center gap-1.5">
                    {#each selectedTable.foreignKeys as fk, i (`${fk.column}-${i}`)}
                      <span
                        class="inline-flex items-center gap-1.5 rounded-md border border-border
                               bg-surface px-2 py-0.5 text-[10.5px] text-fg-muted"
                      >
                        <Icon name="link" size={10} />
                        <span class="font-mono">{fk.column}</span>
                        <span class="text-fg-subtle">→</span>
                        <span class="font-mono">{fk.refTable}.{fk.refColumn}</span>
                      </span>
                    {/each}
                  </div>
                {/if}
              </div>
            </div>

            <!-- View switcher tab bar (sticky; always visible) -->
            <div
              class="shrink-0 flex items-center gap-1 px-3 border-b border-border/60 bg-surface/40"
              role="tablist"
            >
              {#each tabs as tab (tab.id)}
                <button
                  type="button"
                  role="tab"
                  aria-selected={view === tab.id}
                  onclick={() => (view = tab.id)}
                  class="relative h-9 px-3 text-[12px] transition-colors {view === tab.id
                    ? 'text-fg font-medium'
                    : 'text-fg-muted hover:text-fg'}"
                >
                  {tab.label}
                  {#if view === tab.id}
                    <span
                      class="absolute left-2 right-2 -bottom-px h-[2px] rounded-full bg-accent"
                    ></span>
                  {/if}
                </button>
              {/each}
            </div>

            <!-- View body (fills remaining height; scroll contained here) -->
            <div class="flex-1 min-h-0 overflow-hidden">
              {#if view === "diagram" && schema}
                <div class="h-full p-2">
                  <SchemaDiagram {schema} {selectedKey} onSelect={selectTable} />
                </div>
              {:else if view === "builder" && schema}
                <div class="h-full overflow-auto p-4">
                  <QueryBuilder
                    tables={schema.tables}
                    selected={selectedTable}
                    onBuild={applyBuiltQuery}
                  />
                </div>
              {:else if view === "explain"}
                <ExplainView plan={explainPlan} isLoading={explaining} error={explainError} />
              {:else if view === "chart"}
                <div class="h-full overflow-auto p-4">
                  <ResultChart {rows} />
                </div>
              {:else}
                <!-- Data grid -->
                <div class="h-full flex min-h-0">
                  <div class="flex-1 min-w-0 flex flex-col min-h-0">
                    {#if rowsError}
                      <div class="px-4 py-4 text-[12px] text-status-crashed">{rowsError}</div>
                    {:else if loadingRows && !rows}
                      <p class="px-4 py-4 text-[12px] text-fg-subtle">Loading rows...</p>
                    {:else if rows && rows.columns.length > 0}
                      <!-- Toolbar (does not scroll) -->
                      <div
                        class="shrink-0 border-b border-border/60 bg-surface px-3 py-2
                               flex flex-wrap items-center gap-2"
                      >
                        <label
                          class="inline-flex h-8 min-w-[180px] flex-1 items-center gap-2 rounded-md
                                 border border-border bg-surface-2/50 px-2 text-[11px] text-fg-subtle"
                        >
                          <Icon name="search" size={11} />
                          <input
                            value={rowFilter}
                            placeholder="Filter rows"
                            class="min-w-0 flex-1 bg-transparent text-fg focus:outline-none"
                            oninput={(event) => {
                              rowFilter = event.currentTarget.value;
                              page = 0;
                            }}
                          />
                        </label>
                        <select
                          aria-label="Rows per page"
                          value={pageSize}
                          class="h-8 rounded-md border border-border bg-surface px-2 text-[11px] text-fg-muted"
                          onchange={(event) => {
                            pageSize = Number(event.currentTarget.value);
                            page = 0;
                          }}
                        >
                          {#each pageSizes as size (size)}
                            <option value={size}>{size} rows</option>
                          {/each}
                        </select>
                        <button
                          type="button"
                          onclick={exportCsv}
                          class="inline-flex h-8 items-center gap-1.5 rounded-md border
                                 border-border px-2 text-[11px] text-fg-muted hover:bg-surface-2 hover:text-fg"
                        >
                          <Icon name="file-text" size={11} />
                          CSV
                        </button>
                      </div>

                      <!-- Scroll region (only the rows scroll) -->
                      <div class="flex-1 min-h-0 overflow-auto">
                        <table class="min-w-full text-left text-[12px]">
                          <thead class="sticky top-0 bg-surface z-10">
                            <tr class="border-b border-border/60">
                              {#each rows.columns as col (col.name)}
                                <th
                                  class="px-3 py-2 font-medium text-fg-muted whitespace-nowrap"
                                  aria-sort={sortColumn === col.name
                                    ? (sortDirection === "asc" ? "ascending" : "descending")
                                    : "none"}
                                >
                                  <button
                                    type="button"
                                    onclick={() => toggleSort(col.name)}
                                    aria-label={sortColumn === col.name
                                      ? `Sort by ${col.name} ${sortDirection === "asc" ? "descending" : "ascending"}`
                                      : `Sort by ${col.name} ascending`}
                                    class="inline-flex items-center gap-1 rounded px-1 -ml-1 hover:bg-surface-2 hover:text-fg"
                                  >
                                    <span>{col.name}</span>
                                    {#if sortColumn === col.name}
                                      <Icon
                                        name={sortDirection === "asc" ? "chevron-up" : "chevron-down"}
                                        size={10}
                                      />
                                    {:else}
                                      <Icon name="chevrons-up-down" size={10} />
                                    {/if}
                                  </button>
                                  {#if col.dataType}
                                    <span class="ml-1 font-normal text-fg-subtle">{col.dataType}</span>
                                  {/if}
                                </th>
                              {/each}
                            </tr>
                          </thead>
                          <tbody>
                            {#each visibleRows as row, rowIndex (`${page}-${rowIndex}`)}
                              <tr class="border-b border-border/30 hover:bg-surface-2/50">
                                {#each row as value, colIndex (`${rowIndex}-${colIndex}`)}
                                  <td class="px-3 py-2 max-w-[260px] align-top font-mono text-[11px] text-fg-muted">
                                    {#if isInspectable(value)}
                                      <button
                                        type="button"
                                        onclick={() =>
                                          (inspected = {
                                            label: rows?.columns[colIndex]?.name ?? "value",
                                            value,
                                          })}
                                        class="block w-full text-left truncate text-accent hover:underline"
                                      >
                                        {cellText(value)}
                                      </button>
                                    {:else}
                                      <span
                                        class:italic={value === null}
                                        class:text-fg-subtle={value === null}
                                        class="block truncate"
                                        title={cellText(value)}
                                      >
                                        {cellText(value)}
                                      </span>
                                    {/if}
                                  </td>
                                {/each}
                              </tr>
                            {/each}
                          </tbody>
                        </table>
                        {#if filteredRows.length === 0}
                          <p class="px-4 py-4 text-[12px] text-fg-subtle">
                            No rows match the current filter.
                          </p>
                        {/if}
                      </div>

                      <!-- Pagination (does not scroll) -->
                      <div
                        class="shrink-0 border-t border-border/60 bg-surface px-3 py-2 flex flex-wrap
                               items-center justify-between gap-2 text-[11px] text-fg-subtle"
                      >
                        <span>
                          Showing {pageStart}-{pageEnd} of {filteredRows.length} rows
                          {#if rows.truncated}
                            <span class="text-fg-subtle/80">· refine your query to load more</span>
                          {/if}
                        </span>
                        <div class="flex items-center gap-1">
                          <button
                            type="button"
                            onclick={() => (page = Math.max(0, page - 1))}
                            disabled={page === 0}
                            aria-label="Previous page"
                            class="inline-flex h-7 w-7 items-center justify-center rounded-md
                                   border border-border text-fg-muted hover:bg-surface-2 disabled:opacity-40"
                          >
                            <Icon name="chevron-left" size={12} />
                          </button>
                          <span class="min-w-16 text-center">{page + 1} / {pageCount}</span>
                          <button
                            type="button"
                            onclick={() => (page = Math.min(pageCount - 1, page + 1))}
                            disabled={page >= pageCount - 1}
                            aria-label="Next page"
                            class="inline-flex h-7 w-7 items-center justify-center rounded-md
                                   border border-border text-fg-muted hover:bg-surface-2 disabled:opacity-40"
                          >
                            <Icon name="chevron-right" size={12} />
                          </button>
                        </div>
                      </div>
                    {:else if rows}
                      <p class="px-4 py-4 text-[12px] text-fg-subtle">Query returned no rows.</p>
                    {/if}
                  </div>

                  {#if inspected}
                    <aside class="w-[300px] shrink-0 border-l border-border/60 bg-surface/40 overflow-auto">
                      <div
                        class="sticky top-0 bg-surface px-3 py-2 border-b border-border/60
                               flex items-center justify-between gap-2"
                      >
                        <span class="text-[12px] font-medium text-fg">JSON</span>
                        <button
                          type="button"
                          onclick={() => (inspected = null)}
                          title="Close JSON viewer"
                          aria-label="Close JSON viewer"
                          class="p-1 rounded text-fg-subtle hover:text-fg hover:bg-surface-2"
                        >
                          <Icon name="x" size={12} />
                        </button>
                      </div>
                      <div class="p-3">
                        <JsonTree label={inspected.label} value={inspected.value} />
                      </div>
                    </aside>
                  {/if}
                </div>
              {/if}
            </div>
          {/if}
        </section>
      </div>
    {/if}
  </article>
{/if}
