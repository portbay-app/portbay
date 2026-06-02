<script lang="ts">
  /**
   * ERD entry point. Wraps the canvas in <SvelteFlowProvider> so the inner
   * component can use useSvelteFlow() (fitView/zoom) — mirroring tabularis'
   * <ReactFlowProvider> + SchemaDiagramContent split. The workbench imports this
   * component unchanged.
   */
  import { SvelteFlowProvider } from "@xyflow/svelte";
  import type { DbClientSchema, DbClientTable } from "$lib/types/databases";
  import SchemaDiagramContent from "./SchemaDiagramContent.svelte";

  interface Props {
    schema: DbClientSchema;
    selectedKey?: string | null;
    onSelect?: (table: DbClientTable) => void;
  }

  let { schema, selectedKey = null, onSelect }: Props = $props();
</script>

{#if schema.tables.length === 0}
  <div
    class="flex h-full min-h-[320px] items-center justify-center rounded-lg border border-border/60
           bg-surface-2/30 px-4 text-center text-[12px] text-fg-subtle"
  >
    No tables to diagram.
  </div>
{:else}
  <div class="h-full min-h-[340px] overflow-hidden rounded-lg border border-border/60">
    <SvelteFlowProvider>
      <SchemaDiagramContent {schema} {selectedKey} {onSelect} />
    </SvelteFlowProvider>
  </div>
{/if}
