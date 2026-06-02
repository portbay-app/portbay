<script lang="ts">
  /**
   * In-section navigator for the Databases workspace — the page's own left
   * column (NOT the global app sidebar). Lists the user's database instances;
   * each expands into its table tree plus Schema-diagram / New-query shortcuts.
   * Everything drives the {@link dbWorkspace} tab store: clicking an instance
   * opens its Overview, a table opens a Table tab, etc.
   */
  import Icon from "$lib/components/atoms/Icon.svelte";
  import StatusDot from "$lib/components/atoms/StatusDot.svelte";
  import DatabaseMark from "$lib/components/databases/DatabaseMark.svelte";
  import { databases } from "$lib/stores/databases.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import { dbWorkspace } from "$lib/stores/dbWorkspace.svelte";
  import { statusLabel } from "$lib/types/databases";
  import type {
    DatabaseInstanceView,
    DbClientTable,
    InstanceStatus,
  } from "$lib/types/databases";

  /** Engines the embedded client can inspect (others only get an Overview). */
  const DB_CLIENT_ENGINES = new Set(["mysql", "mariadb", "postgres", "sqlite"]);

  let filter = $state<string>("");
  let openInstances = $state<Set<string>>(new Set());

  const instances = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    if (!q) return databases.value;
    return databases.value.filter(
      (d) =>
        d.name.toLowerCase().includes(q) ||
        d.engineLabel.toLowerCase().includes(q) ||
        d.engine.toLowerCase().includes(q),
    );
  });

  /** Status-pill tone per instance state (mirrors the old detail header). */
  const statusToneClass: Record<InstanceStatus, string> = {
    running: "bg-status-running/15 text-status-running",
    stopped: "bg-fg-subtle/15 text-fg-subtle",
    starting: "bg-status-starting/15 text-status-starting",
    errored: "bg-status-crashed/15 text-status-crashed",
  };

  /** Resolve a linked-project id to its display name (falls back to the id). */
  function projectName(id: string): string {
    return projects.value.find((p) => p.id === id)?.name ?? id;
  }

  function tableKey(table: DbClientTable): string {
    return `${table.schema ?? ""}.${table.name}`;
  }

  function canInspect(inst: DatabaseInstanceView): boolean {
    return DB_CLIENT_ENGINES.has(inst.engine);
  }

  function toggleInstance(inst: DatabaseInstanceView) {
    const next = new Set(openInstances);
    if (next.has(inst.id)) {
      next.delete(inst.id);
    } else {
      next.add(inst.id);
      if (canInspect(inst)) void dbWorkspace.loadSchema(inst.id);
    }
    openInstances = next;
  }

  function selectInstance(inst: DatabaseInstanceView) {
    dbWorkspace.selectInstance(inst.id);
    if (canInspect(inst) && !openInstances.has(inst.id)) toggleInstance(inst);
  }
</script>

