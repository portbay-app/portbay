<!--
  ProjectRowMenu — the per-row ellipsis dropdown.

  Renders actions that don't deserve a dedicated row button. The
  primary stop/start button sits next to this in the row; this menu
  is for everything else (Open URL, Reveal, View Logs, Restart,
  Edit, Remove).
-->
<script lang="ts">
  import { openUrl } from "@tauri-apps/plugin-opener";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import { safeInvoke } from "$lib/ipc";
  import { logViewer } from "$lib/stores/logViewer.svelte";
  import { projectDetailPanel } from "$lib/stores/detailPanel.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import { errorBus } from "$lib/stores/errors.svelte";
  import type { ProjectView } from "$lib/types/projects";

  interface Props {
    project: ProjectView;
  }
  let { project }: Props = $props();

  let open = $state<boolean>(false);
  let menuEl: HTMLDivElement | undefined = $state();
  let buttonEl: HTMLButtonElement | undefined = $state();

  function toggle(e: MouseEvent) {
    e.stopPropagation();
    open = !open;
  }

  function close() {
    open = false;
  }

  function onWindowClick(e: MouseEvent) {
    if (!open) return;
    const t = e.target as Node | null;
    if (
      menuEl &&
      buttonEl &&
      t &&
      !menuEl.contains(t) &&
      !buttonEl.contains(t)
    ) {
      close();
    }
  }

  function onWindowKey(e: KeyboardEvent) {
    if (open && e.key === "Escape") close();
  }

  async function openProjectUrl(e: MouseEvent) {
    e.stopPropagation();
    close();
    try {
      await safeInvoke("open_project", { id: project.id });
    } catch {
      /* toast already pushed */
    }
  }

  async function reveal(e: MouseEvent) {
    e.stopPropagation();
    close();
    try {
      await openUrl(`file://${project.path}`);
    } catch {
      /* opener pushes its own toast */
    }
  }

  function viewLogs(e: MouseEvent) {
    e.stopPropagation();
    close();
    logViewer.show(project.id);
  }

  function editProject(e: MouseEvent) {
    e.stopPropagation();
    close();
    projectDetailPanel.show(project.id);
  }

  async function restart(e: MouseEvent) {
    e.stopPropagation();
    close();
    try {
      await safeInvoke("restart_project", { id: project.id });
    } catch {
      /* toast already pushed */
    }
  }

  async function remove(e: MouseEvent) {
    e.stopPropagation();
    close();
    try {
      await safeInvoke("remove_project", { id: project.id });
      await projects.refresh();
      errorBus.push({
        code: "REMOVE_OK",
        whatHappened: `${project.name} removed.`,
        whyItMatters: "Registry entry, cert, and hosts entry were cleaned up.",
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });
    } catch {
      /* toast already pushed */
    }
  }
</script>

<svelte:window onclick={onWindowClick} onkeydown={onWindowKey} />

<div class="relative inline-block">
  <button
    bind:this={buttonEl}
    type="button"
    onclick={toggle}
    aria-haspopup="menu"
    aria-expanded={open}
    title="More actions"
    aria-label="More actions for {project.name}"
    class="inline-flex items-center justify-center w-7 h-7 rounded-md
           text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
  >
    <Icon name="more-horizontal" size={14} />
  </button>

  {#if open}
    <div
      bind:this={menuEl}
      role="menu"
      class="absolute right-0 top-8 z-30 w-48 py-1
             rounded-lg border border-border bg-surface shadow-2xl"
    >
      <button
        type="button"
        onclick={openProjectUrl}
        class="w-full flex items-center gap-2 px-3 py-1.5 text-[12px]
               text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
        role="menuitem"
      >
        <Icon name="globe" size={12} /> Open URL
      </button>
      <button
        type="button"
        onclick={reveal}
        class="w-full flex items-center gap-2 px-3 py-1.5 text-[12px]
               text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
        role="menuitem"
      >
        <Icon name="folder" size={12} /> Reveal in Finder
      </button>
      <button
        type="button"
        onclick={viewLogs}
        class="w-full flex items-center gap-2 px-3 py-1.5 text-[12px]
               text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
        role="menuitem"
      >
        <Icon name="file-text" size={12} /> View logs
      </button>
      <button
        type="button"
        onclick={restart}
        class="w-full flex items-center gap-2 px-3 py-1.5 text-[12px]
               text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
        role="menuitem"
      >
        <Icon name="rotate-cw" size={12} /> Restart
      </button>
      <div class="my-1 border-t border-border/70"></div>
      <button
        type="button"
        onclick={editProject}
        class="w-full flex items-center gap-2 px-3 py-1.5 text-[12px]
               text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
        role="menuitem"
      >
        <Icon name="pencil" size={12} /> Edit project…
      </button>
      <button
        type="button"
        onclick={remove}
        class="w-full flex items-center gap-2 px-3 py-1.5 text-[12px]
               text-status-crashed hover:bg-status-crashed/10 transition-colors"
        role="menuitem"
      >
        <Icon name="x" size={12} /> Remove
      </button>
    </div>
  {/if}
</div>
