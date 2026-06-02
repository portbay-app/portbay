<script lang="ts">
  /**
   * Tabular plan view: a collapsible tree table on the left + node details on
   * the right. Ported from tabularis `ExplainTableView.tsx`. The recursive rows
   * are rendered with a self-referencing Svelte snippet.
   */
  import { ChevronRight, ChevronDown } from "@lucide/svelte";
  import type { DbExplainNode, DbExplainPlan } from "$lib/types/databases";
  import {
    findExplainNode,
    formatCost,
    formatRatio,
    formatRows,
    formatTime,
    getRowEstimateRatio,
  } from "./explainPlan";
  import ExplainNodeDetails from "./ExplainNodeDetails.svelte";

  interface Props {
    plan: DbExplainPlan;
    selectedId: string | null;
    onSelect: (id: string) => void;
  }

  let { plan, selectedId, onSelect }: Props = $props();

  function collectIds(root: DbExplainNode): Set<string> {
    const ids = new Set<string>();
    const walk = (node: DbExplainNode) => {
      ids.add(node.id);
      node.children.forEach(walk);
    };
    walk(root);
    return ids;
  }

  let expanded = $state<Set<string>>(new Set());

  // Expand everything on mount and whenever the plan changes.
  $effect(() => {
    expanded = collectIds(plan.root);
  });

  function toggle(id: string) {
    const next = new Set(expanded);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    expanded = next;
  }

  function costStr(node: DbExplainNode): string {
    if (node.startupCost != null && node.totalCost != null) {
      return `${formatCost(node.startupCost)} - ${formatCost(node.totalCost)}`;
    }
    return node.totalCost != null ? formatCost(node.totalCost) : "-";
  }

  function ratioStr(node: DbExplainNode): string {
    const ratio = getRowEstimateRatio(node);
    if (ratio == null) return "-";
    return ratio >= 1 ? formatRatio(ratio) : formatRatio(1 / ratio);
  }

  const selectedNode = $derived(findExplainNode(plan.root, selectedId));
</script>

<div class="flex h-full">
  <div class="flex-1 overflow-auto border-r border-border min-w-0">
    <table class="w-full text-xs">
      <thead class="sticky top-0 z-10 bg-surface border-b border-border">
        <tr>
          <th class="text-left px-3 py-2 text-fg-subtle font-semibold whitespace-nowrap">Node type</th>
          <th class="text-left px-3 py-2 text-fg-subtle font-semibold whitespace-nowrap">Relation</th>
          <th class="text-right px-3 py-2 text-fg-subtle font-semibold whitespace-nowrap">Cost</th>
          <th class="text-right px-3 py-2 text-fg-subtle font-semibold whitespace-nowrap">Est. rows</th>
          <th class="text-right px-3 py-2 text-fg-subtle font-semibold whitespace-nowrap">Time</th>
          <th class="text-right px-3 py-2 text-fg-subtle font-semibold whitespace-nowrap">Estimate gap</th>
          <th class="text-left px-3 py-2 text-fg-subtle font-semibold whitespace-nowrap">Filter</th>
        </tr>
      </thead>
      <tbody>
        {@render rows(plan.root, 0)}
      </tbody>
    </table>
  </div>

  <div class="w-[320px] shrink-0 overflow-y-auto bg-surface-2/30">
    <ExplainNodeDetails node={selectedNode} hasAnalyzeData={plan.hasAnalyzeData} />
  </div>
</div>

{#snippet rows(node: DbExplainNode, depth: number)}
  {@const ratio = getRowEstimateRatio(node)}
  {@const isSelected = selectedId === node.id}
  <tr
    class="cursor-pointer transition-colors border-b border-border/30 {isSelected
      ? 'bg-blue-900/30'
      : 'hover:bg-surface-2/60'}"
    onclick={() => onSelect(node.id)}
  >
    <td class="px-3 py-1.5 whitespace-nowrap">
      <div class="flex items-center gap-1" style="padding-left: {depth * 20}px">
        {#if node.children.length > 0}
          <button
            type="button"
            onclick={(event) => {
              event.stopPropagation();
              toggle(node.id);
            }}
            class="p-0.5 text-fg-subtle hover:text-fg"
            aria-label={expanded.has(node.id) ? "Collapse" : "Expand"}
          >
            {#if expanded.has(node.id)}
              <ChevronDown size={12} />
            {:else}
              <ChevronRight size={12} />
            {/if}
          </button>
        {:else}
          <span class="w-4"></span>
        {/if}
        <span class="text-fg font-medium">{node.nodeType}</span>
      </div>
    </td>
    <td class="px-3 py-1.5 text-fg-muted whitespace-nowrap">{node.relation ?? ""}</td>
    <td class="px-3 py-1.5 text-right text-fg-muted font-mono whitespace-nowrap">{costStr(node)}</td>
    <td class="px-3 py-1.5 text-right text-fg-muted font-mono whitespace-nowrap">
      {node.planRows != null ? formatRows(node.planRows) : "-"}
    </td>
    <td class="px-3 py-1.5 text-right text-fg-muted font-mono whitespace-nowrap">
      {plan.hasAnalyzeData && node.actualTimeMs != null ? formatTime(node.actualTimeMs) : "-"}
    </td>
    <td class="px-3 py-1.5 text-right whitespace-nowrap">
      <span
        class="font-mono {ratio != null && (ratio >= 4 || ratio <= 0.25)
          ? 'text-amber-300'
          : 'text-fg-muted'}"
      >
        {ratioStr(node)}
      </span>
    </td>
    <td class="px-3 py-1.5 text-fg-subtle truncate max-w-[200px]">{node.filter ?? ""}</td>
  </tr>
  {#if expanded.has(node.id)}
    {#each node.children as child (child.id)}
      {@render rows(child, depth + 1)}
    {/each}
  {/if}
{/snippet}
