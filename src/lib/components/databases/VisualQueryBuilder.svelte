<script lang="ts">
  /**
   * Visual query builder entry point. Wraps the canvas in <SvelteFlowProvider>
   * so the inner component can call useSvelteFlow() (fitView), mirroring the ERD
   * provider/content split.
   */
  import { SvelteFlowProvider } from "@xyflow/svelte";
  import VisualQueryBuilderInner from "./VisualQueryBuilderInner.svelte";
  import type { DbClientSchema } from "$lib/types/databases";

  interface Props {
    schema: DbClientSchema;
    onOpenQuery: (sql: string) => void;
  }

  let { schema, onOpenQuery }: Props = $props();
</script>

<SvelteFlowProvider>
  <VisualQueryBuilderInner {schema} {onOpenQuery} />
</SvelteFlowProvider>
