<script lang="ts">
  /**
   * Inner ERD canvas. Lives inside <SvelteFlowProvider> (see SchemaDiagram.svelte)
   * so it can call useSvelteFlow() for imperative fitView/zoom. Ported from
   * tabularis `SchemaDiagramContent`: dagre auto-layout, per-column FK edges,
   * click-to-focus filtering, a direction toggle, keyboard zoom, Controls and a
   * conditional MiniMap. Unlike the reference it receives the schema as a prop
   * (the workbench already loaded it) rather than fetching it.
   */
  import {
    SvelteFlow,
    Background,
    Controls,
    MiniMap,
    useSvelteFlow,
    type Node,
    type Edge,
  } from "@xyflow/svelte";
  import "@xyflow/svelte/dist/style.css";
  import { ArrowLeftRight, ArrowUpDown, Maximize2, Focus, Table2 } from "@lucide/svelte";
  import type { DbClientSchema, DbClientTable } from "$lib/types/databases";
  import SchemaTableNode from "./SchemaTableNode.svelte";
  import { layoutSchema, shouldShowMinimap, type ErdDirection } from "./erdLayout";

  interface Props {
    schema: DbClientSchema;
    selectedKey?: string | null;
    onSelect?: (table: DbClientTable) => void;
  }

  let { schema, selectedKey = null, onSelect }: Props = $props();

  const nodeTypes = { schemaTable: SchemaTableNode };
  const { fitView, zoomIn, zoomOut } = useSvelteFlow();

  let direction = $state<ErdDirection>("TB");
  let focused = $state<string | null>(null);
  let nodes = $state.raw<Node[]>([]);
  let edges = $state.raw<Edge[]>([]);
  let contextMenu = $state<{ x: number; y: number; key: string } | null>(null);

  function tableKey(table: DbClientTable): string {
    return `${table.schema ?? ""}.${table.name}`;
  }

  /** Build flow nodes/edges from the PortBay schema shape. */
  function buildGraph(input: DbClientSchema): { nodes: Node[]; edges: Edge[] } {
    const keys = new Set<string>();
    const byName = new Map<string, DbClientTable>();
    for (const table of input.tables) {
      keys.add(tableKey(table));
      byName.set(table.name, table);
    }

    const flowNodes: Node[] = [];
    const flowEdges: Edge[] = [];

    for (const table of input.tables) {
      const fkColumns = new Set(table.foreignKeys.map((fk) => fk.column));
      const key = tableKey(table);
      flowNodes.push({
        id: key,
        type: "schemaTable",
        position: { x: 0, y: 0 },
        data: {
          label: table.name,
          table,
          highlighted: key === selectedKey,
          columns: table.columns.map((column) => ({
            name: column.name,
            type: column.dataType,
            isPk: column.primaryKey,
            isFk: fkColumns.has(column.name),
          })),
        },
      });

      for (const fk of table.foreignKeys) {
        const sameSchemaKey = `${table.schema ?? ""}.${fk.refTable}`;
        const targetKey = keys.has(sameSchemaKey)
          ? sameSchemaKey
          : byName.has(fk.refTable)
            ? tableKey(byName.get(fk.refTable)!)
            : null;
        if (!targetKey) continue;
        flowEdges.push({
          id: `e-${key}-${fk.column}-${targetKey}-${fk.refColumn}`,
          source: key,
          target: targetKey,
          sourceHandle: fk.column,
          targetHandle: fk.refColumn,
          animated: flowEdges.length < 50,
          type: "smoothstep",
          style: "stroke: #6366f1; stroke-width: 1.5;",
        });
      }
    }

    return { nodes: flowNodes, edges: flowEdges };
  }

  // Full, laid-out graph (recomputes when schema/direction/selection changes).
  const fullGraph = $derived.by(() => {
    const built = buildGraph(schema);
    return layoutSchema(built.nodes, built.edges, direction);
  });

  // Focus mode: show only the focused table + its FK neighbours, re-laid out.
  const visibleGraph = $derived.by(() => {
    if (!focused) return fullGraph;
    const related = new Set<string>([focused]);
    for (const edge of fullGraph.edges) {
      if (edge.source === focused) related.add(edge.target);
      if (edge.target === focused) related.add(edge.source);
    }
    const fn = fullGraph.nodes.filter((node) => related.has(node.id));
    const fe = fullGraph.edges.filter(
      (edge) => related.has(edge.source) && related.has(edge.target),
    );
    return layoutSchema(fn, fe, direction);
  });

  const minimap = $derived(shouldShowMinimap(visibleGraph.nodes.length));

  // Push the laid-out graph into the flow and refit. Reads `visibleGraph` only,
  // so SvelteFlow's own writes to `nodes`/`edges` don't retrigger it.
  $effect(() => {
    const next = visibleGraph;
    nodes = next.nodes;
    edges = next.edges;
    queueMicrotask(() => void fitView({ padding: 0.2 }));
  });

  // Keyboard zoom (+/-), matching the reference.
  $effect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "+" || event.key === "=") {
        event.preventDefault();
        void zoomIn();
      } else if (event.key === "-" || event.key === "_") {
        event.preventDefault();
        void zoomOut();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  // Close the context menu on any outside interaction.
  $effect(() => {
    if (!contextMenu) return;
    const close = () => (contextMenu = null);
    window.addEventListener("click", close);
    window.addEventListener("scroll", close, true);
    return () => {
      window.removeEventListener("click", close);
      window.removeEventListener("scroll", close, true);
    };
  });

  function toggleDirection() {
    direction = direction === "LR" ? "TB" : "LR";
  }

  function tableFor(key: string): DbClientTable | undefined {
    return schema.tables.find((table) => tableKey(table) === key);
  }
</script>

<div class="relative h-full w-full bg-surface">
  <!-- Toolbar -->
  <div class="absolute top-3 left-3 z-10 flex gap-2">
    <button
      type="button"
      onclick={toggleDirection}
      class="flex items-center gap-1.5 px-2.5 py-1.5 rounded-md border border-border bg-surface-2
             text-[12px] font-medium text-fg-muted hover:text-fg hover:bg-surface shadow-lg transition-colors"
      title={direction === "LR" ? "Switch to vertical layout" : "Switch to horizontal layout"}
    >
      {#if direction === "LR"}
        <ArrowLeftRight size={14} />
        <span>Horizontal</span>
      {:else}
        <ArrowUpDown size={14} />
        <span>Vertical</span>
      {/if}
    </button>

    {#if focused}
      <button
        type="button"
        onclick={() => (focused = null)}
        class="flex items-center gap-1.5 px-2.5 py-1.5 rounded-md border border-indigo-500 bg-indigo-600
               text-[12px] font-medium text-white hover:bg-indigo-700 shadow-lg transition-colors"
        title="Show all tables"
      >
        <Maximize2 size={14} />
        <span>Show all</span>
      </button>
    {/if}
  </div>

  {#if focused}
    <div
      class="absolute top-3 right-3 z-10 px-3 py-1.5 rounded-md border border-indigo-500 bg-indigo-600
             text-[12px] font-medium text-white shadow-lg"
    >
      Focused: {focused.replace(/^\./, "")}
    </div>
  {/if}

  <SvelteFlow
    bind:nodes
    bind:edges
    {nodeTypes}
    colorMode="dark"
    proOptions={{ hideAttribution: true }}
    fitView
    minZoom={0.05}
    maxZoom={2}
    nodesDraggable={false}
    nodesConnectable={false}
    elementsSelectable
    panOnScroll={false}
    zoomOnScroll
    zoomOnDoubleClick={false}
    defaultEdgeOptions={{ type: "smoothstep" }}
    onnodeclick={({ node }) => {
      focused = focused === node.id ? null : node.id;
    }}
    onnodecontextmenu={({ event, node }) => {
      event.preventDefault();
      const mouse = event as MouseEvent;
      contextMenu = { x: mouse.clientX, y: mouse.clientY, key: node.id };
    }}
  >
    <Background gap={20} size={1} />
    <Controls />
    {#if minimap}
      <MiniMap nodeColor={() => "#6366f1"} maskColor="rgba(15, 23, 42, 0.9)" />
    {/if}
  </SvelteFlow>

  {#if contextMenu}
    {@const menu = contextMenu}
    <div
      class="fixed z-50 min-w-[180px] rounded-md border border-border bg-surface shadow-xl py-1 text-[12px]"
      style="left: {menu.x}px; top: {menu.y}px"
      role="menu"
      tabindex="-1"
    >
      <button
        type="button"
        role="menuitem"
        onclick={() => {
          focused = menu.key;
          contextMenu = null;
        }}
        class="w-full flex items-center gap-2 px-3 py-1.5 text-left text-fg-muted hover:bg-surface-2 hover:text-fg"
      >
        <Focus size={13} /> Focus on table
      </button>
      {#if onSelect}
        <button
          type="button"
          role="menuitem"
          onclick={() => {
            const table = tableFor(menu.key);
            if (table) onSelect?.(table);
            contextMenu = null;
          }}
          class="w-full flex items-center gap-2 px-3 py-1.5 text-left text-fg-muted hover:bg-surface-2 hover:text-fg"
        >
          <Table2 size={13} /> Open table data
        </button>
      {/if}
      {#if focused}
        <button
          type="button"
          role="menuitem"
          onclick={() => {
            focused = null;
            contextMenu = null;
          }}
          class="w-full flex items-center gap-2 px-3 py-1.5 text-left text-fg-muted hover:bg-surface-2 hover:text-fg"
        >
          <Maximize2 size={13} /> Show all
        </button>
      {/if}
    </div>
  {/if}
</div>
