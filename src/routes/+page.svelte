<!--
  Projects route (/) — page heading, status cards row, projects table,
  and the table footer (count + sort + view toggle).

  Sort + view-mode persist across launches via localStorage.
-->
<script lang="ts">
  import { ProjectsTable, type SortKey } from "$lib/components/projects";
  import { StatusCards } from "$lib/components/dashboard";
  import Icon from "$lib/components/atoms/Icon.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import { goto } from "$app/navigation";

  // ---- Sort + view persistence ----
  // Restore the user's last choices; fall back to the default on an unknown or
  // unreadable value (private mode). A `$effect` writes each back on change.
  const SORT_STORAGE_KEY = "portbay:dashboard-sort";
  const VIEW_STORAGE_KEY = "portbay:dashboard-view";
  const SORT_KEYS: readonly SortKey[] = ["name-asc", "name-desc", "status", "port"];

  function readStored<T extends string>(
    key: string,
    allowed: readonly T[],
    fallback: T,
  ): T {
    try {
      const v = localStorage.getItem(key);
      return allowed.includes(v as T) ? (v as T) : fallback;
    } catch {
      return fallback;
    }
  }

  let sortKey = $state<SortKey>(readStored(SORT_STORAGE_KEY, SORT_KEYS, "name-asc"));
  let viewMode = $state<"list" | "grid">(
    readStored(VIEW_STORAGE_KEY, ["list", "grid"] as const, "list"),
  );

  $effect(() => {
    try {
      localStorage.setItem(SORT_STORAGE_KEY, sortKey);
    } catch {
      /* private mode — running without persistence is fine */
    }
  });
  $effect(() => {
    try {
      localStorage.setItem(VIEW_STORAGE_KEY, viewMode);
    } catch {
      /* private mode — running without persistence is fine */
    }
  });

  const projectCount = $derived(projects.value.length);

  // ---- MCP nudge ----
  const MCP_NUDGE_KEY = "portbay:mcp-nudge-dismissed";

  function readNudgeDismissed(): boolean {
    try {
      return localStorage.getItem(MCP_NUDGE_KEY) === "1";
    } catch {
      return false;
    }
  }

  let nudgeDismissed = $state<boolean>(readNudgeDismissed());

  function dismissNudge() {
    nudgeDismissed = true;
    try {
      localStorage.setItem(MCP_NUDGE_KEY, "1");
    } catch {
      /* private mode */
    }
  }

  const showMcpNudge = $derived(projectCount >= 1 && !nudgeDismissed);
</script>

<div class="px-6 py-5 space-y-6">
  <header class="space-y-1">
    <h1 class="text-[22px] font-semibold tracking-tight text-fg">Projects</h1>
    <p class="text-[13px] text-fg-muted">
      Manage your local development environment.
    </p>
  </header>

  <!-- MCP nudge — shown once per install when at least one project exists -->
  {#if showMcpNudge}
    <div
      class="flex items-center justify-between gap-3 rounded-xl border
             border-accent/30 bg-accent/8 px-4 py-2.5"
    >
      <div class="flex items-center gap-2 min-w-0">
        <Icon name="sparkles" size={14} class="shrink-0 text-accent" />
        <span class="text-[13px] text-fg-muted">
          Control PortBay from Claude Code or Cursor —
          <button
            type="button"
            onclick={() => void goto("/settings?tab=ai")}
            class="text-accent hover:underline"
          >
            AI Integrations →
          </button>
        </span>
      </div>
      <button
        type="button"
        onclick={dismissNudge}
        aria-label="Dismiss AI integrations nudge"
        class="shrink-0 inline-flex items-center justify-center w-6 h-6
               rounded-md text-fg-subtle hover:text-fg hover:bg-surface-2
               transition-colors"
      >
        <Icon name="x" size={12} />
      </button>
    </div>
  {/if}

  <StatusCards />

  <ProjectsTable {sortKey} {viewMode} />

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
          title="Grid view"
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
