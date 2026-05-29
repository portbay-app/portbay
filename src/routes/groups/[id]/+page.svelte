<!--
  /groups/[id] — filtered projects view for one group.

  Header shows group name, member counts, and the three batch actions
  (Start all / Stop all / Restart all) plus Edit / Delete. Body is a
  ProjectRow table filtered to the group's members. Missing members
  (drift between the group and the registry) surface as a separate
  warning row.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { page } from "$app/state";

  import { DashboardCard, Icon } from "$lib/components/atoms";
  import EmptyState from "$lib/components/projects/EmptyState.svelte";
  import ProjectRow from "$lib/components/projects/ProjectRow.svelte";
  import { safeInvoke } from "$lib/ipc";
  import { errorBus } from "$lib/stores/errors.svelte";
  import { groupEditor } from "$lib/stores/groupEditor.svelte";
  import { groups } from "$lib/stores/groups.svelte";
  import { density } from "$lib/stores/density.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import type { GroupOpReport } from "$lib/types/groups";

  const groupId = $derived(page.params.id);
  const group = $derived(groups.value.find((g) => g.id === groupId) ?? null);

  let busy = $state<"start" | "stop" | "restart" | "delete" | null>(null);
  let confirmingDelete = $state<boolean>(false);

  onMount(() => {
    // Keep projects polling so the per-row status is live.
    void projects.start();
    void groups.refresh();
    return () => projects.stop();
  });

  /** Members that exist in the registry — the table rows. */
  const memberRows = $derived.by(() => {
    if (!group) return [];
    const known = new Set(group.knownIds);
    return projects.value.filter((p) => known.has(p.id));
  });

  /** Members listed in the group but no longer in the registry — shown
   *  as a warning so the user can clean them up. */
  const missingIds = $derived.by(() => {
    if (!group) return [] as string[];
    const known = new Set(group.knownIds);
    return group.projectIds.filter((id) => !known.has(id));
  });

  /** Computed status — how many group members are running right now. */
  const liveCount = $derived(
    memberRows.filter((p) => p.status === "running").length,
  );

  async function runOp(op: "start" | "stop" | "restart") {
    if (!group) return;
    busy = op;
    try {
      const report = await safeInvoke<GroupOpReport>(`${op}_group`, {
        id: group.id,
      });
      errorBus.push({
        code: `GROUP_${op.toUpperCase()}_REPORT`,
        whatHappened: `${op}: ${report.succeeded} succeeded, ${report.failed} failed.`,
        whyItMatters:
          report.failed > 0
            ? "See per-row inline errors for the failures."
            : "All members responded successfully.",
        whoCausedIt: "system",
        actions: [],
      });
    } catch {
      /* toast already pushed */
    } finally {
      busy = null;
    }
  }

  async function deleteGroup() {
    if (!group) return;
    busy = "delete";
    try {
      await groups.remove(group.id);
      await goto("/");
    } catch {
      /* toast pushed */
    } finally {
      busy = null;
      confirmingDelete = false;
    }
  }

  const compact = $derived(density.value === "compact");
</script>

