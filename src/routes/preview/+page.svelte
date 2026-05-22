<!--
  Atoms preview — keeps the card-#2 demo content reachable for visual QA
  while the shell takes over the home route. Not linked from the sidebar
  on purpose; navigate manually via /preview when verifying atoms.
-->
<script lang="ts">
  import {
    Badge,
    DashboardCard,
    Icon,
    StatusDot,
    StatusPill,
    type BadgeTone,
    type IconName,
  } from "$lib/components/atoms";
  import { density } from "$lib/stores/density";
  import { ALL_STATUSES, statusLabel } from "$lib/types/status";

  const badgeTones: BadgeTone[] = [
    "neutral",
    "info",
    "success",
    "warning",
    "danger",
  ];

  const iconShowcase: IconName[] = [
    "play",
    "stop-circle",
    "rotate-cw",
    "external-link",
    "folder",
    "pencil",
    "globe",
    "settings",
    "search",
    "plus",
    "refresh-cw",
    "terminal",
  ];
</script>

<div class="p-6 max-w-5xl space-y-6">
  <header class="flex items-baseline justify-between">
    <div>
      <h2 class="text-lg font-semibold tracking-tight">Atoms preview</h2>
      <p class="text-sm text-fg-muted">
        Manual visual QA for the lifted Lerd primitives.
      </p>
    </div>
    <button
      onclick={() => density.toggle()}
      class="text-xs px-3 py-1.5 rounded-md border border-border text-fg-muted hover:text-fg hover:border-border-strong transition-colors"
    >
      Density: {density.value}
    </button>
  </header>

  <DashboardCard title="StatusDot" flush>
    <div class="flex flex-wrap items-center gap-6">
      {#each ALL_STATUSES as status (status)}
        <div class="flex items-center gap-2 text-xs text-fg-muted">
          <StatusDot {status} size="md" />
          <span>{statusLabel[status]}</span>
        </div>
      {/each}
    </div>
  </DashboardCard>

  <DashboardCard title="StatusPill" flush>
    <div class="flex flex-wrap items-center gap-2">
      {#each ALL_STATUSES as status (status)}
        <StatusPill {status} />
      {/each}
    </div>
  </DashboardCard>

  <DashboardCard title="Badge" flush>
    <div class="flex flex-wrap items-center gap-2">
      {#each badgeTones as tone (tone)}
        <Badge {tone}>{tone}</Badge>
      {/each}
    </div>
  </DashboardCard>

  <DashboardCard title="Icon" flush>
    <div class="flex flex-wrap items-center gap-3 text-fg-muted">
      {#each iconShowcase as name (name)}
        <div
          class="flex items-center gap-1.5 px-2 py-1 border border-border rounded-md"
        >
          <Icon {name} />
          <span class="text-xs">{name}</span>
        </div>
      {/each}
    </div>
  </DashboardCard>

  <DashboardCard title="Process Compose" tone="critical">
    {#snippet badge()}
      <StatusPill status="crashed" />
    {/snippet}
    {#snippet footer()}
      <span class="text-xs text-fg-muted">Exit code 1 · last run 12s ago</span>
    {/snippet}
    <p class="text-sm text-fg-muted">
      Tone-as-card-accent demonstration. Critical → red left border. Used by
      the sidecar health row when a daemon is down.
    </p>
  </DashboardCard>
</div>
