/**
 * Schema-diagram layout, ported from tabularis `getLayoutedElements`
 * (src/components/ui/SchemaDiagram.tsx). Runs dagre over the table nodes so
 * FK-related tables cluster and edges route cleanly — replacing the old naive
 * 3-column grid. Kept framework-agnostic (plain {id, position, data} objects)
 * for testability.
 */
import dagre from "dagre";
import { Position, type Node, type Edge } from "@xyflow/svelte";

export type ErdDirection = "LR" | "TB";

const NODE_WIDTH = 240;

/**
 * Lay out `nodes`/`edges` with dagre. Mirrors the reference exactly, including
 * its deliberate LR<->TB inversion (the UI label is swapped against the dagre
 * rankdir to correct an upstream display quirk) and per-table height estimate
 * (`40 + columns * 28`) so taller tables get proportionally more spacing.
 */
export function layoutSchema(
  nodes: Node[],
  edges: Edge[],
  direction: ErdDirection = "TB",
): { nodes: Node[]; edges: Edge[] } {
  const graph = new dagre.graphlib.Graph();
  graph.setDefaultEdgeLabel(() => ({}));

  const dagreDirection = direction === "LR" ? "TB" : "LR";
  graph.setGraph({ rankdir: dagreDirection, ranksep: 150, nodesep: 50 });

  for (const node of nodes) {
    const columns = (node.data?.columns as unknown[] | undefined)?.length ?? 0;
    const height = 40 + columns * 28;
    graph.setNode(node.id, { width: NODE_WIDTH, height });
  }

  for (const edge of edges) {
    graph.setEdge(edge.source, edge.target);
  }

  dagre.layout(graph);

  const layouted = nodes.map((node) => {
    const pos = graph.node(node.id);
    return {
      ...node,
      targetPosition: direction === "LR" ? Position.Top : Position.Left,
      sourcePosition: direction === "LR" ? Position.Bottom : Position.Right,
      position: {
        x: pos.x - NODE_WIDTH / 2,
        y: pos.y - pos.height / 2,
      },
    };
  });

  return { nodes: layouted, edges };
}

/** Show the minimap only for medium schemas (10–100 tables), like the reference. */
export function shouldShowMinimap(tableCount: number): boolean {
  return tableCount >= 10 && tableCount <= 100;
}
