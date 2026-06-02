<script lang="ts">
  /**
   * Visual query builder canvas. Lives inside <SvelteFlowProvider>. Left rail is
   * a palette of tables; clicking one drops a node. Each node exposes its
   * columns (select + aggregate); dragging between column handles draws a JOIN
   * edge (click an edge to cycle its join type). A live SQL panel reflects the
   * graph and can be opened in a SQL scratchpad. Ported in spirit from tabularis'
   * VisualQueryBuilder, rebuilt on @xyflow/svelte.
   */
  import {
    SvelteFlow,
    Background,
    Controls,
    useSvelteFlow,
    type Node,
    type Edge,
    type Connection,
  } from "@xyflow/svelte";
  import "@xyflow/svelte/dist/style.css";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import QueryBuilderNode from "./QueryBuilderNode.svelte";
  import {
    generateSql,
    JOIN_TYPES,
    type Aggregate,
    type BuilderColumn,
    type BuilderJoin,
    type BuilderSettings,
    type BuilderTable,
    type JoinType,
  } from "./visualQuery";
  import type { DbClientSchema, DbClientTable } from "$lib/types/databases";

  interface Props {
    schema: DbClientSchema;
    onOpenQuery: (sql: string) => void;
  }

  let { schema, onOpenQuery }: Props = $props();

  const nodeTypes = { builderTable: QueryBuilderNode };
  const { fitView, screenToFlowPosition } = useSvelteFlow();

  let nodes = $state.raw<Node[]>([]);
  let edges = $state.raw<Edge[]>([]);
  let seq = 0;

  let settings = $state<BuilderSettings>({
    where: "",
    orderBy: [],
    limit: 100,
    distinct: false,
  });

  function tableKey(table: DbClientTable): string {
    return `${table.schema ?? ""}.${table.name}`;
  }

  function updateNodeData(id: string, mutate: (data: Record<string, unknown>) => void) {
    nodes = nodes.map((n) => {
      if (n.id !== id) return n;
      const data = { ...n.data };
      mutate(data);
      return { ...n, data };
    });
  }

  function addTable(table: DbClientTable, position?: { x: number; y: number }) {
    seq += 1;
    const id = `${tableKey(table)}#${seq}`;
    const columns: BuilderColumn[] = table.columns.map((c) => ({
      name: c.name,
      selected: false,
      aggregate: "" as Aggregate,
    }));
    const count = nodes.length;
    const node: Node = {
      id,
      type: "builderTable",
      position:
        position ?? { x: 60 + (count % 4) * 300, y: 40 + Math.floor(count / 4) * 260 },
      data: {
        name: table.name,
        columns,
        pkColumns: table.columns.filter((c) => c.primaryKey).map((c) => c.name),
        onToggle: (col: string) =>
          updateNodeData(id, (d) => {
            d.columns = (d.columns as BuilderColumn[]).map((c) =>
              c.name === col ? { ...c, selected: !c.selected } : c,
            );
          }),
        onAggregate: (col: string, agg: Aggregate) =>
          updateNodeData(id, (d) => {
            d.columns = (d.columns as BuilderColumn[]).map((c) =>
              c.name === col ? { ...c, aggregate: agg, selected: agg ? true : c.selected } : c,
            );
          }),
        onSelectAll: (selected: boolean) =>
          updateNodeData(id, (d) => {
            d.columns = (d.columns as BuilderColumn[]).map((c) => ({ ...c, selected }));
          }),
        onRemove: () => removeTable(id),
      },
    };
    nodes = [...nodes, node];
    // Click-added nodes have no position, so frame the whole graph. A
    // drag-dropped node carries its drop position and must stay put.
    if (!position) queueMicrotask(() => void fitView({ padding: 0.2, maxZoom: 1 }));
  }

  // Pointer-based palette drag. We deliberately avoid the HTML5 drag-and-drop
  // API: Tauri's native file-drop (`dragDropEnabled`, needed elsewhere for
  // folder/attachment drops) intercepts DOM drop events, so HTML5 `ondrop`
  // never fires here — and the webview's fallback tries to `shell.open` the
  // dragged item. Pointer events sidestep both.
  let canvasEl = $state<HTMLDivElement | null>(null);
  let drag = $state<{
    table: DbClientTable;
    startX: number;
    startY: number;
    x: number;
    y: number;
    active: boolean;
  } | null>(null);

  function onPalettePointerDown(e: PointerEvent, table: DbClientTable) {
    if (e.button !== 0) return;
    e.preventDefault();
    drag = { table, startX: e.clientX, startY: e.clientY, x: e.clientX, y: e.clientY, active: false };
  }

  function onWindowPointerMove(e: PointerEvent) {
    if (!drag) return;
    const movedFar =
      Math.abs(e.clientX - drag.startX) > 4 || Math.abs(e.clientY - drag.startY) > 4;
    drag = { ...drag, x: e.clientX, y: e.clientY, active: drag.active || movedFar };
  }

  function onWindowPointerUp(e: PointerEvent) {
    if (!drag) return;
    const d = drag;
    drag = null;
    if (!d.active) {
      // No real movement — treat as a click and place with the grid default.
      addTable(d.table);
      return;
    }
    const rect = canvasEl?.getBoundingClientRect();
    const overCanvas =
      !!rect &&
      e.clientX >= rect.left &&
      e.clientX <= rect.right &&
      e.clientY >= rect.top &&
      e.clientY <= rect.bottom;
    if (overCanvas) {
      addTable(d.table, screenToFlowPosition({ x: e.clientX, y: e.clientY }));
    }
    // Released outside the canvas → no-op.
  }

  function removeTable(id: string) {
    nodes = nodes.filter((n) => n.id !== id);
    edges = edges.filter((e) => e.source !== id && e.target !== id);
  }

  function onconnect(conn: Connection) {
    if (!conn.source || !conn.target || conn.source === conn.target) return;
    const id = `j-${conn.source}.${conn.sourceHandle}->${conn.target}.${conn.targetHandle}`;
    if (edges.some((e) => e.id === id)) return;
    const edge: Edge = {
      id,
      source: conn.source,
      target: conn.target,
      sourceHandle: conn.sourceHandle,
      targetHandle: conn.targetHandle,
      type: "smoothstep",
      label: "INNER",
      data: { joinType: "INNER" as JoinType },
      style: "stroke: var(--color-accent); stroke-width: 1.5;",
    };
    edges = [...edges, edge];
  }

  function cycleJoin(edgeId: string) {
    edges = edges.map((e) => {
      if (e.id !== edgeId) return e;
      const current = (e.data?.joinType as JoinType) ?? "INNER";
      const next = JOIN_TYPES[(JOIN_TYPES.indexOf(current) + 1) % JOIN_TYPES.length];
      return { ...e, label: next, data: { ...e.data, joinType: next } };
    });
  }

  // ─── Derive builder state + SQL from the live graph ───
  const builderTables = $derived<BuilderTable[]>(
    nodes.map((n) => ({
      id: n.id,
      name: n.data.name as string,
      columns: n.data.columns as BuilderColumn[],
    })),
  );
  const builderJoins = $derived<BuilderJoin[]>(
    edges.map((e) => ({
      sourceId: e.source,
      sourceColumn: (e.sourceHandle as string) ?? "",
      targetId: e.target,
      targetColumn: (e.targetHandle as string) ?? "",
      type: (e.data?.joinType as JoinType) ?? "INNER",
    })),
  );
  const generatedSql = $derived(generateSql(builderTables, builderJoins, settings));

  let tableFilter = $state("");
  const filteredTables = $derived(
    schema.tables.filter((t) =>
      t.name.toLowerCase().includes(tableFilter.trim().toLowerCase()),
    ),
  );

  function addOrderBy() {
    settings.orderBy = [...settings.orderBy, { expr: "", dir: "ASC" }];
  }
  function removeOrderBy(i: number) {
    settings.orderBy = settings.orderBy.filter((_, idx) => idx !== i);
  }

  async function copySql() {
    if (generatedSql) await navigator.clipboard.writeText(generatedSql);
  }
