<script lang="ts">
  /**
   * Collapsible "Overview" strip above the plan: the highest-cost step, slowest
   * step, largest row-estimate gap, sequential-scan and temp-operation counts —
   * each a button that selects the relevant node — plus driver notes. Ported
   * from tabularis `ExplainOverviewBar.tsx`.
   */
  import {
    AlertTriangle,
    BookOpenText,
    ChevronDown,
    ChevronRight,
    Clock3,
    Database,
    LayoutDashboard,
    Layers2,
    ScanSearch,
  } from "@lucide/svelte";
  import type { DbExplainPlan } from "$lib/types/databases";
  import {
    formatCost,
    formatRatio,
    formatTime,
    getExplainDriverLegend,
    getExplainPlanSummary,
  } from "./explainPlan";

  interface Props {
    plan: DbExplainPlan;
    onSelectNode: (id: string) => void;
  }

  let { plan, onSelectNode }: Props = $props();

  type Tone = "blue" | "amber" | "red" | "purple";
  interface Finding {
    key: string;
    label: string;
    value: string;
    description: string;
    nodeId: string;
    icon: typeof Layers2;
    tone: Tone;
  }

  const summary = $derived(getExplainPlanSummary(plan));
  const legend = $derived(getExplainDriverLegend(plan));
  let expandedView = $state(true);

  function nodeLabel(nodeType: string, relation: string | null): string {
    return relation ? `${nodeType} · ${relation}` : nodeType;
  }

  const findings = $derived.by<Finding[]>(() => {
    const out: Finding[] = [];
    if (summary.highestCostNode) {
      out.push({
        key: "highest-cost",
        label: "Highest cost",
        value: formatCost(summary.highestCostNode.value),
        description: nodeLabel(summary.highestCostNode.nodeType, summary.highestCostNode.relation),
        nodeId: summary.highestCostNode.nodeId,
        icon: Layers2,
        tone: "blue",
      });
    }
    if (summary.slowestNode) {
      out.push({
        key: "slowest-step",
        label: "Slowest step",
        value: formatTime(summary.slowestNode.value),
        description: nodeLabel(summary.slowestNode.nodeType, summary.slowestNode.relation),
        nodeId: summary.slowestNode.nodeId,
        icon: Clock3,
        tone: "amber",
      });
    }
    if (summary.largestRowMismatchNode?.ratio != null) {
      out.push({
        key: "estimate-gap",
        label: "Largest estimate gap",
        value: formatRatio(summary.largestRowMismatchNode.value),
        description:
          summary.largestRowMismatchNode.ratio >= 1 ? "Over-estimated rows" : "Under-estimated rows",
        nodeId: summary.largestRowMismatchNode.nodeId,
        icon: AlertTriangle,
        tone: "red",
      });
    }
    if (summary.sequentialScans > 0) {
      out.push({
        key: "sequential-scans",
        label: "Sequential scans",
        value: String(summary.sequentialScans),
        description: "Full-scan operations",
        nodeId: summary.highestCostNode?.nodeId ?? plan.root.id,
        icon: ScanSearch,
        tone: "amber",
      });
    }
    if (summary.tempOperations > 0) {
      out.push({
        key: "temp-operations",
        label: "Temp operations",
        value: String(summary.tempOperations),
        description: "Sort or temporary operations",
        nodeId: summary.slowestNode?.nodeId ?? plan.root.id,
        icon: Database,
        tone: "purple",
      });
    }
    return out;
  });

  const toneClass: Record<Tone, string> = {
    blue: "border-blue-500/30 bg-blue-950/20 text-blue-200 hover:bg-blue-950/30",
    amber: "border-amber-500/30 bg-amber-950/20 text-amber-200 hover:bg-amber-950/30",
    red: "border-red-500/30 bg-red-950/20 text-red-200 hover:bg-red-950/30",
    purple: "border-fuchsia-500/30 bg-fuchsia-950/20 text-fuchsia-200 hover:bg-fuchsia-950/30",
  };
</script>

<div class="border-b border-border bg-surface-2/20 px-4 py-2">
  <div class="rounded-xl border border-border bg-surface-2/30 overflow-hidden">
    <button
      type="button"
      onclick={() => (expandedView = !expandedView)}
      class="w-full flex items-center gap-3 px-3 py-2.5 hover:bg-surface-2/50 transition-colors"
    >
      <div class="p-1.5 rounded-md bg-blue-900/25 text-blue-300">
        <LayoutDashboard size={13} />
      </div>
      <div class="min-w-0 text-left">
        <div class="text-[11px] uppercase tracking-[0.14em] text-fg-subtle font-semibold">Overview</div>
        <div class="text-xs text-fg-muted">
          {findings.length} top issues{legend.length > 0 ? ` • ${legend.length} driver notes` : ""}
        </div>
      </div>
      <div class="ml-auto flex items-center gap-2 text-[11px] text-fg-subtle">
        <span>{expandedView ? "Hide" : "Show"}</span>
        {#if expandedView}
          <ChevronDown size={14} />
        {:else}
          <ChevronRight size={14} />
        {/if}
      </div>
    </button>

    {#if expandedView}
      <div class="px-3 pb-3 space-y-3">
        <div class="flex flex-wrap gap-2">
          {#if findings.length === 0}
            <div class="text-xs text-fg-muted">No obvious issues found in this plan.</div>
          {:else}
            {#each findings as finding (finding.key)}
              {@const Icon = finding.icon}
              <button
                type="button"
                onclick={() => onSelectNode(finding.nodeId)}
                class="min-w-[170px] flex items-start gap-2 rounded-xl border px-3 py-2 text-left transition-colors {toneClass[
                  finding.tone
                ]}"
              >
                <Icon size={14} class="mt-0.5 shrink-0" />
                <div class="min-w-0">
                  <div class="text-[10px] uppercase tracking-[0.14em] opacity-80">{finding.label}</div>
                  <div class="text-sm font-semibold leading-tight mt-1">{finding.value}</div>
                  <div class="text-[11px] opacity-80 mt-1">{finding.description}</div>
                </div>
              </button>
            {/each}
          {/if}
        </div>

        {#if legend.length > 0}
          <div class="rounded-xl border border-border bg-surface-2/30 px-3 py-2.5">
            <div class="flex items-center gap-2 mb-2">
              <div class="p-1.5 rounded-md bg-cyan-900/30 text-cyan-300">
                <BookOpenText size={13} />
              </div>
              <span class="text-[11px] uppercase tracking-[0.14em] text-fg-subtle font-semibold">
                Driver notes
              </span>
            </div>
            <div class="flex flex-wrap gap-x-4 gap-y-1.5">
              {#each legend as entry (entry)}
                <div class="flex items-start gap-2 text-xs text-fg-muted leading-relaxed max-w-[720px]">
                  <span class="mt-[6px] h-1.5 w-1.5 shrink-0 rounded-full bg-cyan-400/80"></span>
                  <span>{entry}</span>
                </div>
              {/each}
            </div>
          </div>
        {/if}
      </div>
    {/if}
  </div>
</div>
