<script lang="ts">
  /**
   * Right-hand detail panel for the selected plan node. Ported from tabularis
   * `ExplainNodeDetails.tsx`: General / Analyze data / Extra detail sections.
   */
  import type { DbExplainNode } from "$lib/types/databases";
  import { explainNodeNarrative, formatCost, formatRows, formatTime } from "./explainPlan";

  interface Props {
    node: DbExplainNode | null;
    hasAnalyzeData: boolean;
  }

  let { node, hasAnalyzeData }: Props = $props();

  type Entry = [string, string];

  const general = $derived.by<Entry[]>(() => {
    if (!node) return [];
    const out: Entry[] = [["Node type", node.nodeType]];
    if (node.relation) out.push(["Relation", node.relation]);
    if (node.startupCost != null && node.totalCost != null) {
      out.push(["Cost", `${formatCost(node.startupCost)} - ${formatCost(node.totalCost)}`]);
    } else if (node.totalCost != null) {
      out.push(["Cost", formatCost(node.totalCost)]);
    }
    if (node.planRows != null) out.push(["Est. rows", formatRows(node.planRows)]);
    if (node.filter) out.push(["Filter", node.filter]);
    if (node.indexCondition) out.push(["Index condition", node.indexCondition]);
    if (node.joinType) out.push(["Join type", node.joinType]);
    if (node.hashCondition) out.push(["Hash condition", node.hashCondition]);
    return out;
  });

  const analyze = $derived.by<Entry[]>(() => {
    if (!node || !hasAnalyzeData) return [];
    const out: Entry[] = [];
    if (node.actualRows != null) out.push(["Actual rows", formatRows(node.actualRows)]);
    if (node.actualTimeMs != null) out.push(["Time", formatTime(node.actualTimeMs)]);
    if (node.actualLoops != null) out.push(["Loops", String(node.actualLoops)]);
    if (node.buffersHit != null) out.push(["Buffers hit", String(node.buffersHit)]);
    if (node.buffersRead != null) out.push(["Buffers read", String(node.buffersRead)]);
    return out;
  });

  const extra = $derived.by<Entry[]>(() => {
    if (!node) return [];
    return Object.entries(node.extra).map(([key, value]) => [
      key,
      typeof value === "string" ? value : JSON.stringify(value),
    ]);
  });

  const narrative = $derived(node ? explainNodeNarrative(node) : "");
</script>

{#snippet section(title: string, entries: Entry[])}
  {#if entries.length > 0}
    <div class="border-b border-border/60 last:border-b-0">
      <div class="px-4 py-3 text-[11px] uppercase tracking-wide text-fg-subtle font-semibold bg-surface-2/50">
        {title}
      </div>
      <div class="divide-y divide-border/40">
        {#each entries as [label, value] (label)}
          <div class="px-4 py-2.5">
            <div class="text-[11px] text-fg-subtle mb-1">{label}</div>
            <div class="text-fg-muted break-words font-mono leading-relaxed text-xs">{value}</div>
          </div>
        {/each}
      </div>
    </div>
  {/if}
{/snippet}

{#if !node}
  <div class="p-4 text-xs text-fg-subtle">Select a node to see its details.</div>
{:else}
  <div class="text-xs">
    <div class="border-b border-border/60 px-4 py-3">
      <div class="text-[11px] uppercase tracking-wide text-fg-subtle font-semibold mb-1.5">
        What this step does
      </div>
      <p class="text-[12px] text-fg-muted leading-relaxed">{narrative}</p>
    </div>
    {@render section("General", general)}
    {@render section("Analyze data", analyze)}
    {@render section("Extra details", extra)}
  </div>
{/if}
