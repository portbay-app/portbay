<!--
  ProjectRow — one row of the projects table.

  Action buttons map to the Tauri command surface from card #1. Failed
  actions surface via both the toast bus (safeInvoke) AND an inline error
  envelope directly beneath the row (P3 — Inline error rows). The
  envelope auto-clears when the project recovers to "running".
-->
<script lang="ts">
  import { onMount } from "svelte";
  import Badge from "$lib/components/atoms/Badge.svelte";
  import Icon from "$lib/components/atoms/Icon.svelte";
  import StatusDot from "$lib/components/atoms/StatusDot.svelte";
  import ErrorEnvelope from "$lib/components/errors/ErrorEnvelope.svelte";
  import { safeInvoke } from "$lib/ipc";
  import { projectDetailPanel } from "$lib/stores/detailPanel.svelte";
  import { devTools } from "$lib/stores/devTools.svelte";
  import { density } from "$lib/stores/density.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import type { CommandError } from "$lib/types/error";
  import type { ProjectView } from "$lib/types/projects";
  import { typeLabel } from "$lib/types/projects";

  interface Props {
    project: ProjectView;
  }
  let { project }: Props = $props();

  let busy = $state<"start" | "stop" | "restart" | "ide" | null>(null);

  const isSelected = $derived(projects.selectedId === project.id);
  const isRunning = $derived(
    project.status === "running" || project.status === "starting",
  );
  const compact = $derived(density.value === "compact");
  const cellClass = $derived(compact ? "py-1.5 px-3" : "py-2.5 px-4");
  /** Number of visible columns — Type column hides in compact mode. */
  const colCount = $derived(compact ? 4 : 5);

  const inlineError = $derived(projects.lastErrors[project.id] ?? null);

  /** PHP projects get an inline Xdebug toggle. The state is derived
   *  from XDEBUG_MODE in the project's env (off when absent). */
  const isPhp = $derived(project.type === "php");
  const xdebugOn = $derived(
    Boolean(project.env?.XDEBUG_MODE && project.env.XDEBUG_MODE !== "off"),
  );
  let xdebugBusy = $state<boolean>(false);

  async function toggleXdebug(e: MouseEvent) {
    e.stopPropagation();
    if (xdebugBusy) return;
    xdebugBusy = true;
    try {
      await safeInvoke<ProjectView>("set_xdebug_mode", {
        id: project.id,
        mode: xdebugOn ? "off" : "develop,debug",
      });
      await projects.refresh();
    } catch {
      /* toast already pushed */
    } finally {
      xdebugBusy = false;
    }
  }

  onMount(() => {
    void devTools.start();
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
      // Command succeeded — clear any stale inline error.
      projects.clearError(project.id);
    } catch (err) {
      // safeInvoke already pushed the toast; also persist as inline error.
      projects.setError(project.id, err as CommandError);
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

  async function openDevTool(ide: string) {
    if (busy) return;
    busy = "ide";
    try {
      await safeInvoke("open_in_ide", { id: project.id, ide });
    } catch {
      // safeInvoke already pushed the toast.
    } finally {
      busy = null;
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
  <td class={cellClass}>
    <div class="flex items-center gap-2 min-w-0">
      <StatusDot status={project.status} size="md" />
      <span class="font-medium text-fg truncate" title={typeLabel[project.type]}>
        {project.name}
      </span>
    </div>
  </td>

  <!-- Domains -->
  <td class="{cellClass} text-fg-muted">
    <span class="truncate">{project.hostname}</span>
  </td>

  <!-- Type -->
  {#if !compact}
    <td class={cellClass}>
      <Badge tone="neutral">{typeLabel[project.type]}</Badge>
    </td>
  {/if}

  <!-- Port -->
  <td class="{cellClass} text-fg-muted font-mono text-xs tabular-nums">
    {project.port ?? "—"}
  </td>

  <!-- Actions -->
  <td class={cellClass}>
    <div class="flex items-center gap-1 justify-end">
      {#if isPhp}
        <button
          type="button"
          onclick={toggleXdebug}
          disabled={xdebugBusy}
          title={xdebugOn
            ? "Xdebug enabled (XDEBUG_MODE=develop,debug). Click to disable."
            : "Xdebug disabled. Click to enable develop,debug mode."}
          aria-label={xdebugOn ? "Disable Xdebug" : "Enable Xdebug"}
          aria-pressed={xdebugOn}
          class="p-1.5 rounded-md transition-colors disabled:opacity-50"
          class:bg-accent={xdebugOn}
          class:text-on-accent={xdebugOn}
          class:text-fg-muted={!xdebugOn}
          class:hover:bg-surface-2={!xdebugOn}
          class:hover:text-fg={!xdebugOn}
        >
          <Icon name="circle-alert" size={14} />
        </button>
      {/if}
      <button
        type="button"
        onclick={(e) => { e.stopPropagation(); openUrl(); }}
        title="Open URL"
        aria-label="Open project URL"
        class="p-1.5 rounded-md text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
      >
        <Icon name="globe" size={14} />
      </button>
      {#if devTools.value.length === 1}
        <button
          type="button"
          onclick={(e) => {
            e.stopPropagation();
            void openDevTool(devTools.value[0].id);
          }}
          disabled={busy !== null}
          title="Open in {devTools.value[0].label}"
          aria-label="Open project in {devTools.value[0].label}"
          class="p-1.5 rounded-md text-fg-muted hover:text-fg hover:bg-surface-2 disabled:opacity-50 transition-colors"
        >
          <Icon name="terminal" size={14} />
        </button>
      {:else if devTools.value.length > 1}
        <select
          aria-label="Open project in developer tool"
          title="Open in developer tool"
          disabled={busy !== null}
          class="h-7 max-w-24 rounded-md border border-border bg-surface px-1.5 text-[11px] text-fg-muted hover:text-fg hover:border-border-strong focus:outline-none focus:ring-1 focus:ring-accent disabled:opacity-50"
          onchange={(e) => {
            e.stopPropagation();
            const select = e.currentTarget;
            const ide = select.value;
            select.value = "";
            if (ide) void openDevTool(ide);
          }}
          onclick={(e) => e.stopPropagation()}
        >
          <option value="">Open in…</option>
          {#each devTools.value as tool (tool.id)}
            <option value={tool.id}>{tool.label}</option>
          {/each}
        </select>
      {:else}
        <button
          type="button"
          disabled
          title="No supported editor or agent app found"
          aria-label="No supported editor or agent app found"
          class="p-1.5 rounded-md text-fg-subtle opacity-50"
        >
          <Icon name="terminal" size={14} />
        </button>
      {/if}
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

<!-- Inline error row — shows beneath the project row when an error is active. -->
{#if inlineError}
  <tr
    class="bg-surface-2/50"
    onclick={(e) => e.stopPropagation()}
  >
    <td colspan={colCount} class="px-4 py-2">
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