<div class="p-6 space-y-4">
  {#if !group}
    <DashboardCard title="Group not found" flush>
      <p class="text-sm text-fg-muted">
        No group with id <code class="font-mono">{groupId}</code>. It may
        have been deleted from another window.
      </p>
      <a href="/" class="inline-flex items-center gap-1 text-xs text-accent mt-3">
        <Icon name="chevron-right" size={11} />
        Back to projects
      </a>
    </DashboardCard>
  {:else}
    <!-- Header -->
    <header class="flex items-start justify-between gap-3 flex-wrap">
      <div class="min-w-0">
        <h2 class="text-lg font-semibold tracking-tight truncate">
          {group.name}
        </h2>
        <p class="text-xs text-fg-muted mt-0.5">
          {liveCount} of {memberRows.length} member{memberRows.length === 1
            ? ""
            : "s"} running
          {#if missingIds.length > 0}
            · <span class="text-status-unhealthy">
              {missingIds.length} stale
            </span>
          {/if}
        </p>
      </div>

      <div class="flex items-center gap-1.5 flex-wrap">
        <button
          type="button"
          onclick={() => runOp("start")}
          disabled={busy !== null || memberRows.length === 0}
          class="inline-flex items-center gap-1.5 px-2.5 py-1.5 text-xs rounded-md
                 text-accent border border-accent/40 hover:bg-accent/10
                 disabled:opacity-50 transition-colors"
        >
          <Icon
            name={busy === "start" ? "refresh-cw" : "play"}
            size={11}
            class={busy === "start" ? "animate-spin" : ""}
          />
          Start all
        </button>
        <button
          type="button"
          onclick={() => runOp("stop")}
          disabled={busy !== null || memberRows.length === 0}
          class="inline-flex items-center gap-1.5 px-2.5 py-1.5 text-xs rounded-md
                 text-fg-muted border border-border hover:text-fg hover:bg-surface-2
                 disabled:opacity-50 transition-colors"
        >
          <Icon
            name={busy === "stop" ? "refresh-cw" : "square"}
            size={11}
            class={busy === "stop" ? "animate-spin" : ""}
          />
          Stop all
        </button>
        <button
          type="button"
          onclick={() => runOp("restart")}
          disabled={busy !== null || memberRows.length === 0}
          class="inline-flex items-center gap-1.5 px-2.5 py-1.5 text-xs rounded-md
                 text-fg-muted border border-border hover:text-fg hover:bg-surface-2
                 disabled:opacity-50 transition-colors"
        >
          <Icon
            name="rotate-cw"
            size={11}
            class={busy === "restart" ? "animate-spin" : ""}
          />
          Restart all
        </button>
        <span class="w-px h-5 bg-border mx-1"></span>
        <button
          type="button"
          onclick={() => groupEditor.edit(group)}
          class="inline-flex items-center gap-1.5 px-2.5 py-1.5 text-xs rounded-md
                 text-fg-muted border border-border hover:text-fg hover:bg-surface-2
                 transition-colors"
        >
          <Icon name="pencil" size={11} /> Edit
        </button>
        {#if confirmingDelete}
          <button
            type="button"
            onclick={deleteGroup}
            disabled={busy !== null}
            class="inline-flex items-center gap-1.5 px-2.5 py-1.5 text-xs rounded-md
                   text-status-crashed border border-status-crashed/60 bg-status-crashed/10
                   hover:bg-status-crashed/20 disabled:opacity-50 transition-colors"
          >
            <Icon name="x" size={11} /> Confirm delete
          </button>
          <button
            type="button"
            onclick={() => (confirmingDelete = false)}
            class="px-2.5 py-1.5 text-xs rounded-md text-fg-muted hover:text-fg
                   hover:bg-surface-2 transition-colors"
          >
            Cancel
          </button>
        {:else}
          <button
            type="button"
            onclick={() => (confirmingDelete = true)}
            class="inline-flex items-center gap-1.5 px-2.5 py-1.5 text-xs rounded-md
                   text-fg-subtle border border-border hover:text-status-crashed
                   hover:border-status-crashed/40 transition-colors"
          >
            <Icon name="x" size={11} /> Delete
          </button>
        {/if}
      </div>
    </header>

    {#if missingIds.length > 0}
      <div
        class="px-3 py-2 rounded-md border border-status-unhealthy/40 bg-status-unhealthy/5
               text-xs text-fg-muted flex items-start gap-2"
      >
        <Icon name="info" size={12} class="mt-0.5 text-status-unhealthy" />
        <div class="flex-1 min-w-0">
          <div class="font-medium text-status-unhealthy">
            Stale member{missingIds.length === 1 ? "" : "s"}
          </div>
          <div class="font-mono">
            {missingIds.join(", ")}
          </div>
          <div class="mt-1">
            These ids are listed in the group but no longer in the registry.
            Click <em>Edit</em> to remove them.
          </div>
        </div>
      </div>
    {/if}

    <DashboardCard title="Members" flush>
      {#if memberRows.length === 0}
        <EmptyState variant="registry-empty" />
      {:else}
        <!--
          Scroll horizontally on narrow windows instead of crushing the
          columns. A `position: fixed` row menu escapes this overflow box, so
          it isn't clipped. `min-w` keeps the columns legible until the scroll
          kicks in.
        -->
        <div class="overflow-x-auto">
        <table class="w-full min-w-[640px]">
          <thead>
            <tr class="text-xs text-fg-muted text-left border-b border-border">
              <th class="py-2 px-4 font-medium">Name</th>
              <th class="py-2 px-4 font-medium">Domains</th>
              {#if !compact}
                <th class="py-2 px-4 font-medium">Type</th>
              {/if}
              <th class="py-2 px-4 font-medium">Port</th>
              <th class="py-2 px-4 font-medium text-right">Actions</th>
            </tr>
          </thead>
          <tbody>
            {#each memberRows as project (project.id)}
              <ProjectRow {project} />
            {/each}
          </tbody>
        </table>
        </div>
      {/if}
    </DashboardCard>
  {/if}
</div>
