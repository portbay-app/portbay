<script lang="ts">
  /**
   * Custom @xyflow/svelte node for the visual query builder. One row per column
   * with a select checkbox, an aggregate dropdown, and source/target handles so
   * a JOIN edge can attach at the exact column. Callbacks are passed through
   * node `data` by the canvas, which owns the builder state.
   */
  import { Handle, Position, type NodeProps } from "@xyflow/svelte";
  import { Key, X } from "@lucide/svelte";
  import { AGGREGATES, type Aggregate, type BuilderColumn } from "./visualQuery";

  let { data }: NodeProps = $props();

  const name = $derived((data.name as string) ?? "");
  const columns = $derived((data.columns as BuilderColumn[]) ?? []);
  const pkSet = $derived(new Set((data.pkColumns as string[]) ?? []));

  const onToggle = $derived(data.onToggle as (col: string) => void);
  const onAggregate = $derived(data.onAggregate as (col: string, agg: Aggregate) => void);
  const onRemove = $derived(data.onRemove as () => void);
  const onSelectAll = $derived(data.onSelectAll as (selected: boolean) => void);

  const allSelected = $derived(columns.length > 0 && columns.every((c) => c.selected));
</script>

<div
  class="bg-surface border border-border rounded-lg shadow-xl min-w-[230px] overflow-hidden"
>
  <div
    class="bg-surface-2 px-3 py-2 border-b border-border flex items-center gap-2"
  >
    <span class="text-[12px] font-bold text-fg truncate flex-1">{name}</span>
    <button
      type="button"
      onclick={() => onSelectAll(!allSelected)}
      class="text-[10px] text-fg-subtle hover:text-fg px-1 rounded nodrag"
      title={allSelected ? "Deselect all" : "Select all"}
    >
      {allSelected ? "none" : "all"}
    </button>
    <button
      type="button"
      onclick={() => onRemove()}
      class="text-fg-subtle/60 hover:text-status-crashed nodrag"
      title="Remove table"
      aria-label="Remove table"
    >
      <X size={12} />
    </button>
  </div>

  <div class="flex flex-col">
    {#each columns as col (col.name)}
      <div
        class="flex items-center gap-2 text-[11px] py-1 px-2.5 border-b border-border/40
               last:border-0 relative {col.selected ? 'bg-accent/5' : ''}"
      >
        <input
          type="checkbox"
          checked={col.selected}
          onchange={() => onToggle(col.name)}
          class="accent-accent shrink-0 nodrag"
          aria-label="Select {col.name}"
        />
        {#if pkSet.has(col.name)}
          <Key size={9} class="text-yellow-500 shrink-0" />
        {/if}
        <span class="truncate font-mono flex-1 min-w-0 text-fg-muted">{col.name}</span>

        <select
          value={col.aggregate}
          onchange={(e) => onAggregate(col.name, e.currentTarget.value as Aggregate)}
          class="h-5 rounded border border-border bg-surface text-[9.5px] text-fg-subtle
                 px-0.5 nodrag shrink-0"
          aria-label="Aggregate for {col.name}"
          title="Aggregate"
        >
          {#each AGGREGATES as agg (agg.value)}
            <option value={agg.value}>{agg.label}</option>
          {/each}
        </select>

        <Handle
          type="source"
          position={Position.Right}
          id={col.name}
          class="!w-2 !h-2 !bg-accent !border-border !right-0"
        />
        <Handle
          type="target"
          position={Position.Left}
          id={col.name}
          class="!w-2 !h-2 !bg-accent !border-border !left-0"
        />
      </div>
    {/each}
  </div>
</div>
