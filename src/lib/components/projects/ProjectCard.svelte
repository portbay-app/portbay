<!--
  ProjectCard — grid-view variant of a single project.

  Mirrors the data ProjectRow surfaces (avatar, name, stack, URL, port,
  status, primary start/stop, ellipsis menu) in a square-ish card so the
  dashboard can swap to a card grid via the footer view toggle.
-->
<script lang="ts">
  import Icon from "$lib/components/atoms/Icon.svelte";
  import StatusPill from "$lib/components/atoms/StatusPill.svelte";
  import StackIcon from "$lib/components/atoms/StackIcon.svelte";
  import ProjectAvatar from "$lib/components/atoms/ProjectAvatar.svelte";
  import ErrorEnvelope from "$lib/components/errors/ErrorEnvelope.svelte";

  import { safeInvoke } from "$lib/ipc";
  import { projects } from "$lib/stores/projects.svelte";

  import type { CommandError } from "$lib/types/error";
  import type { ProjectView } from "$lib/types/projects";
  import { typeLabel } from "$lib/types/projects";

  import ProjectRowMenu from "./ProjectRowMenu.svelte";

  interface Props {
    project: ProjectView;
  }
  let { project }: Props = $props();

  let busy = $state<"start" | "stop" | null>(null);

  const isSelected = $derived(projects.selectedId === project.id);
  const isRunning = $derived(
    project.status === "running" || project.status === "starting",
  );
  const inlineError = $derived(projects.lastErrors[project.id] ?? null);

  async function run(op: "start" | "stop") {
    if (busy) return;
    busy = op;
    try {
      await safeInvoke(op === "start" ? "start_project" : "stop_project", {
        id: project.id,
      });
      projects.clearError(project.id);
    } catch (err) {
      projects.setError(project.id, err as CommandError);
    } finally {
      busy = null;
    }
  }

  async function openProjectUrl(e: MouseEvent) {
    e.stopPropagation();
    try {
      await safeInvoke("open_project", { id: project.id });
    } catch {
      /* toast already pushed */
    }
  }
</script>

<div
  role="button"
  tabindex="0"
  onclick={() => projects.select(project.id)}
  onkeydown={(e) => {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      projects.select(project.id);
    }
  }}
  data-selected={isSelected}
  class="group text-left bg-surface border border-border rounded-xl p-3
         flex flex-col gap-3 cursor-pointer transition-colors
         hover:bg-surface-2
         focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40
         data-[selected=true]:bg-accent/10
         data-[selected=true]:ring-1 data-[selected=true]:ring-inset
         data-[selected=true]:ring-accent/40"
>
  <header class="flex items-start gap-2.5 min-w-0">
    <ProjectAvatar id={project.id} name={project.name} size={36} />
    <div class="min-w-0 flex-1 leading-tight">
      <p class="text-[13.5px] font-semibold text-fg truncate">
        {project.name}
      </p>
      <p
        class="text-[11px] text-fg-subtle truncate inline-flex items-center gap-1.5"
      >
        <StackIcon type={project.type} size={11} />
        {typeLabel[project.type]}
      </p>
    </div>
    <div onclick={(e) => e.stopPropagation()} role="presentation">
      <ProjectRowMenu {project} />
    </div>
  </header>

  <button
    type="button"
    onclick={openProjectUrl}
    title="Open in browser"
    class="inline-flex items-center justify-between gap-1.5 px-2.5 py-1.5
           rounded-md bg-surface-2/60 hover:bg-surface-2
           text-[11.5px] text-accent hover:text-accent-hover
           border border-border/60 transition-colors w-full"
  >
    <span class="truncate font-mono">{project.url}</span>
    <Icon name="external-link" size={11} class="shrink-0 opacity-80" />
  </button>

  <footer class="flex items-center justify-between gap-2">
    <div class="flex items-center gap-2 min-w-0">
      <StatusPill status={project.status} />
      {#if project.port !== null}
        <span class="text-[11px] font-mono tabular-nums text-fg-subtle">
          :{project.port}
        </span>
      {/if}
    </div>
    {#if isRunning}
      <button
        type="button"
        onclick={(e) => {
          e.stopPropagation();
          void run("stop");
        }}
        disabled={!!busy}
        title="Stop project"
        aria-label="Stop {project.name}"
        class="inline-flex items-center justify-center w-8 h-8 rounded-md
               bg-status-crashed text-on-accent shadow-sm
               hover:brightness-110 active:brightness-95
               disabled:opacity-60 disabled:cursor-not-allowed transition"
      >
        <Icon name="square" size={12} />
      </button>
    {:else}
      <button
        type="button"
        onclick={(e) => {
          e.stopPropagation();
          void run("start");
        }}
        disabled={!!busy}
        title="Start project"
        aria-label="Start {project.name}"
        class="inline-flex items-center justify-center w-8 h-8 rounded-md
               bg-status-running text-on-accent shadow-sm
               hover:brightness-110 active:brightness-95
               disabled:opacity-60 disabled:cursor-not-allowed transition"
      >
        <Icon name="play" size={12} />
      </button>
    {/if}
  </footer>

  {#if inlineError}
    <div onclick={(e) => e.stopPropagation()} role="presentation">
      <ErrorEnvelope
        envelope={inlineError}
        onDismiss={() => projects.clearError(project.id)}
      />
    </div>
  {/if}
</div>
