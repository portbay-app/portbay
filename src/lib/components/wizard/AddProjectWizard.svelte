<!--
  AddProjectWizard — the global right-side rail host for the add-project form.

  Thin wrapper: it supplies the rail chrome (full-height surface with a left
  border, rendered into the root layout's grid rail) and binds the shared
  `AddProjectForm` to the global open/close store. The Domains page mounts the
  same `AddProjectForm` inline instead of going through this rail.
-->
<script lang="ts">
  import AddProjectForm from "./AddProjectForm.svelte";
  import { addProjectWizard } from "$lib/stores/wizard.svelte";
</script>

{#if addProjectWizard.isOpen}
  <aside
    class="h-full w-full min-h-0 overflow-hidden bg-surface border-l border-border flex flex-col"
    aria-label="Add Project"
  >
    <AddProjectForm
      mode={addProjectWizard.mode}
      onClose={() => addProjectWizard.hide()}
    />
  </aside>
{/if}
