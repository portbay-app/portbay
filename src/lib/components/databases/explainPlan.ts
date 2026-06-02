/**
 * Query-plan helpers, ported from tabularis `src/utils/explainPlan.ts` and
 * adapted to PortBay's camelCase {@link DbExplainPlan} shape. Covers tree
 * traversal, cost/time formatting, the overview summary, node colour styling,
 * and the dagre tree→flow conversion used by the graph view.
 */
import dagre from "dagre";
import type { Node, Edge } from "@xyflow/svelte";
import type { DbExplainNode, DbExplainPlan } from "$lib/types/databases";

/** Which explain presentation is active. (No "ai" mode — see ExplainView.) */
export type ExplainViewMode = "graph" | "table" | "raw";

// ---------------------------------------------------------------------------
// Tree → flow conversion + dagre layout
// ---------------------------------------------------------------------------

export interface ExplainNodeData extends Record<string, unknown> {
  node: DbExplainNode;
  maxCost: number;
  maxTime: number;
  hasAnalyzeData: boolean;
  isSelected: boolean;
}

export function explainPlanToFlow(
  plan: DbExplainPlan,
  selectedNodeId?: string | null,
): { nodes: Node[]; edges: Edge[] } {
  const maxCost = getMaxCost(plan.root);
  const maxTime = getMaxTime(plan.root);
  const rawNodes: Node[] = [];
  const edges: Edge[] = [];

  function walk(node: DbExplainNode) {
    rawNodes.push({
      id: node.id,
      type: "explainPlan",
      position: { x: 0, y: 0 },
      data: {
        node,
        maxCost,
        maxTime,
        hasAnalyzeData: plan.hasAnalyzeData,
        isSelected: selectedNodeId === node.id,
      } satisfies ExplainNodeData,
    });
    for (const child of node.children) {
      edges.push({
        id: `${node.id}-${child.id}`,
        source: node.id,
        target: child.id,
        animated: true,
        type: "smoothstep",
        style: "stroke: #6366f1;",
      });
      walk(child);
    }
  }

  walk(plan.root);
  return layoutExplainNodes(rawNodes, edges);
}

export function layoutExplainNodes(
  nodes: Node[],
  edges: Edge[],
): { nodes: Node[]; edges: Edge[] } {
  const graph = new dagre.graphlib.Graph();
  graph.setDefaultEdgeLabel(() => ({}));
  graph.setGraph({ rankdir: "TB", ranksep: 80, nodesep: 40 });

  const NODE_WIDTH = 280;

  for (const node of nodes) {
    const data = node.data as ExplainNodeData;
    const lines = 3 + (data.hasAnalyzeData ? 1 : 0) + (data.node.filter ? 1 : 0);
    const height = 28 + lines * 22;
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
      position: { x: pos.x - NODE_WIDTH / 2, y: pos.y - pos.height / 2 },
    };
  });

  return { nodes: layouted, edges };
}

// ---------------------------------------------------------------------------
// Cost colour styling
// ---------------------------------------------------------------------------

export interface NodeCostStyle {
  border: string;
  headerBg: string;
}

export function getNodeCostStyle(cost: number, maxCost: number): NodeCostStyle {
  if (maxCost <= 0) return { border: "border-l-green-500", headerBg: "bg-green-950/30" };
  const ratio = cost / maxCost;
  if (ratio < 0.2) return { border: "border-l-green-500", headerBg: "bg-green-950/30" };
  if (ratio < 0.6) return { border: "border-l-yellow-500", headerBg: "bg-yellow-950/30" };
  return { border: "border-l-red-500", headerBg: "bg-red-950/30" };
}

// ---------------------------------------------------------------------------
// Formatters
// ---------------------------------------------------------------------------

export function formatCost(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  if (n >= 100) return n.toFixed(0);
  if (n >= 1) return n.toFixed(1);
  return n.toFixed(2);
}

export function formatTime(ms: number): string {
  if (ms >= 1000) return `${(ms / 1000).toFixed(2)} s`;
  if (ms >= 1) return `${ms.toFixed(2)} ms`;
  return `${(ms * 1000).toFixed(0)} us`;
}

export function formatRows(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return n.toFixed(0);
}

export function formatRatio(n: number): string {
  if (n >= 100) return `${n.toFixed(0)}x`;
  if (n >= 10) return `${n.toFixed(1)}x`;
  return `${n.toFixed(2)}x`;
}

// ---------------------------------------------------------------------------
// Tree traversal
// ---------------------------------------------------------------------------

