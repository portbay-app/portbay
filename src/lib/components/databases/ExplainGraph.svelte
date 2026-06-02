<script lang="ts">
  /** Provider wrapper for the explain graph (lets the inner canvas use fitView). */
  import { SvelteFlowProvider } from "@xyflow/svelte";
  import type { DbExplainPlan } from "$lib/types/databases";
  import ExplainGraphInner from "./ExplainGraphInner.svelte";

  interface Props {
    plan: DbExplainPlan;
    selectedNodeId: string | null;
    onSelectNode: (id: string) => void;
  }

  let { plan, selectedNodeId, onSelectNode }: Props = $props();
</script>

<div class="h-full w-full">
  <SvelteFlowProvider>
    <ExplainGraphInner {plan} {selectedNodeId} {onSelectNode} />
  </SvelteFlowProvider>
</div>
