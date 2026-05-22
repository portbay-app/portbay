<!--
  Logs index — list of registered projects, each clickable to open
  the full log viewer modal.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { DashboardCard, Icon, StatusDot } from "$lib/components/atoms";
  import { logViewer } from "$lib/stores/logViewer.svelte";
  import { projects } from "$lib/stores/projects.svelte";
</script>

<div class="p-6 space-y-4">
  <DashboardCard title="Logs" flush>
    {#if projects.value.length === 0}
      <p class="text-sm text-fg-muted py-4 text-center">
        No registered projects. Add one to see its logs here.
      </p>
    {:else}
      <ul class="divide-y divide-border -mx-4">
        {#each projects.value as project (project.id)}
          <li>
            <button
              type="button"
              onclick={() => logViewer.show(project.id)}
              class="w-full flex items-center gap-3 px-4 py-2.5 text-left hover:bg-surface-2 transition-colors"
            >
              <StatusDot status={project.status} size="md" />
              <span class="font-medium text-fg flex-1">{project.name}</span>
              <span class="text-xs text-fg-subtle font-mono">{project.hostname}</span>
              <Icon name="external-link" size={12} class="text-fg-subtle" />
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </DashboardCard>
</div>
