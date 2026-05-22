<!--
  ProjectsTable — the central table of registered projects.

  Loads on mount, listens to portbay://status events, filters by the
  global search store, and exposes basic keyboard shortcuts:
    Up / Down — change selection
    S / X / R — start / stop / restart the selected project
    Enter    — opens the detail panel (card #9)
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { DashboardCard } from "$lib/components/atoms";
  import { safeInvoke } from "$lib/ipc";
  import { projects } from "$lib/stores/projects";
  import { search } from "$lib/stores/search";
  import EmptyState from "./EmptyState.svelte";
  import ProjectRow from "./ProjectRow.svelte";

  onMount(() => {
    void projects.start();
    return () => projects.stop();
  });

  const filtered = $derived.by(() => {
    const q = search.value.trim().toLowerCase();
    if (!q) return projects.value;
    return projects.value.filter(
      (p) =>
        p.name.toLowerCase().includes(q) ||
        p.hostname.toLowerCase().includes(q) ||
        p.id.toLowerCase().includes(q),
    );
  });

  function isEditableTarget(el: EventTarget | null): boolean {
    if (!(el instanceof HTMLElement)) return false;
    const tag = el.tagName.toLowerCase();
    return (
      tag === "input" ||
      tag === "textarea" ||
      tag === "select" ||
      el.isContentEditable
    );
  }

  function onKeydown(e: KeyboardEvent) {
    if (isEditableTarget(e.target)) return;
    if (e.metaKey || e.ctrlKey || e.altKey) return;

    if (e.key === "ArrowDown") {
      e.preventDefault();
      projects.selectRelative(1);
      return;
    }
    if (e.key === "ArrowUp") {
      e.preventDefault();
      projects.selectRelative(-1);
      return;
    }

    const sel = projects.selectedId;
    if (!sel) return;

    if (e.key === "s" || e.key === "S") {
      void safeInvoke("start_project", { id: sel });
    } else if (e.key === "x" || e.key === "X") {
      void safeInvoke("stop_project", { id: sel });
    } else if (e.key === "r" || e.key === "R") {
      void safeInvoke("restart_project", { id: sel });
    }
  }
</script>

<svelte:window onkeydown={onKeydown} />

<DashboardCard title="Websites" flush>
  {#snippet badge()}
    <span class="text-xs text-fg-muted tabular-nums">
      {projects.value.length} total
    </span>
  {/snippet}

  {#if projects.loading && projects.value.length === 0}
    <p class="text-xs text-fg-subtle py-4">Loading projects…</p>
  {:else if projects.value.length === 0}
    <EmptyState variant="registry-empty" />
  {:else if filtered.length === 0}
    <EmptyState variant="filtered" query={search.value} />
  {:else}
    <table class="w-full">
      <thead>
        <tr class="text-xs text-fg-muted text-left border-b border-border">
          <th class="py-2 px-4 font-medium">Name</th>
          <th class="py-2 px-4 font-medium">Domains</th>
          <th class="py-2 px-4 font-medium">Type</th>
          <th class="py-2 px-4 font-medium">Port</th>
          <th class="py-2 px-4 font-medium text-right">Actions</th>
        </tr>
      </thead>
      <tbody>
        {#each filtered as project (project.id)}
          <ProjectRow {project} />
        {/each}
      </tbody>
    </table>
  {/if}
</DashboardCard>