export function getMaxCost(node: DbExplainNode): number {
  let max = node.totalCost ?? 0;
  for (const child of node.children) {
    const childMax = getMaxCost(child);
    if (childMax > max) max = childMax;
  }
  return max;
}

export function getMaxTime(node: DbExplainNode): number {
  let max = node.actualTimeMs ?? 0;
  for (const child of node.children) {
    const childMax = getMaxTime(child);
    if (childMax > max) max = childMax;
  }
  return max;
}

export function flattenExplainNodes(root: DbExplainNode): DbExplainNode[] {
  const nodes: DbExplainNode[] = [];
  function walk(node: DbExplainNode) {
    nodes.push(node);
    for (const child of node.children) walk(child);
  }
  walk(root);
  return nodes;
}

export function findExplainNode(
  root: DbExplainNode,
  nodeId: string | null,
): DbExplainNode | null {
  if (!nodeId) return null;
  if (root.id === nodeId) return root;
  for (const child of root.children) {
    const found = findExplainNode(child, nodeId);
    if (found) return found;
  }
  return null;
}

export function getRowEstimateRatio(node: DbExplainNode): number | null {
  if (
    node.planRows == null ||
    node.actualRows == null ||
    node.planRows <= 0 ||
    node.actualRows <= 0
  ) {
    return null;
  }
  return node.actualRows / node.planRows;
}

function getMismatchMagnitude(node: DbExplainNode): number | null {
  const ratio = getRowEstimateRatio(node);
  if (ratio == null) return null;
  return ratio >= 1 ? ratio : 1 / ratio;
}

function extraText(node: DbExplainNode): string {
  return Object.values(node.extra)
    .filter((value): value is string => typeof value === "string")
    .join(" ")
    .toLowerCase();
}

function isSequentialScan(node: DbExplainNode): boolean {
  const type = node.nodeType.toLowerCase();
  const accessType =
    typeof node.extra.accessType === "string"
      ? node.extra.accessType.toLowerCase()
      : typeof node.extra.access_type === "string"
        ? (node.extra.access_type as string).toLowerCase()
        : "";
  return (
    type.includes("seq scan") ||
    type.includes("table scan") ||
    type.includes("full scan") ||
    accessType === "all"
  );
}

function isTempOperation(node: DbExplainNode): boolean {
  const type = node.nodeType.toLowerCase();
  const extra = extraText(node);
  return (
    type.includes("sort") ||
    type.includes("filesort") ||
    type.includes("temporary") ||
    extra.includes("using temporary") ||
    extra.includes("using filesort")
  );
}

// ---------------------------------------------------------------------------
// Overview summary
// ---------------------------------------------------------------------------

export interface ExplainMetricNode {
  nodeId: string;
  nodeType: string;
  relation: string | null;
  value: number;
  ratio?: number;
}

export interface ExplainPlanSummary {
  highestCostNode: ExplainMetricNode | null;
  slowestNode: ExplainMetricNode | null;
  largestRowMismatchNode: ExplainMetricNode | null;
  sequentialScans: number;
  tempOperations: number;
}

export function getExplainPlanSummary(plan: DbExplainPlan): ExplainPlanSummary {
  const nodes = flattenExplainNodes(plan.root);

  let highestCostNode: ExplainMetricNode | null = null;
  let slowestNode: ExplainMetricNode | null = null;
  let largestRowMismatchNode: ExplainMetricNode | null = null;
  let sequentialScans = 0;
  let tempOperations = 0;

  for (const node of nodes) {
    if (
      node.totalCost != null &&
      (highestCostNode == null || node.totalCost > highestCostNode.value)
    ) {
      highestCostNode = {
        nodeId: node.id,
        nodeType: node.nodeType,
        relation: node.relation,
        value: node.totalCost,
      };
    }

    if (
      node.actualTimeMs != null &&
      (slowestNode == null || node.actualTimeMs > slowestNode.value)
    ) {
      slowestNode = {
        nodeId: node.id,
        nodeType: node.nodeType,
        relation: node.relation,
        value: node.actualTimeMs,
      };
    }

    const ratio = getRowEstimateRatio(node);
    const magnitude = getMismatchMagnitude(node);
    if (
      ratio != null &&
      magnitude != null &&
      (largestRowMismatchNode == null || magnitude > largestRowMismatchNode.value)
    ) {
      largestRowMismatchNode = {
        nodeId: node.id,
        nodeType: node.nodeType,
        relation: node.relation,
        value: magnitude,
        ratio,
      };
    }

    if (isSequentialScan(node)) sequentialScans += 1;
    if (isTempOperation(node)) tempOperations += 1;
  }

  return {
    highestCostNode,
    slowestNode,
    largestRowMismatchNode,
    sequentialScans,
    tempOperations,
  };
}

