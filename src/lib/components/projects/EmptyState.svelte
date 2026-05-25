<!--
  EmptyState — shown when the registry has no projects yet.

  Two variants: filtered (search returned zero) and registry-empty (no
  projects at all). The latter mirrors `docs/UX_DESIGN.md` §5.5's
  onboarding emphasis — "I have a project" call to action sits front and
  centre.
-->
<script lang="ts">
  import Icon from "$lib/components/atoms/Icon.svelte";
  import { addProjectWizard } from "$lib/stores/wizard.svelte";

  interface Props {
    variant: "registry-empty" | "filtered";
    query?: string;
  }
  let { variant, query }: Props = $props();

  function openAddProject() {
    addProjectWizard.requestAdd();
  }
</script>

<div class="flex flex-col items-center justify-center text-center py-12 gap-3">
  {#if variant === "registry-empty"}
    <Icon name="folder" size={32} class="text-fg-subtle" />
    <div class="space-y-1">
      <p class="text-sm font-medium text-fg">No projects yet</p>
      <p class="text-xs text-fg-muted max-w-xs">
        Add a project from a folder to start managing its dev server,
        hostname, and certificate.
      </p>
    </div>
    <button
      type="button"
      onclick={openAddProject}
      class="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md text-sm
             text-status-running border border-status-running/40
             hover:bg-status-running/10 hover:border-status-running/60
             transition-colors"
    >
      <Icon name="plus" size={14} />
      Add project
    </button>
  {:else}
    <Icon name="search" size={24} class="text-fg-subtle" />
    <p class="text-sm text-fg-muted">
      No projects match <span class="text-fg font-medium">"{query}"</span>.
    </p>
  {/if}
</div>
