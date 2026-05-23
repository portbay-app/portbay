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
  import { safeInvoke } from "$lib/ipc";
  import { projectDetailPanel } from "$lib/stores/detailPanel.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import { dns } from "$lib/stores/dns.svelte";
  import { search } from "$lib/stores/search.svelte";
  import EmptyState from "./EmptyState.svelte";
  import ProjectRow from "./ProjectRow.svelte";
  import ProjectCard from "./ProjectCard.svelte";
  import type { ProjectView } from "$lib/types/projects";

  export type SortKey = "name-asc" | "name-desc" | "status" | "port";
  export type ViewMode = "list" | "grid";

  interface Props {
    sortKey?: SortKey;
    viewMode?: ViewMode;
  }
  let { sortKey = "name-asc", viewMode = "list" }: Props = $props();

  onMount(() => {
    // Store lifetime is owned by the root layout — calling start()
    // here is a no-op when the listener is already up. We deliberately
    // do *not* stop() on unmount: other routes (/domains, /services,
    // /logs) share the same store and would lose their live data when
    // the user navigates away from the dashboard.
    void projects.start();
  });

  const statusRank: Record<string, number> = {
    running: 0,
    starting: 1,
    unhealthy: 2,
    port_conflict: 3,
    crashed: 4,
    stopped: 5,
  };

  function compare(a: ProjectView, b: ProjectView): number {
    switch (sortKey) {
      case "name-asc":
        return a.name.localeCompare(b.name);
      case "name-desc":
        return b.name.localeCompare(a.name);
      case "port":
        return (a.port ?? 0) - (b.port ?? 0);
      case "status": {
        const ra = statusRank[a.status] ?? 9;
        const rb = statusRank[b.status] ?? 9;
        if (ra !== rb) return ra - rb;
        return a.name.localeCompare(b.name);
      }
    }
  }

  const filtered = $derived.by(() => {
    const q = search.value.trim().toLowerCase();
    const base = q
      ? projects.value.filter(
          (p) =>
            p.name.toLowerCase().includes(q) ||
            p.hostname.toLowerCase().includes(q) ||
            p.id.toLowerCase().includes(q),
        )
      : projects.value;
    return base.slice().sort(compare);
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
      void (async () => {
        await dns.ensureReady();
        await safeInvoke("start_project", { id: sel });
      })();
    } else if (e.key === "x" || e.key === "X") {
      void safeInvoke("stop_project", { id: sel });
    } else if (e.key === "r" || e.key === "R") {
      void safeInvoke("restart_project", { id: sel });
    } else if (e.key === "Enter") {
      projectDetailPanel.show(sel);
    }
  }
</script>

<svelte:window onkeydown={onKeydown} />

{#if projects.loading && projects.value.length === 0}
  <div class="bg-surface border border-border rounded-2xl">
    <p class="text-xs text-fg-subtle px-4 py-6">Loading projects…</p>
  </div>
{:else if projects.value.length === 0}
  <div class="bg-surface border border-border rounded-2xl px-4 py-6">
    <EmptyState variant="registry-empty" />
  </div>
{:else if filtered.length === 0}
  <div class="bg-surface border border-border rounded-2xl px-4 py-6">
    <EmptyState variant="filtered" query={search.value} />
  </div>
{:else if viewMode === "grid"}
  <!--
    Grid view — responsive 1/2/3 column card grid. No outer card chrome
    so the cards themselves carry the visual weight.
  -->
  <div
    class="grid gap-3 grid-cols-1 sm:grid-cols-2 xl:grid-cols-3"
    role="list"
    aria-label="Projects"
  >
    {#each filtered as project (project.id)}
      <ProjectCard {project} />
    {/each}
  </div>
{:else}
  <!--
    The table card intentionally does NOT clip overflow — the per-row
    ellipsis menu is `position: fixed` and needs to escape the card
    bounds. ProjectRowMenu handles its own layering.
  -->
  <div class="bg-surface border border-border rounded-2xl">
    <table class="w-full">
      <thead>
        <tr
          class="text-[11px] uppercase tracking-wide text-fg-subtle text-left
                 border-b border-border bg-surface-2/40"
        >
          <th class="py-2.5 px-4 font-medium">Project</th>
          <th class="py-2.5 px-4 font-medium">Stack</th>
          <th class="py-2.5 px-4 font-medium">URL</th>
          <th class="py-2.5 px-4 font-medium">Port</th>
          <th class="py-2.5 px-4 font-medium">Status</th>
          <th class="py-2.5 px-4 font-medium text-right">Actions</th>
        </tr>
      </thead>
      <tbody>
        {#each filtered as project (project.id)}
          <ProjectRow {project} />
        {/each}
      </tbody>
    </table>
  </div>
{/if}
