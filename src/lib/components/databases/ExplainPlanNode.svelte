<script lang="ts">
  /**
   * Custom @xyflow/svelte node for one step of a query plan. Ported from
   * tabularis `ExplainPlanNode.tsx`: a left cost-colour bar, header with node
   * type + relation, then estimated rows / cost / actual (ANALYZE) metrics and
   * any filter / index condition. Top/bottom handles connect the tree.
   */
  import { Handle, Position, type NodeProps } from "@xyflow/svelte";
  import type { DbExplainNode } from "$lib/types/databases";
  import {
    getNodeCostStyle,
    formatCost,
    formatRatio,
    formatRows,
    formatTime,
    getRowEstimateRatio,
  } from "./explainPlan";

  let { data }: NodeProps = $props();

  const node = $derived(data.node as DbExplainNode);
  const maxCost = $derived((data.maxCost as number) ?? 0);
  const hasAnalyzeData = $derived(Boolean(data.hasAnalyzeData));
  const isSelected = $derived(Boolean(data.isSelected));

  const costStyle = $derived(getNodeCostStyle(node.totalCost ?? 0, maxCost));
  const rowRatio = $derived(getRowEstimateRatio(node));
  const mismatch = $derived.by(() => {
    if (rowRatio == null || (rowRatio < 4 && rowRatio > 0.25)) return null;
    return rowRatio >= 1
      ? { value: formatRatio(rowRatio), label: "Planner over-estimated rows" }
      : { value: formatRatio(1 / rowRatio), label: "Planner under-estimated rows" };
  });
</script>

<div
  class="bg-surface border border-border rounded shadow-xl min-w-[260px] max-w-[300px] overflow-hidden
         border-l-4 {costStyle.border} {isSelected ? 'ring-2 ring-blue-400/70 border-blue-400/70' : ''}"
>
  <div class="px-3 py-2 border-b border-border {costStyle.headerBg}">
    <div class="text-sm font-bold text-fg">{node.nodeType}</div>
    {#if node.relation}
      <div class="text-xs text-fg-subtle mt-0.5">on {node.relation}</div>
    {/if}
  </div>

  <div class="px-3 py-2 space-y-1">
    <div class="flex items-center justify-between text-xs">
      <span class="text-fg-subtle">Est. rows</span>
      <span class="text-fg-muted font-mono">
        {node.planRows != null ? formatRows(node.planRows) : "-"}
      </span>
    </div>

    {#if node.totalCost != null}
      <div class="flex items-center justify-between text-xs">
        <span class="text-fg-subtle">Cost</span>
        <span class="text-fg-muted font-mono">{formatCost(node.totalCost)}</span>
      </div>
    {/if}

    {#if mismatch}
      <div class="flex items-center justify-between text-xs">
        <span class="text-fg-subtle">Estimate gap</span>
        <span class="text-amber-300 font-mono font-semibold">{mismatch.value}</span>
      </div>
    {/if}

    {#if hasAnalyzeData && node.actualRows != null}
      <div class="flex items-center justify-between text-xs">
        <span class="text-fg-subtle">Actual rows</span>
        <span class="text-fg font-mono font-semibold">{formatRows(node.actualRows)}</span>
      </div>
    {/if}

    {#if hasAnalyzeData && node.actualTimeMs != null}
      <div class="flex items-center justify-between text-xs">
        <span class="text-fg-subtle">Time</span>
        <span class="text-fg font-mono font-semibold">{formatTime(node.actualTimeMs)}</span>
      </div>
    {/if}

    {#if hasAnalyzeData && node.actualLoops != null && node.actualLoops > 1}
      <div class="flex items-center justify-between text-xs">
        <span class="text-fg-subtle">Loops</span>
        <span class="text-fg-muted font-mono">{node.actualLoops}</span>
      </div>
    {/if}

    {#if node.filter}
      <div class="text-[10px] text-fg-subtle mt-1 font-mono truncate border-t border-border/50 pt-1">
        Filter: {node.filter}
      </div>
    {/if}

    {#if node.indexCondition}
      <div class="text-[10px] text-fg-subtle font-mono truncate">
        Index: {node.indexCondition}
      </div>
    {/if}

    {#if mismatch}
      <div class="text-[10px] text-amber-300 font-mono truncate">{mismatch.label}</div>
    {/if}
  </div>

  <Handle type="target" position={Position.Top} class="!w-2 !h-2 !bg-indigo-500 !border-border" />
  <Handle type="source" position={Position.Bottom} class="!w-2 !h-2 !bg-indigo-500 !border-border" />
</div>
