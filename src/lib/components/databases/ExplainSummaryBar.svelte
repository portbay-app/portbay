<script lang="ts">
  /**
   * Top bar for the explain view: planning / execution time + total cost on the
   * left, and a graph / table / raw view switcher on the right. Ported from
   * tabularis `ExplainSummaryBar.tsx` (the AI tab is intentionally omitted).
   */
  import { Network, TableProperties, FileText } from "@lucide/svelte";
  import type { DbExplainPlan } from "$lib/types/databases";
  import { formatTime, formatCost, getMaxCost, type ExplainViewMode } from "./explainPlan";

  interface Props {
    plan: DbExplainPlan | null;
    viewMode: ExplainViewMode;
    onViewModeChange: (mode: ExplainViewMode) => void;
  }

  let { plan, viewMode, onViewModeChange }: Props = $props();

  const maxCost = $derived(plan ? getMaxCost(plan.root) : 0);

  const modes: Array<{ id: ExplainViewMode; label: string; icon: typeof Network }> = [
    { id: "graph", label: "Graph", icon: Network },
    { id: "table", label: "Table", icon: TableProperties },
    { id: "raw", label: "Raw", icon: FileText },
  ];
</script>

{#if plan}
  <div class="flex items-center gap-4 px-4 py-2 border-b border-border bg-surface-2/30 text-xs">
    {#if plan.planningTimeMs != null}
      <div class="flex items-center gap-1.5">
        <span class="text-fg-subtle">Planning:</span>
        <span class="text-fg font-mono font-semibold">{formatTime(plan.planningTimeMs)}</span>
      </div>
    {/if}
    {#if plan.executionTimeMs != null}
      <div class="flex items-center gap-1.5">
        <span class="text-fg-subtle">Execution:</span>
        <span class="text-fg font-mono font-semibold">{formatTime(plan.executionTimeMs)}</span>
      </div>
    {/if}
    {#if maxCost > 0}
      <div class="flex items-center gap-1.5">
        <span class="text-fg-subtle">Total cost:</span>
        <span class="text-fg font-mono font-semibold">{formatCost(maxCost)}</span>
      </div>
    {/if}

    <div class="flex-1"></div>

    <div class="flex items-center gap-0.5 rounded-lg bg-surface-2 p-0.5">
      {#each modes as mode (mode.id)}
        {@const Icon = mode.icon}
        <button
          type="button"
          onclick={() => onViewModeChange(mode.id)}
          class="flex items-center gap-1.5 px-2 py-1 rounded text-xs transition-colors {viewMode === mode.id
            ? 'bg-blue-900/40 text-blue-300'
            : 'text-fg-subtle hover:text-fg'}"
        >
          <Icon size={12} />
          {mode.label}
        </button>
      {/each}
    </div>
  </div>
{/if}
