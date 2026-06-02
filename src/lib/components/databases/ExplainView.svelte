<script lang="ts">
  /**
   * Visual EXPLAIN presentation. Ported from tabularis `VisualExplainView.tsx`:
   * a summary bar (timings + view switcher), an overview strip of findings, and
   * a body that is the plan graph (with a node-details side panel), a tree
   * table, or the raw EXPLAIN output. The AI tab from the reference is omitted.
   */
  import { Loader2 } from "@lucide/svelte";
  import type { DbExplainPlan } from "$lib/types/databases";
  import { findExplainNode, type ExplainViewMode } from "./explainPlan";
  import ExplainSummaryBar from "./ExplainSummaryBar.svelte";
  import ExplainOverviewBar from "./ExplainOverviewBar.svelte";
  import ExplainGraph from "./ExplainGraph.svelte";
  import ExplainTableView from "./ExplainTableView.svelte";
  import ExplainNodeDetails from "./ExplainNodeDetails.svelte";

  interface Props {
    plan: DbExplainPlan | null;
    isLoading?: boolean;
    error?: string | null;
  }

  let { plan, isLoading = false, error = null }: Props = $props();

  let viewMode = $state<ExplainViewMode>("graph");
  let selectedNodeId = $state<string | null>(null);

  // Reset the selection whenever a new plan arrives.
  $effect(() => {
    void plan;
    selectedNodeId = null;
  });

  const selectedNode = $derived(plan ? findExplainNode(plan.root, selectedNodeId) : null);

  const rawText = $derived.by(() => {
    if (!plan) return "";
    if (plan.rawOutput && plan.rawOutput.trim()) return plan.rawOutput;
    return JSON.stringify(plan.root, null, 2);
  });
</script>

<div class="flex flex-col h-full min-h-0">
  <ExplainSummaryBar {plan} {viewMode} onViewModeChange={(mode) => (viewMode = mode)} />
  {#if plan}
    <ExplainOverviewBar {plan} onSelectNode={(id) => (selectedNodeId = id)} />
  {/if}

  <div class="flex-1 overflow-hidden min-h-0">
    {#if isLoading}
      <div class="flex flex-col items-center justify-center h-full gap-2 text-fg-subtle">
        <Loader2 size={24} class="animate-spin" />
        <span class="text-sm">Building the query plan…</span>
      </div>
    {:else if error}
      <div class="flex flex-col items-center justify-center h-full gap-2 px-8">
        <div class="text-status-crashed text-sm text-center max-w-lg">{error}</div>
      </div>
    {:else if !plan}
      <div class="flex flex-col items-center justify-center h-full gap-1 px-8 text-center">
        <div class="text-sm text-fg-muted">Run Visual Explain to see the query plan.</div>
        <div class="text-xs text-fg-subtle">
          PortBay runs EXPLAIN read-only — it never executes your query's writes.
        </div>
      </div>
    {:else if viewMode === "raw"}
      <pre
        class="h-full w-full overflow-auto bg-surface-2/20 p-4 font-mono text-[12px] leading-relaxed text-fg-muted whitespace-pre">{rawText}</pre>
    {:else if viewMode === "table"}
      <ExplainTableView {plan} selectedId={selectedNodeId} onSelect={(id) => (selectedNodeId = id)} />
    {:else}
      <div class="flex h-full">
        <div class="flex-1 min-w-0 border-r border-border">
          <ExplainGraph {plan} {selectedNodeId} onSelectNode={(id) => (selectedNodeId = id)} />
        </div>
        <div class="w-[320px] shrink-0 overflow-y-auto bg-surface-2/20">
          <ExplainNodeDetails node={selectedNode} hasAnalyzeData={plan.hasAnalyzeData} />
        </div>
      </div>
    {/if}
  </div>
</div>
