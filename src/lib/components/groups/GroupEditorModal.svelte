<!--
  GroupEditorModal — slide-over for creating or editing a project group.

  Mounted at the layout root (same pattern as AddProjectWizard). Open
  via `groupEditor.create()` or `groupEditor.edit(view)`. Saves through
  the groups store; closes on success.
-->
<script lang="ts">
  import Icon from "$lib/components/atoms/Icon.svelte";
  import { ErrorEnvelope } from "$lib/components/errors";
  import { errorBus } from "$lib/stores/errors.svelte";
  import { groupEditor } from "$lib/stores/groupEditor.svelte";
  import { groups } from "$lib/stores/groups.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import type { CommandError } from "$lib/types/error";

  let name = $state<string>("");
  let selectedIds = $state<string[]>([]);
  let submitting = $state<boolean>(false);
  let error = $state<CommandError | null>(null);

  // Re-initialise form when the modal opens or switches mode.
  $effect(() => {
    const m = groupEditor.mode;
    if (m.kind === "closed") return;
    if (m.kind === "create") {
      name = "";
      selectedIds = [];
    } else {
      name = m.group.name;
      selectedIds = [...m.group.projectIds];
    }
    error = null;
    submitting = false;
  });

  function toggle(id: string) {
    if (selectedIds.includes(id)) {
      selectedIds = selectedIds.filter((x) => x !== id);
    } else {
      selectedIds = [...selectedIds, id];
    }
  }

  const canSubmit = $derived(name.trim().length > 0);

  async function save() {
    if (!canSubmit) return;
    submitting = true;
    error = null;
    try {
      const m = groupEditor.mode;
      if (m.kind === "create") {
        await groups.add({
          name: name.trim(),
          projectIds: selectedIds,
        });
        errorBus.push({
          code: "GROUP_CREATED",
          whatHappened: `Group "${name.trim()}" created with ${selectedIds.length} project${selectedIds.length === 1 ? "" : "s"}.`,
          whyItMatters: "Open it from the sidebar to start the whole group at once.",
          whoCausedIt: "system",
          actions: [],
        });
      } else if (m.kind === "edit") {
        await groups.update(m.group.id, {
          name: name.trim(),
          projectIds: selectedIds,
        });
        errorBus.push({
          code: "GROUP_UPDATED",
          whatHappened: `Group "${name.trim()}" updated.`,
          whyItMatters: "Member changes apply on the next group action.",
          whoCausedIt: "system",
          severity: "success",
          actions: [],
        });
      }
      groupEditor.close();
    } catch (e) {
      error = e as CommandError;
    } finally {
      submitting = false;
    }
  }

  function cancel() {
    groupEditor.close();
  }

  /** Sorted projects, with currently-selected ones bubbled to the top
   *  so the user can see what's in the group at a glance. */
  const sortedProjects = $derived.by(() => {
    const sel = new Set(selectedIds);
    return [...projects.value].sort((a, b) => {
      const aSel = sel.has(a.id);
      const bSel = sel.has(b.id);
      if (aSel !== bSel) return aSel ? -1 : 1;
      return a.name.localeCompare(b.name);
    });
  });
</script>

{#if groupEditor.isOpen}
  <!-- In-layout right-side panel (rendered into the grid rail by the root
       layout). Escape + the header close button dismiss it. -->
  <aside
    class="h-full w-full bg-surface border-l border-border flex flex-col"
    aria-label={groupEditor.mode.kind === "edit" ? "Edit group" : "New group"}
  >
    <header
      class="shrink-0 px-5 py-4 border-b border-border flex items-center justify-between"
    >
      <div class="flex items-center gap-2">
        <Icon name="folder" size={16} />
        <h2 class="text-base font-semibold tracking-tight">
          {groupEditor.mode.kind === "edit" ? "Edit group" : "New group"}
        </h2>
      </div>
      <button
        type="button"
        onclick={cancel}
        title="Close"
        aria-label="Close"
        class="p-1 rounded-md text-fg-subtle hover:text-fg hover:bg-surface-2 transition-colors"
      >
        <Icon name="x" size={14} />
      </button>
    </header>

    <div class="flex-1 min-h-0 overflow-y-auto px-5 py-4 space-y-5">
      <section class="space-y-2">
        <label for="group-name" class="text-xs uppercase tracking-wide text-fg-subtle">
          Group name
        </label>
        <input
          id="group-name"
          type="text"
          bind:value={name}
          placeholder="Enter group name"
          spellcheck="false"
          class="w-full px-3 py-2 rounded-md bg-bg border border-border
                 focus:border-accent/60 outline-none text-fg"
        />
        {#if groupEditor.mode.kind === "create"}
          <p class="text-[11px] text-fg-subtle">
            The group's id is derived from the name — spaces become hyphens
            and uppercase becomes lowercase.
          </p>
        {/if}
      </section>

      <section class="space-y-2">
        <div class="flex items-center justify-between">
          <span class="text-xs uppercase tracking-wide text-fg-subtle">
            Members
          </span>
          <span class="text-[10px] text-fg-subtle">
            {selectedIds.length} of {projects.value.length}
          </span>
        </div>
        {#if projects.value.length === 0}
          <p class="text-sm text-fg-muted py-4 text-center">
            No registered projects yet. Add a project first, then come back
            to group it.
          </p>
        {:else}
          <div
            class="rounded-md border border-border bg-surface divide-y divide-border max-h-[50vh] overflow-y-auto"
          >
            {#each sortedProjects as project (project.id)}
              {@const on = selectedIds.includes(project.id)}
              <label
                class="flex items-center gap-3 px-3 py-2 cursor-pointer hover:bg-surface-2 transition-colors"
              >
                <input
                  type="checkbox"
                  checked={on}
                  onchange={() => toggle(project.id)}
                  class="accent-accent"
                />
                <div class="flex-1 min-w-0">
                  <div class="text-sm font-medium truncate">{project.name}</div>
                  <div class="text-[11px] text-fg-subtle font-mono truncate">
                    {project.hostname}
                  </div>
                </div>
              </label>
            {/each}
          </div>
        {/if}
      </section>

      {#if error}
        <ErrorEnvelope envelope={error} tone="inline" />
      {/if}
    </div>

    <footer
      class="shrink-0 px-5 py-3 border-t border-border flex items-center justify-end gap-2"
    >
      <button
        type="button"
        onclick={cancel}
        class="px-3 py-1.5 text-xs rounded-md text-fg-muted
               hover:text-fg hover:bg-surface-2 transition-colors"
      >
        Cancel
      </button>
      <button
        type="button"
        onclick={save}
        disabled={!canSubmit || submitting}
        class="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-md
               text-accent border border-accent/40 hover:bg-accent/10
               disabled:opacity-50 transition-colors"
      >
        {#if submitting}
          <Icon name="refresh-cw" size={11} class="animate-spin" /> Saving…
        {:else}
          <Icon name="check" size={11} />
          {groupEditor.mode.kind === "edit" ? "Save changes" : "Create group"}
        {/if}
      </button>
    </footer>
  </aside>
{/if}
