<!--
  Projects route (/) — sidecar health row + the project list table.
-->
<script lang="ts">
  import { SidecarRow } from "$lib/components/sidecars";
  import { ProjectsTable } from "$lib/components/projects";
</script>

<div class="p-6 space-y-4">
  <SidecarRow />
  <ProjectsTable />
</div>
