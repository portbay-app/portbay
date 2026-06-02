<script lang="ts">
  /**
   * Inner explain-graph canvas (inside <SvelteFlowProvider>). Ported from
   * tabularis `ExplainGraphInner`: lays the plan tree out top-to-bottom with
   * dagre, renders each step as an ExplainPlanNode, and refits on change.
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
  import type { DbExplainPlan } from "$lib/types/databases";
  import ExplainPlanNode from "./ExplainPlanNode.svelte";
  import { explainPlanToFlow } from "./explainPlan";

  interface Props {
    plan: DbExplainPlan;
    selectedNodeId: string | null;
    onSelectNode: (id: string) => void;
  }

  let { plan, selectedNodeId, onSelectNode }: Props = $props();

  const nodeTypes = { explainPlan: ExplainPlanNode };
  const { fitView } = useSvelteFlow();

  let nodes = $state.raw<Node[]>([]);
  let edges = $state.raw<Edge[]>([]);

  const graph = $derived(explainPlanToFlow(plan, selectedNodeId));

  $effect(() => {
    const next = graph;
    nodes = next.nodes;
    edges = next.edges;
    queueMicrotask(() => void fitView({ padding: 0.2 }));
  });
</script>

<SvelteFlow
  bind:nodes
  bind:edges
  {nodeTypes}
  colorMode="dark"
  proOptions={{ hideAttribution: true }}
  fitView
  minZoom={0.1}
  maxZoom={2}
  nodesDraggable={false}
  nodesConnectable={false}
  onnodeclick={({ node }) => onSelectNode(node.id)}
>
  <Background gap={20} size={1} />
  <Controls />
  {#if nodes.length > 10}
    <MiniMap nodeColor={() => "#6366f1"} maskColor="rgba(15, 23, 42, 0.9)" />
  {/if}
</SvelteFlow>