/**
 * One or two plain-English sentences describing what a single plan node does,
 * and where relevant the lever to make it cheaper. This is what actually makes
 * the graph "explain": SQLite plans carry no cost/row numbers, so without this
 * a selected node shows only its type + relation and reads as empty.
 */
export function explainNodeNarrative(node: DbExplainNode): string {
  const type = node.nodeType.toLowerCase();
  const rel = node.relation ? `\`${node.relation}\`` : "the input";
  const accessType =
    typeof node.extra.accessType === "string"
      ? node.extra.accessType.toLowerCase()
      : typeof node.extra.access_type === "string"
        ? (node.extra.access_type as string).toLowerCase()
        : "";
  const fullScan =
    accessType === "all" ||
    type.includes("seq scan") ||
    type.includes("table scan") ||
    type.includes("full scan");

  if (type === "query plan") {
    return "The whole query plan. Each step below is one operation the database runs; rows flow from the bottom (leaf) steps upward to produce the result.";
  }
  if (type.includes("co-routine")) {
    return "Runs a sub-pipeline that streams rows on demand into the step above it, instead of building them all up front.";
  }
  if (type.includes("materialize")) {
    return `Stashes ${rel} in a temporary result so it can be re-read cheaply — useful when the same rows are scanned more than once.`;
  }
  if (type.includes("subquer")) {
    if (type.includes("correlated")) {
      return "A correlated subquery — it re-runs once for every row of the outer query. On large outer results this is the usual hotspot; rewriting it as a JOIN often removes the repetition.";
    }
    return "A subquery whose result feeds the step above it.";
  }
  if (type.includes("compound")) {
    return "Combines the results of several sub-selects (a UNION / INTERSECT / EXCEPT).";
  }
  if (fullScan || (type.includes("scan") && !type.includes("index") && !node.indexCondition)) {
    return `Full scan of ${rel} — every row is read and tested. Fine on small tables, but the slowest path on large ones. An index on the columns you filter or join by lets the database jump straight to matching rows instead of reading them all.`;
  }
  if (type.includes("search") || type.includes("index") || node.indexCondition) {
    const idx = node.indexCondition ? ` (${node.indexCondition})` : "";
    return `Index lookup on ${rel}${idx} — rows are located through an index rather than by reading the whole table. This is the efficient access path.`;
  }
  if (type.includes("sort") || type.includes("temp b-tree") || type.includes("filesort")) {
    return "Sorts rows using a temporary structure. ORDER BY / GROUP BY / DISTINCT with no matching index forces this throwaway sort; an index already in the required order removes the step.";
  }
  if (type.includes("nested loop")) {
    return "Nested-loop join — for each row on one side it probes the other. Fast when one side is small or indexed; expensive when both sides are large.";
  }
  if (type.includes("merge join")) {
    return "Merge join — walks two already-sorted inputs in lockstep. Efficient when both sides arrive in the join-key order.";
  }
  if (type.includes("hash")) {
    return "Hash join — builds a hash table from one side and probes it with the other. Good for large, unsorted inputs joined on equality.";
  }
  if (type.includes("aggregate") || type.includes("group")) {
    return "Aggregates rows (COUNT / SUM / GROUP BY) into grouped output.";
  }
  if (type.includes("limit")) {
    return "Stops once enough rows exist to satisfy the LIMIT, so earlier steps can short-circuit instead of running to completion.";
  }
  return `Runs the "${node.nodeType}" step${node.relation ? ` on ${rel}` : ""} as part of the query plan.`;
}

/** Plain-English driver notes shown in the overview legend. */
export function getExplainDriverLegend(plan: DbExplainPlan): string[] {
  switch (plan.driver) {
    case "postgres":
      return plan.hasAnalyzeData
        ? [
            "Costs and timings are real measurements from EXPLAIN ANALYZE.",
            "Compare estimated vs actual rows to spot stale statistics.",
          ]
        : [
            "Costs are the planner's estimates in arbitrary cost units, not time.",
            "Run with ANALYZE to capture real row counts and timing.",
          ];
    case "mysql":
      return [
        "Rows are the optimizer's estimates; 'filtered' is the % kept after conditions.",
        "Watch for 'Using temporary' and 'Using filesort' in the extra details.",
      ];
    case "sqlite":
      return [
        "SQLite reports access strategy (SCAN/SEARCH) without cost or timing.",
        "SEARCH using an index is generally cheaper than a full table SCAN.",
      ];
    default:
      return [];
  }
}
