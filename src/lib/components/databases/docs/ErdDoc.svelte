<script lang="ts">
  /**
   * ErdDoc — Schema diagram (ERD) for a database instance.
   * Loads schema via dbWorkspace.loadSchema (cached/shared).
   * Clicking a table opens a TableDoc via dbWorkspace.openTable.
   */
  import { onMount } from "svelte";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import SchemaDiagram from "$lib/components/databases/SchemaDiagram.svelte";

  import { dbWorkspace } from "$lib/stores/dbWorkspace.svelte";
  import type { DatabaseInstanceView, DbClientSchema, DbClientTable } from "$lib/types/databases";

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

  // Reload if the instance id changes while this doc is mounted.
  $effect(() => {
    void instance.id;
    void dbWorkspace.loadSchema(instance.id);
  });

  function onTableSelect(table: DbClientTable) {
    dbWorkspace.openTable(instance.id, table.schema ?? null, table.name);
  }

  async function reload() {
    await dbWorkspace.loadSchema(instance.id, true);
  }
</script>

<div class="h-full flex flex-col min-h-0">
  <!-- Toolbar -->
  <div
    class="shrink-0 px-4 py-2.5 border-b border-border/60 bg-surface/60
           flex items-center justify-between gap-3"
  >
    <div class="flex items-center gap-2">
      <Icon name="share" size={13} class="text-fg-muted" />
      <span class="text-[13px] font-semibold text-fg">Schema Diagram</span>
      {#if schema}
        <span class="text-[11px] text-fg-subtle">
          {schema.tables.length} table{schema.tables.length === 1 ? "" : "s"}
        </span>
      {/if}
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

  <!-- Body -->
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
          <button
            type="button"
            onclick={reload}
            class="mt-3 inline-flex items-center gap-1.5 h-8 px-3 rounded-md
                   border border-border bg-surface text-[12px] text-fg-muted
                   hover:bg-surface-2 hover:text-fg transition-colors"
          >
            <Icon name="refresh-cw" size={11} />
            Retry
          </button>
        </div>
      </div>
    {:else if schema}
      <div class="h-full p-2">
        <SchemaDiagram {schema} onSelect={onTableSelect} />
      </div>
    {:else}
      <div class="h-full flex items-center justify-center">
        <p class="text-[12px] text-fg-subtle">No schema loaded.</p>
      </div>
    {/if}
  </div>
</div>
