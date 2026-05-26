<!--
  RightRail — slot container for the right column.

  Behaviour after the dashboard redesign:
    - Default content is the ProjectDetailRail, which renders the
      selected project's summary (avatar, status, URL, metadata,
      quick actions, recent activity, checks) and shows an empty
      placeholder when nothing is selected.
    - The previous default (MetricsRail with CPU + Memory cards)
      moved into the sidebar footer; the right rail is no longer
      a system-stats surface.
    - Hidden when density is `compact`, freeing horizontal space
      for the project table.

  Callers can still pass children — `/groups/[id]` overrides this
  slot with its own batch-action panel.
-->
<script lang="ts">
  import type { Snippet } from "svelte";
  import { ProjectDetailRail } from "$lib/components/projects";
  import { density } from "$lib/stores/density.svelte";

  let { children }: { children?: Snippet } = $props();
</script>

<aside
  class="h-full overflow-y-auto bg-surface border-l border-border p-4"
  class:hidden={density.value === "compact"}
  aria-label="Status panel"
>
  {#if children}
    {@render children()}
  {:else}
    <ProjectDetailRail />
  {/if}
</aside>
