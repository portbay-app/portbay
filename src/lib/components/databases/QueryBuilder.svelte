<script lang="ts">
  import Icon from "$lib/components/atoms/Icon.svelte";
  import type { DbClientTable } from "$lib/types/databases";

  interface Props {
    tables: DbClientTable[];
    selected: DbClientTable | null;
    onBuild?: (sql: string, table: DbClientTable) => void;
  }

  let { tables, selected, onBuild }: Props = $props();

  let tableKey = $state<string>("");
  let selectedColumns = $state<Set<string>>(new Set());
  let joinKey = $state<string>("");
  let whereText = $state<string>("");

  function key(table: DbClientTable): string {
    return `${table.schema ?? ""}.${table.name}`;
  }

  function label(table: DbClientTable): string {
    return table.schema ? `${table.schema}.${table.name}` : table.name;
  }

  const table = $derived(
    tables.find((item) => key(item) === tableKey) ?? selected ?? tables[0] ?? null,
  );

  const joins = $derived(
    table?.foreignKeys
      .map((fk) => {
        const target = tables.find(
          (item) =>
            item.name === fk.refTable ||
            (table.schema && key(item) === `${table.schema}.${fk.refTable}`),
        );
        return target ? { fk, target } : null;
      })
      .filter((item) => item !== null) ?? [],
  );

  function toggleColumn(name: string) {
    const next = new Set(selectedColumns);
    if (next.has(name)) next.delete(name);
    else next.add(name);
    selectedColumns = next;
  }

  function build() {
    if (!table) return;
    const cols = table.columns
      .filter((column) => selectedColumns.size === 0 || selectedColumns.has(column.name))
      .map((column) => `${table.name}.${column.name}`)
      .join(", ");
    const join = joins.find((item) => `${item.fk.column}:${item.target.name}` === joinKey);
    const fromRef = label(table);
    const joinSql = join
      ? `\nJOIN ${label(join.target)} ON ${table.name}.${join.fk.column} = ${join.target.name}.${join.fk.refColumn}`
      : "";
    const whereSql = whereText.trim() ? `\nWHERE ${whereText.trim()}` : "";
    onBuild?.(`SELECT ${cols || "*"}\nFROM ${fromRef}${joinSql}${whereSql}\nLIMIT 100`, table);
  }

  $effect(() => {
    if (!tableKey && selected) tableKey = key(selected);
  });
</script>

<div class="rounded-lg border border-border/60 bg-surface-2/30 p-3 space-y-3">
  <div class="grid grid-cols-1 md:grid-cols-[220px,1fr] gap-3">
    <label class="block">
      <span class="block mb-1 text-[11px] text-fg-muted">Table</span>
      <select
        bind:value={tableKey}
        class="w-full h-8 rounded-md border border-border bg-surface px-2 text-[12px] text-fg"
      >
        {#each tables as item (key(item))}
          <option value={key(item)}>{label(item)}</option>
        {/each}
      </select>
    </label>

    <label class="block">
      <span class="block mb-1 text-[11px] text-fg-muted">Filter</span>
      <input
        bind:value={whereText}
        placeholder="status = 'active'"
        class="w-full h-8 rounded-md border border-border bg-surface px-2 font-mono
               text-[12px] text-fg placeholder:text-fg-subtle"
      />
    </label>
  </div>

  {#if table}
    <div>
      <div class="mb-1 text-[11px] text-fg-muted">Columns</div>
      <div class="flex flex-wrap gap-1.5">
        {#each table.columns as column (column.name)}
          <button
            type="button"
            onclick={() => toggleColumn(column.name)}
            class="inline-flex items-center gap-1 rounded-md border px-2 py-1 text-[11px]
                   {selectedColumns.size === 0 || selectedColumns.has(column.name)
              ? 'border-accent/50 bg-accent/10 text-accent'
              : 'border-border text-fg-muted hover:bg-surface'}"
          >
            {#if column.primaryKey}
              <Icon name="lock" size={10} />
            {/if}
            {column.name}
          </button>
        {/each}
      </div>
    </div>

    {#if joins.length > 0}
      <label class="block">
        <span class="block mb-1 text-[11px] text-fg-muted">Join</span>
        <select
          bind:value={joinKey}
          class="w-full h-8 rounded-md border border-border bg-surface px-2 text-[12px] text-fg"
        >
          <option value="">No join</option>
          {#each joins as join (`${join.fk.column}:${join.target.name}`)}
            <option value={`${join.fk.column}:${join.target.name}`}>
              {join.fk.column} -> {join.target.name}.{join.fk.refColumn}
            </option>
          {/each}
        </select>
      </label>
    {/if}

    <button
      type="button"
      onclick={build}
      class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md bg-accent text-on-accent
             text-[12px] font-medium hover:brightness-110"
    >
      <Icon name="sparkles" size={12} />
      Build query
    </button>
  {/if}
</div>
