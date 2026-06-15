<!--
  Projects EmptyState — the registry's zero / filtered-to-zero states.

  Thin wrapper over the shared `atoms/EmptyState`: this keeps the
  projects-specific copy and the "Add project" wizard CTA, but the layout is the
  shared atom so every empty surface looks the same. See `docs/UX_DESIGN.md` §5.5
  for the onboarding emphasis.
-->
<script lang="ts">
  import { EmptyState } from "$lib/components/atoms";
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

{#if variant === "registry-empty"}
  <EmptyState
    icon="folder"
    title="No projects yet"
    description="Add a project from a folder to start managing its dev server, hostname, and certificate."
  >
    {#snippet children()}
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
    {/snippet}
  </EmptyState>
{:else}
  <EmptyState icon="search" title={`No projects match “${query}”`} />
{/if}