</script>

<svelte:window onpointermove={onWindowPointerMove} onpointerup={onWindowPointerUp} />

{#if drag?.active}
  <div
    class="pointer-events-none fixed z-[80] -translate-x-1/2 -translate-y-1/2 rounded-md
           border border-accent/60 bg-surface px-2 py-1 text-[11px] text-fg shadow-lg"
    style="left: {drag.x}px; top: {drag.y}px;"
  >
    {drag.table.name}
  </div>
{/if}

<div class="h-full flex min-h-0">
  <!-- Table palette -->
  <aside class="w-52 shrink-0 border-r border-border/60 bg-surface/40 flex flex-col min-h-0">
    <div class="px-2.5 py-2 border-b border-border/60">
      <label
        class="inline-flex h-7 w-full items-center gap-1.5 rounded-md border border-border
               bg-surface-2/50 px-2 text-[11px] text-fg-subtle"
      >
        <Icon name="search" size={10} />
        <input
          bind:value={tableFilter}
          placeholder="Tables"
          class="min-w-0 flex-1 bg-transparent text-fg focus:outline-none"
        />
      </label>
    </div>
    <div class="flex-1 min-h-0 overflow-auto py-1">
      {#each filteredTables as table (tableKey(table))}
        <button
          type="button"
          onpointerdown={(e) => onPalettePointerDown(e, table)}
          title="Click or drag onto the canvas"
          class="w-full flex items-center gap-2 px-2.5 py-1.5 text-left text-[12px]
                 text-fg-muted hover:bg-surface-2 hover:text-fg transition-colors
                 cursor-grab active:cursor-grabbing select-none"
        >
          <Icon name="database" size={11} class="text-fg-subtle shrink-0" />
          <span class="truncate flex-1">{table.name}</span>
          <Icon name="plus" size={11} class="text-fg-subtle shrink-0" />
        </button>
      {:else}
        <p class="px-2.5 py-2 text-[11px] text-fg-subtle">No tables.</p>
      {/each}
    </div>
  </aside>

  <!-- Canvas + SQL -->
  <div class="flex-1 min-w-0 flex flex-col min-h-0">
    <div class="flex-1 min-h-0 relative" bind:this={canvasEl}>
      {#if nodes.length === 0}
        <div
          class="absolute inset-0 z-10 flex items-center justify-center pointer-events-none"
        >
          <p class="text-[12px] text-fg-subtle text-center max-w-xs">
            Add tables from the left, tick columns, then drag between column dots to
            create a JOIN.
          </p>
        </div>
      {/if}
      <SvelteFlow
        bind:nodes
        bind:edges
        {nodeTypes}
        colorMode="dark"
        proOptions={{ hideAttribution: true }}
        fitView
        minZoom={0.2}
        maxZoom={1.5}
        defaultEdgeOptions={{ type: "smoothstep" }}
        {onconnect}
        onedgeclick={({ edge }) => cycleJoin(edge.id)}
      >
        <Background gap={20} size={1} />
        <Controls />
      </SvelteFlow>
    </div>

    <!-- Settings + generated SQL -->
    <div class="shrink-0 border-t border-border/60 bg-surface/60 max-h-[44%] overflow-auto">
      <div class="px-3 py-2 flex flex-wrap items-center gap-2 border-b border-border/40">
        <label class="inline-flex items-center gap-1.5 text-[11px] text-fg-subtle select-none">
          <input type="checkbox" bind:checked={settings.distinct} class="accent-accent" />
          DISTINCT
        </label>
        <label class="inline-flex items-center gap-1.5 text-[11px] text-fg-subtle">
          LIMIT
          <input
            type="number"
            min="0"
            value={settings.limit ?? ""}
            oninput={(e) => {
              const v = e.currentTarget.value;
              settings.limit = v === "" ? null : Number(v);
            }}
            class="h-7 w-20 rounded-md border border-border bg-surface px-2 text-[11px] text-fg"
          />
        </label>
        <button
          type="button"
          onclick={addOrderBy}
          class="inline-flex h-7 items-center gap-1 rounded-md border border-border px-2
                 text-[11px] text-fg-muted hover:bg-surface-2 hover:text-fg"
        >
          <Icon name="plus" size={10} /> Order by
        </button>
      </div>

      <div class="px-3 py-2 space-y-2">
        <label class="block">
          <span class="block mb-1 text-[10.5px] uppercase tracking-wide text-fg-subtle">Where</span>
          <input
            bind:value={settings.where}
            placeholder="t1.status = 'active'"
            class="w-full h-8 rounded-md border border-border bg-surface px-2 font-mono
                   text-[12px] text-fg placeholder:text-fg-subtle/60"
          />
        </label>

        {#each settings.orderBy as ob, i (i)}
          <div class="flex items-center gap-2">
            <input
              bind:value={ob.expr}
              placeholder="t1.created_at"
              class="flex-1 h-7 rounded-md border border-border bg-surface px-2 font-mono
                     text-[11px] text-fg placeholder:text-fg-subtle/60"
            />
            <select
              bind:value={ob.dir}
              class="h-7 rounded-md border border-border bg-surface px-1.5 text-[11px] text-fg-muted"
            >
              <option value="ASC">ASC</option>
              <option value="DESC">DESC</option>
            </select>
            <button
              type="button"
              onclick={() => removeOrderBy(i)}
              class="text-fg-subtle/60 hover:text-status-crashed"
              aria-label="Remove order by"
            >
              <Icon name="x" size={12} />
            </button>
          </div>
        {/each}
      </div>

      <div class="px-3 pb-3">
        <div class="flex items-center justify-between mb-1">
          <span class="text-[10.5px] uppercase tracking-wide text-fg-subtle">Generated SQL</span>
          <div class="flex items-center gap-1.5">
            <button
              type="button"
              onclick={copySql}
              disabled={!generatedSql}
              class="inline-flex h-6 items-center gap-1 rounded border border-border px-1.5
                     text-[10.5px] text-fg-muted hover:bg-surface-2 hover:text-fg disabled:opacity-40"
            >
              <Icon name="copy" size={10} /> Copy
            </button>
            <button
              type="button"
              onclick={() => onOpenQuery(generatedSql)}
              disabled={!generatedSql}
              class="inline-flex h-6 items-center gap-1 rounded bg-accent px-2
                     text-[10.5px] font-medium text-on-accent hover:brightness-110 disabled:opacity-40"
            >
              <Icon name="terminal" size={10} /> Open in query
            </button>
          </div>
        </div>
        <pre
          class="w-full overflow-x-auto rounded-md bg-surface-2 border border-border px-2.5 py-2
                 text-[11.5px] font-mono text-fg leading-relaxed whitespace-pre min-h-[40px]">{generatedSql ||
            "-- add a table to start building"}</pre>
      </div>
    </div>
  </div>
</div>
