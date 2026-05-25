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
  import { startProject } from "$lib/actions/startProject";
  import { projects } from "$lib/stores/projects.svelte";
  import { dns } from "$lib/stores/dns.svelte";

  import type { CommandError } from "$lib/types/error";
  import type { ProjectView } from "$lib/types/projects";
  import {
    typeLabel,
    effectiveWebServer,
    webServerLabel,
  } from "$lib/types/projects";

  import ProjectRowMenu from "./ProjectRowMenu.svelte";

  interface Props {
    project: ProjectView;
  }
  let { project }: Props = $props();

  let busy = $state<"start" | "stop" | null>(null);

  const isSelected = $derived(projects.selectedId === project.id);
  const display = $derived(projects.displayStatusOf(project));
  // Web server fronting this project — PHP doc-root projects only; null
  // otherwise so we don't mislabel a Node app's edge proxy as its server.
  const server = $derived(effectiveWebServer(project));
  const showStop = $derived(
    display === "running" || display === "starting" || display === "stopping",
  );
  const inlineError = $derived(projects.lastErrors[project.id] ?? null);

  async function run(op: "start" | "stop") {
    if (busy) return;
    busy = op;
    // Optimistic flip on click — see ProjectRow for the full rationale.
    projects.beginTransition(project.id, op);
    try {
      if (op === "start") {
        await dns.ensureReady();
        const r = await startProject(project.id, project.name);
        if (r.kind === "declined") {
          projects.failTransition(project.id);
          return;
        }
        if (r.kind === "error") throw r.error;
      } else {
        await safeInvoke("stop_project", { id: project.id });
      }
      projects.clearError(project.id);
    } catch (err) {
      projects.failTransition(project.id);
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
        {#if server}
          <span
            class="px-1 py-0.5 rounded bg-surface-2 text-fg-subtle text-[10px]
                   border border-border/50"
            title="Served by {webServerLabel[server]}"
          >
            {webServerLabel[server]}
          </span>
        {/if}
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
      <StatusPill status={display} />
      {#if project.port !== null}
        <span class="text-[11px] font-mono tabular-nums text-fg-subtle">
          :{project.port}
        </span>
      {/if}
    </div>
    {#if showStop}
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
        {#if busy === "stop"}
          <Icon name="refresh-cw" size={12} class="animate-spin" />
        {:else}
          <Icon name="square" size={12} />
        {/if}
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
        {#if busy === "start"}
          <Icon name="refresh-cw" size={12} class="animate-spin" />
        {:else}
          <Icon name="play" size={12} />
        {/if}
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