<aside class="flex h-full w-[264px] shrink-0 flex-col border-r border-border/60 bg-surface/40">
  <!-- Header -->
  <div class="shrink-0 flex items-center gap-2 px-3 py-2.5 border-b border-border/60">
    <h2 class="text-[12px] font-semibold text-fg flex-1">Databases</h2>
    <button
      type="button"
      onclick={() => databases.showWizard()}
      title="Add database"
      aria-label="Add database"
      class="inline-flex items-center justify-center w-7 h-7 rounded-md border border-border
             text-fg-muted hover:bg-surface-2 hover:text-fg transition-colors"
    >
      <Icon name="plus" size={13} />
    </button>
  </div>

  <!-- Filter -->
  <div class="shrink-0 px-3 py-2 border-b border-border/60">
    <label
      class="inline-flex h-7 w-full items-center gap-2 rounded-md border border-border
             bg-surface-2/50 px-2 text-[11px] text-fg-subtle"
    >
      <Icon name="search" size={11} />
      <input
        bind:value={filter}
        placeholder="Filter databases"
        class="min-w-0 flex-1 bg-transparent text-fg focus:outline-none"
      />
    </label>
  </div>

  <!-- One card per database -->
  <div class="flex-1 min-h-0 overflow-y-auto px-3 py-3 space-y-2">
    {#if instances.length === 0}
      <p class="px-2 py-2 text-[11px] text-fg-subtle">
        {databases.value.length === 0
          ? "No databases yet. Add one to browse its tables."
          : "No databases match the filter."}
      </p>
    {:else}
      {#each instances as inst (inst.id)}
        {@const entry = dbWorkspace.schemaEntry(inst.id)}
        {@const inspectable = canInspect(inst)}
        {@const instActive = dbWorkspace.activeInstanceId === inst.id}
        {@const expanded = openInstances.has(inst.id)}
        <!-- One card per database: its name is the title, linked project(s)
             are the "environment", and the chevron expands its worktree. -->
        <div
          class="rounded-xl border transition-colors {instActive
            ? 'border-accent/40 bg-accent/[0.06]'
            : 'border-border/70 bg-surface/70 hover:border-border'}"
        >
          <div class="flex items-start gap-2 px-2.5 py-2.5">
            {#if inspectable}
              <button
                type="button"
                onclick={() => toggleInstance(inst)}
                aria-expanded={expanded}
                aria-label={expanded ? `Collapse ${inst.name}` : `Expand ${inst.name}`}
                class="mt-0.5 shrink-0 rounded p-0.5 text-fg-subtle hover:bg-surface-2 hover:text-fg"
              >
                <Icon name={expanded ? "chevron-down" : "chevron-right"} size={13} />
              </button>
            {:else}
              <span class="w-[20px] shrink-0"></span>
            {/if}
            <DatabaseMark id={inst.engine} size={24} class="mt-0.5 shrink-0" />
            <button
              type="button"
              onclick={() => selectInstance(inst)}
              title={`${inst.engineLabel}${inst.version ? ` ${inst.version}` : ""}`}
              class="min-w-0 flex-1 text-left"
            >
              <span
                class="block truncate text-[13.5px] font-semibold {instActive
                  ? 'text-fg'
                  : 'text-fg-muted'}"
              >
                {inst.name}
              </span>
              <span class="mt-0.5 block truncate text-[11px] text-fg-subtle">
                {inst.engineLabel}{inst.version ? ` ${inst.version}` : ""}{inst.fileBased
                  ? ""
                  : ` · :${inst.port}`}
              </span>
            </button>
            <span
              class="shrink-0 inline-flex items-center gap-1 rounded-full px-1.5 py-0.5
                     text-[10px] font-medium {statusToneClass[inst.status]}"
            >
              <StatusDot
                status={inst.status === "running" ? "running" : "stopped"}
                size="sm"
              />
              {statusLabel[inst.status]}
            </span>
          </div>

          {#if inst.linkedProjects.length > 0}
            <div class="-mt-1 flex flex-wrap items-center gap-1 pb-2.5 pl-[52px] pr-2.5">
              <Icon name="link" size={10} class="shrink-0 text-fg-subtle" />
              {#each inst.linkedProjects as pid (pid)}
                <span
                  class="inline-flex max-w-[150px] items-center truncate rounded-md
                         bg-surface-2/70 px-1.5 py-0.5 text-[10px] text-fg-muted"
                  title={projectName(pid)}
                >
                  {projectName(pid)}
                </span>
              {/each}
            </div>
          {/if}

          <!-- Worktree: table tree + feature shortcuts. The left border is the
               IDE-style guide line connecting the database to its children. -->
          {#if inspectable && expanded}
            <div class="px-2.5 pb-2.5">
              <div class="ml-[10px] space-y-0.5 border-l border-border/50 pl-2.5">
                {#if entry.loading && !entry.schema}
                  <p class="px-2 py-1 text-[11px] text-fg-subtle">Loading tables…</p>
                {:else if entry.error}
                  <p class="px-2 py-1 text-[11px] text-status-crashed">{entry.error}</p>
                {:else if entry.schema}
                  {#each entry.schema.tables as t (tableKey(t))}
                    {@const tableActive =
                      instActive &&
                      dbWorkspace.activeTab?.kind === "table" &&
                      dbWorkspace.activeTab?.table === t.name &&
                      (dbWorkspace.activeTab?.schema ?? null) === (t.schema ?? null)}
                    <button
                      type="button"
                      onclick={() => dbWorkspace.openTable(inst.id, t.schema ?? null, t.name)}
                      title={tableKey(t)}
                      class="flex w-full items-center gap-2 rounded px-2 py-1 text-left text-[12px]
                             transition-colors {tableActive
                        ? 'bg-accent/10 text-accent'
                        : 'text-fg-muted hover:bg-surface-2 hover:text-fg'}"
                    >
                      <Icon name="database" size={11} class="shrink-0 opacity-70" />
                      <span class="min-w-0 flex-1 truncate">{t.name}</span>
                      <span class="text-[10px] tabular-nums text-fg-subtle">{t.columns.length}</span>
                    </button>
                  {/each}
                  {#if entry.schema.tables.length === 0}
                    <p class="px-2 py-1 text-[11px] text-fg-subtle">No tables.</p>
                  {/if}
                  <div class="flex items-center gap-1 px-1 pt-1">
                    <button
                      type="button"
                      onclick={() => dbWorkspace.openErd(inst.id)}
                      class="rounded px-1.5 py-0.5 text-[10.5px] text-fg-subtle transition-colors hover:bg-surface-2 hover:text-fg"
                    >
                      Schema diagram
                    </button>
                    <button
                      type="button"
                      onclick={() => dbWorkspace.openQuery(inst.id)}
                      class="inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-[10.5px] text-fg-subtle transition-colors hover:bg-surface-2 hover:text-fg"
                    >
                      <Icon name="terminal" size={10} />
                      New query
                    </button>
                  </div>
                {/if}
              </div>
            </div>
          {/if}
        </div>
      {/each}
    {/if}
  </div>
</aside>
