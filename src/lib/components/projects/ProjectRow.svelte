<!--
  ProjectRow — one row of the redesigned projects table.

  Columns: Project (avatar + name + group subtitle), Stack (icon +
  label), URL (clickable), Port, Status (dot + label), Actions
  (primary stop/start + ellipsis menu).

  Row click selects the project — the right rail shows the detail.
  Editing the project is an explicit "Edit…" action in the ellipsis
  menu (or the rail's footer link) so a stray click doesn't pop the
  heavy modal.

  Inline error envelopes appear in a follow-up row when an action
  fails — same shape as the previous design.
-->
<script lang="ts">
  import Icon from "$lib/components/atoms/Icon.svelte";
  import StatusDot from "$lib/components/atoms/StatusDot.svelte";
  import StackIcon from "$lib/components/atoms/StackIcon.svelte";
  import ProjectAvatar from "$lib/components/atoms/ProjectAvatar.svelte";
  import ErrorEnvelope from "$lib/components/errors/ErrorEnvelope.svelte";

  import { safeInvoke } from "$lib/ipc";
  import { groups } from "$lib/stores/groups.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import { density } from "$lib/stores/density.svelte";

  import type { CommandError } from "$lib/types/error";
  import type { ProjectView } from "$lib/types/projects";
  import type { PortbayStatus } from "$lib/types/status";
  import { typeLabel } from "$lib/types/projects";

  import ProjectRowMenu from "./ProjectRowMenu.svelte";

  interface Props {
    project: ProjectView;
  }
  let { project }: Props = $props();

  let busy = $state<"start" | "stop" | "restart" | null>(null);

  const isSelected = $derived(projects.selectedId === project.id);
  const isRunning = $derived(
    project.status === "running" || project.status === "starting",
  );
  const compact = $derived(density.value === "compact");
  const cellClass = $derived(compact ? "py-2 px-3" : "py-3 px-4");

  const inlineError = $derived(projects.lastErrors[project.id] ?? null);

  // Subtitle = first group the project belongs to. Projects in zero
  // groups fall back to the type label so the row never feels empty.
  const groupSubtitle = $derived.by<string>(() => {
    const g = groups.value.find((g) => g.knownIds.includes(project.id));
    if (g) return g.name;
    return typeLabel[project.type];
  });

  const statusLabel = $derived.by<string>(() => {
    const m: Record<PortbayStatus, string> = {
      running: "Running",
      stopped: "Stopped",
      starting: "Starting",
      unhealthy: "Unhealthy",
      crashed: "Crashed",
      port_conflict: "Port conflict",
    };
    return m[project.status];
  });

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
      projects.clearError(project.id);
    } catch (err) {
      projects.setError(project.id, err as CommandError);
    } finally {
      busy = null;
    }
  }

  async function openUrl(e: MouseEvent) {
    e.stopPropagation();
    try {
      await safeInvoke("open_project", { id: project.id });
    } catch {
      /* toast already pushed */
    }
  }
</script>

<tr
  onclick={() => projects.select(project.id)}
  data-selected={isSelected}
  class="border-b border-border text-sm cursor-pointer transition-colors
         hover:bg-surface-2
         data-[selected=true]:bg-accent/10
         data-[selected=true]:ring-1 data-[selected=true]:ring-inset
         data-[selected=true]:ring-accent/40"
>
  <!-- Project: avatar + name + group subtitle -->
  <td class={cellClass}>
    <div class="flex items-center gap-3 min-w-0">
      <ProjectAvatar
        id={project.id}
        name={project.name}
        size={32}
      />
      <div class="min-w-0 leading-tight">
        <p class="text-[13.5px] font-semibold text-fg truncate">
          {project.name}
        </p>
        <p class="text-[11px] text-fg-subtle truncate">
          {groupSubtitle}
        </p>
      </div>
    </div>
  </td>

  <!-- Stack -->
  <td class={cellClass}>
    <div class="flex items-center gap-2 text-fg-muted text-[12px]">
      <StackIcon type={project.type} size={16} />
      <span class="truncate">{typeLabel[project.type]}</span>
    </div>
  </td>

  <!-- URL -->
  <td class={cellClass}>
    <button
      type="button"
      onclick={openUrl}
      class="inline-flex items-center gap-1 text-[12px] text-accent
             hover:text-accent-hover hover:underline truncate"
      title="Open {project.url}"
    >
      <span class="truncate">{project.url}</span>
      <Icon name="external-link" size={11} class="shrink-0 opacity-70" />
    </button>
  </td>

  <!-- Port -->
  <td class="{cellClass} text-fg-muted font-mono text-[12px] tabular-nums">
    {project.port ?? "—"}
  </td>

  <!-- Status -->
  <td class={cellClass}>
    <span class="inline-flex items-center gap-1.5 text-[12px]">
      <StatusDot status={project.status} size="md" />
      <span
        class="text-fg-muted"
        class:text-status-running={project.status === "running"}
        class:text-status-unhealthy={project.status === "unhealthy" ||
          project.status === "port_conflict"}
        class:text-status-crashed={project.status === "crashed"}
      >
        {statusLabel}
      </span>
    </span>
  </td>

  <!-- Actions: primary stop/start + ellipsis -->
  <td class={cellClass}>
    <div class="flex items-center gap-1.5 justify-end">
      {#if isRunning}
        <button
          type="button"
          onclick={(e) => {
            e.stopPropagation();
            void run("stop");
          }}
          disabled={busy !== null}
          title="Stop {project.name}"
          aria-label="Stop {project.name}"
          class="inline-flex items-center justify-center w-8 h-8 rounded-md
                 text-on-accent bg-accent hover:brightness-110
                 active:brightness-95 disabled:opacity-50 transition"
        >
          {#if busy === "stop"}
            <Icon name="refresh-cw" size={12} class="animate-spin" />
          {:else}
            <Icon name="square" size={11} class="fill-current" />
          {/if}
        </button>
      {:else}
        <button
          type="button"
          onclick={(e) => {
            e.stopPropagation();
            void run("start");
          }}
          disabled={busy !== null}
          title="Start {project.name}"
          aria-label="Start {project.name}"
          class="inline-flex items-center justify-center w-8 h-8 rounded-md
                 text-status-running border border-status-running/40
                 hover:bg-status-running/10 disabled:opacity-50 transition"
        >
          {#if busy === "start"}
            <Icon name="refresh-cw" size={12} class="animate-spin" />
          {:else}
            <Icon name="play" size={12} class="fill-current" />
          {/if}
        </button>
      {/if}

      <ProjectRowMenu {project} />
    </div>
  </td>
</tr>

<!-- Inline error envelope -->
{#if inlineError}
  <tr
    class="bg-surface-2/50"
    onclick={(e) => e.stopPropagation()}
  >
    <td colspan="6" class="px-4 py-2">
      <div class="flex items-start gap-2">
        <div class="flex-1 min-w-0">
          <ErrorEnvelope envelope={inlineError} tone="inline" />
        </div>
        <button
          type="button"
          onclick={() => projects.clearError(project.id)}
          title="Dismiss error"
          aria-label="Dismiss inline error"
          class="shrink-0 mt-1 p-1 rounded-md text-fg-subtle hover:text-fg hover:bg-surface-2 transition-colors"
        >
          <Icon name="x" size={14} />
        </button>
      </div>
    </td>
  </tr>
{/if}
