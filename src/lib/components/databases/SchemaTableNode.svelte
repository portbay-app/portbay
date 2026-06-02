<script lang="ts">
  /**
   * Custom @xyflow/svelte node for an ERD table. Ported from tabularis
   * `SchemaTableNode.tsx`: header with a glowing dot + name, then one row per
   * column with a PK (key) / FK (link) / plain (columns) icon, the column name,
   * and its type. Per-column source/target Handles let FK edges attach at the
   * exact column row; they stay invisible until the node is hovered.
   */
  import { Handle, Position, type NodeProps } from "@xyflow/svelte";
  import { Key, Link, Columns } from "@lucide/svelte";

  interface ErdColumn {
    name: string;
    type: string;
    isPk: boolean;
    isFk: boolean;
  }

  let { data }: NodeProps = $props();

  const label = $derived((data.label as string) ?? "");
  const columns = $derived((data.columns as ErdColumn[]) ?? []);
  const highlighted = $derived(Boolean(data.highlighted));

  let showHandles = $state(false);
</script>

<div
  role="presentation"
  class="bg-surface border rounded shadow-xl min-w-[220px] overflow-hidden cursor-pointer transition-colors
         {highlighted ? 'border-indigo-400 ring-2 ring-indigo-500/50' : 'border-border hover:border-indigo-500'}"
  onmouseenter={() => (showHandles = true)}
  onmouseleave={() => (showHandles = false)}
>
  <div
    class="bg-surface-2 px-3 py-2 text-sm font-bold text-fg border-b border-border flex items-center gap-2"
  >
    <div
      class="w-2 h-2 rounded-full bg-indigo-500 shrink-0"
      style="box-shadow: 0 0 8px rgba(99,102,241,0.6)"
    ></div>
    <span class="truncate">{label}</span>
  </div>
  <div class="flex flex-col">
    {#each columns as col (col.name)}
      <div
        class="flex items-center justify-between text-xs py-1.5 px-3 border-b border-border/50 last:border-0 relative
               {col.isPk ? 'bg-yellow-500/5 text-yellow-100' : 'text-fg-muted'}"
      >
        <div class="flex items-center gap-2 flex-1 min-w-0">
          {#if col.isPk}
            <Key size={10} class="text-yellow-500 shrink-0" />
          {:else if col.isFk}
            <Link size={10} class="text-purple-400 shrink-0" />
          {:else}
            <Columns size={10} class="text-fg-subtle shrink-0" />
          {/if}
          <span class="truncate font-mono flex-1 min-w-0 {col.isPk ? 'font-bold' : ''}">
            {col.name}
          </span>
        </div>
        <span class="text-[10px] text-fg-subtle ml-2 font-mono shrink-0">{col.type}</span>

        <Handle
          type="source"
          position={Position.Right}
          id={col.name}
          class={showHandles
            ? "!w-2 !h-2 !bg-indigo-500 !border-border !right-0"
            : "!w-1 !h-1 !bg-transparent !border-none !right-0"}
        />
        <Handle
          type="target"
          position={Position.Left}
          id={col.name}
          class={showHandles
            ? "!w-2 !h-2 !bg-indigo-500 !border-border !left-0"
            : "!w-1 !h-1 !bg-transparent !border-none !left-0"}
        />
      </div>
    {/each}
  </div>
</div>
