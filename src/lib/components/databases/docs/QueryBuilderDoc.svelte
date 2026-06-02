<script lang="ts">
  /**
   * QueryBuilderDoc — visual (drag/drop) query builder for an instance.
   * Loads the cached schema and hosts the canvas; "Open in query" hands the
   * generated SQL to a fresh SQL scratchpad tab.
   */
  import { onMount } from "svelte";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import VisualQueryBuilder from "$lib/components/databases/VisualQueryBuilder.svelte";

  import { dbWorkspace } from "$lib/stores/dbWorkspace.svelte";
  import type { DatabaseInstanceView } from "$lib/types/databases";

  interface Props {
    instance: DatabaseInstanceView;
  }

  let { instance }: Props = $props();

  const entry = $derived(dbWorkspace.schemaEntry(instance.id));
  const schema = $derived(entry.schema);
  const loading = $derived(entry.loading);
  const error = $derived(entry.error);

  onMount(() => {
    void dbWorkspace.loadSchema(instance.id);
  });

  function openInQuery(sql: string) {
    if (!sql.trim()) return;
    dbWorkspace.openQuery(instance.id, null, sql);
  }

  async function reload() {
    await dbWorkspace.loadSchema(instance.id, true);
  }
</script>

<div class="h-full flex flex-col min-h-0">
  <div
    class="shrink-0 px-4 py-2.5 border-b border-border/60 bg-surface/60
           flex items-center justify-between gap-3"
  >
    <div class="flex items-center gap-2">
      <Icon name="grid-2x2" size={13} class="text-fg-muted" />
      <span class="text-[13px] font-semibold text-fg">Query Builder</span>
    </div>
    <button
      type="button"
      onclick={reload}
      disabled={loading}
      title="Reload schema"
      aria-label="Reload schema"
      class="inline-flex items-center justify-center w-8 h-8 rounded-md
             border border-border bg-surface text-fg-muted hover:bg-surface-2
             hover:text-fg disabled:opacity-50 transition-colors"
    >
      <Icon name="refresh-cw" size={12} class={loading ? "animate-spin" : ""} />
    </button>
  </div>

  <div class="flex-1 min-h-0 overflow-hidden">
    {#if loading && !schema}
      <div class="h-full flex items-center justify-center">
        <div class="flex flex-col items-center gap-3 text-fg-subtle">
          <Icon name="refresh-cw" size={20} class="animate-spin" />
          <p class="text-[12px]">Inspecting schema…</p>
        </div>
      </div>
    {:else if error}
      <div class="h-full flex items-center justify-center px-8">
        <div class="text-center max-w-sm">
          <Icon name="circle-alert" size={22} class="text-status-crashed mx-auto mb-2" />
          <p class="text-[12px] text-status-crashed">{error}</p>
        </div>
      </div>
    {:else if schema}
      <VisualQueryBuilder {schema} onOpenQuery={openInQuery} />
    {:else}
      <div class="h-full flex items-center justify-center">
        <p class="text-[12px] text-fg-subtle">No schema loaded.</p>
      </div>
    {/if}
  </div>
</div>
