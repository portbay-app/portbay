<!--
  Projects route (/) — page heading, status cards row, projects table,
  and the table footer (count + sort + view toggle).

  Sort + view-mode are local to this page (not persisted yet). The
  grid view is a placeholder — for now flipping to grid keeps the same
  table layout but reflows row density. A dedicated grid renderer
  lands in a follow-up.
-->
<script lang="ts">
  import { ProjectsTable, type SortKey } from "$lib/components/projects";
  import { StatusCards } from "$lib/components/dashboard";
  import Icon from "$lib/components/atoms/Icon.svelte";
  import { projects } from "$lib/stores/projects.svelte";

  let sortKey = $state<SortKey>("name-asc");
  let viewMode = $state<"list" | "grid">("list");

  const projectCount = $derived(projects.value.length);
</script>

<div class="px-6 py-5 space-y-6">
  <header class="space-y-1">
    <h1 class="text-[22px] font-semibold tracking-tight text-fg">Projects</h1>
    <p class="text-[13px] text-fg-muted">
      Manage your local development environment.
    </p>
  </header>

  <StatusCards />

  <ProjectsTable {sortKey} />

  <!-- Table footer: count, sort, view toggle -->
  <footer
    class="flex items-center justify-between gap-3 text-[12px] text-fg-muted"
  >
    <span class="tabular-nums">
      {projectCount} {projectCount === 1 ? "project" : "projects"}
    </span>
    <div class="flex items-center gap-2">
      <label class="flex items-center gap-1.5">
        <span class="text-fg-subtle">Sort by</span>
        <select
          bind:value={sortKey}
          aria-label="Sort projects"
          class="h-7 rounded-md border border-border bg-surface px-2
                 text-[12px] text-fg focus:outline-none focus:ring-1
                 focus:ring-accent/40"
        >
          <option value="name-asc">Name (A–Z)</option>
          <option value="name-desc">Name (Z–A)</option>
          <option value="status">Status</option>
          <option value="port">Port</option>
        </select>
      </label>

      <div
        class="inline-flex items-center rounded-md border border-border bg-surface p-0.5"
        role="group"
        aria-label="View mode"
      >
        <button
          type="button"
          onclick={() => (viewMode = "list")}
          aria-pressed={viewMode === "list"}
          title="List view"
          class="inline-flex items-center justify-center w-6 h-6 rounded {viewMode ===
          'list'
            ? 'bg-surface-2 text-fg'
            : 'text-fg-subtle hover:text-fg'} transition-colors"
        >
          <Icon name="list" size={13} />
        </button>
        <button
          type="button"
          onclick={() => (viewMode = "grid")}
          aria-pressed={viewMode === "grid"}
          title="Grid view (coming soon)"
          class="inline-flex items-center justify-center w-6 h-6 rounded {viewMode ===
          'grid'
            ? 'bg-surface-2 text-fg'
            : 'text-fg-subtle hover:text-fg'} transition-colors"
        >
          <Icon name="grid-2x2" size={13} />
        </button>
      </div>
    </div>
  </footer>
</div>
