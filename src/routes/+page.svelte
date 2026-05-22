<!--
  Projects route (/) — sidecar health row + project list placeholder.
  Card #6 swaps the placeholder for the real table.
-->
<script lang="ts">
  import { DashboardCard } from "$lib/components/atoms";
  import { SidecarRow } from "$lib/components/sidecars";
</script>

<div class="p-6 space-y-4">
  <SidecarRow />

  <DashboardCard title="Websites" flush>
    <p class="text-sm text-fg-muted">
      The projects table lands in card #6. Once wired, this surface shows
      every registered project with its live status, hostname, framework,
      and the per-row action cluster (open / folder / edit / start / stop /
      restart).
    </p>
  </DashboardCard>
</div>
