<!--
  ProjectRowMenu — the per-row ellipsis dropdown.

  Renders actions that don't deserve a dedicated row button. The
  primary stop/start button sits next to this in the row; this menu
  is for everything else (Open URL, Reveal, View Logs, Restart,
  Edit, Remove).
-->
<script lang="ts">
  import { revealItemInDir } from "@tauri-apps/plugin-opener";

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
  // The menu is rendered with `position: fixed` so it can escape the
  // table card's `overflow-hidden`. We anchor it to the trigger button's
  // bounding rect and recompute on each open.
  const MENU_WIDTH = 192; // matches w-48
  let menuTop = $state<number>(0);
  let menuLeft = $state<number>(0);

  function recomputePosition() {
    if (!buttonEl) return;
    const r = buttonEl.getBoundingClientRect();
    menuTop = r.bottom + 4;
    menuLeft = Math.max(8, r.right - MENU_WIDTH);
  }

  function toggle(e: MouseEvent) {
    e.stopPropagation();
    if (!open) recomputePosition();
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

  // Close on scroll/resize — the fixed-position menu wouldn't otherwise
  // follow its anchor, and a stale floating menu is worse than a closed one.
  function onScrollOrResize() {
    if (open) close();
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
      await revealItemInDir(project.path);
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

<svelte:window
  onclick={onWindowClick}
  onkeydown={onWindowKey}
  onscroll={onScrollOrResize}
  onresize={onScrollOrResize}
/>

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
      style:position="fixed"
      style:top="{menuTop}px"
      style:left="{menuLeft}px"
      style:width="{MENU_WIDTH}px"
      class="z-50 py-1 rounded-lg border border-border bg-surface shadow-2xl"
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
