<!--
  ProjectRow — one row of the projects table.

  Action buttons map to the Tauri command surface from card #1. Failed
  actions surface via the toast bus from card #4 (safeInvoke already
  pushes the envelope); no inline error row in Phase 2 (deferred to a
  follow-up since `applyStatusEvent` only carries status, not lastError).
-->
<script lang="ts">
  import Badge from "$lib/components/atoms/Badge.svelte";
  import Icon from "$lib/components/atoms/Icon.svelte";
  import StatusDot from "$lib/components/atoms/StatusDot.svelte";
  import { safeInvoke } from "$lib/ipc";
  import { projectDetailPanel } from "$lib/stores/detailPanel.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import type { ProjectView } from "$lib/types/projects";
  import { typeLabel } from "$lib/types/projects";

  interface Props {
    project: ProjectView;
  }
  let { project }: Props = $props();

  let busy = $state<"start" | "stop" | "restart" | null>(null);

  const isSelected = $derived(projects.selectedId === project.id);
  const isRunning = $derived(
    project.status === "running" || project.status === "starting",
  );

  async function run(op: "start" | "stop" | "restart") {
    if (busy) return;
    busy = op;
    try {
      switch (op) {
        case "start":
          await safeInvoke("start_project", { id: project.id });
          break;
        case "stop":
          await safeInvoke("stop_project", { id: project.id });
          break;
        case "restart":
          await safeInvoke("restart_project", { id: project.id });
          break;
      }
    } catch {
      // safeInvoke already pushed the toast.
    } finally {
      busy = null;
    }
  }

  async function openUrl() {
    try {
      await safeInvoke("open_project", { id: project.id });
    } catch {
      // toast already pushed
    }
  }
</script>

<tr
  onclick={() => {
    projects.select(project.id);
    projectDetailPanel.show(project.id);
  }}
  data-selected={isSelected}
  class="border-b border-border text-sm cursor-pointer transition-colors
         hover:bg-surface-2
         data-[selected=true]:bg-accent/8"
>
  <!-- Name + status dot -->
  <td class="py-2.5 px-4">
    <div class="flex items-center gap-2 min-w-0">
      <StatusDot status={project.status} size="md" />
      <span class="font-medium text-fg truncate">{project.name}</span>
    </div>
  </td>

  <!-- Domains -->
  <td class="py-2.5 px-4 text-fg-muted">
    <span class="truncate">{project.hostname}</span>
  </td>

  <!-- Type -->
  <td class="py-2.5 px-4">
    <Badge tone="neutral">{typeLabel[project.type]}</Badge>
  </td>

  <!-- Port -->
  <td class="py-2.5 px-4 text-fg-muted font-mono text-xs tabular-nums">
    {project.port ?? "—"}
  </td>

  <!-- Actions -->
  <td class="py-2.5 px-4">
    <div class="flex items-center gap-1 justify-end">
      <button
        type="button"
        onclick={(e) => { e.stopPropagation(); openUrl(); }}
        title="Open URL"
        aria-label="Open project URL"
        class="p-1.5 rounded-md text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
      >
        <Icon name="globe" size={14} />
      </button>
      {#if isRunning}
        <button
          type="button"
          onclick={(e) => { e.stopPropagation(); run("stop"); }}
          disabled={busy !== null}
          title="Stop"
          aria-label="Stop project"
          class="p-1.5 rounded-md text-status-crashed hover:bg-status-crashed/10 disabled:opacity-50 transition-colors"
        >
          <Icon name="square" size={14} />
        </button>
      {:else}
        <button
          type="button"
          onclick={(e) => { e.stopPropagation(); run("start"); }}
          disabled={busy !== null}
          title="Start"
          aria-label="Start project"
          class="p-1.5 rounded-md text-status-running hover:bg-status-running/10 disabled:opacity-50 transition-colors"
        >
          <Icon name="play" size={14} />
        </button>
      {/if}
      <button
        type="button"
        onclick={(e) => { e.stopPropagation(); run("restart"); }}
        disabled={busy !== null}
        title="Restart"
        aria-label="Restart project"
        class="p-1.5 rounded-md text-fg-muted hover:text-fg hover:bg-surface-2 disabled:opacity-50 transition-colors"
      >
        <Icon name="rotate-cw" size={14} />
      </button>
    </div>
  </td>
</tr>
