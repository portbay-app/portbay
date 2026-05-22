<!--
  Settings — Phase 2 deliverable is intentionally minimal: density toggle
  and the version line. Full settings (registry path override, sidecar
  versions, log location, etc.) come in Phase 3.
-->
<script lang="ts">
  import { DashboardCard } from "$lib/components/atoms";
  import { density, type Density } from "$lib/stores/density";

  const densityOptions: { value: Density; label: string; detail: string }[] = [
    {
      value: "comfortable",
      label: "Comfortable",
      detail: "Spacious rows, friendly empty states. Recommended for new users.",
    },
    {
      value: "compact",
      label: "Compact",
      detail:
        "Tighter rows, icon-only status, no right-rail. Optimized for power users.",
    },
  ];
</script>

<div class="p-6 max-w-2xl space-y-4">
  <DashboardCard title="Density" flush>
    <div class="space-y-3">
      {#each densityOptions as opt (opt.value)}
        <label
          class="flex items-start gap-3 p-3 rounded-md border cursor-pointer transition-colors
                 {density.value === opt.value
            ? 'border-accent/60 bg-accent/8'
            : 'border-border hover:border-border-strong'}"
        >
          <input
            type="radio"
            name="density"
            value={opt.value}
            checked={density.value === opt.value}
            onchange={() => density.set(opt.value)}
            class="mt-1 accent-accent"
          />
          <div>
            <div class="text-sm font-medium text-fg">{opt.label}</div>
            <div class="text-xs text-fg-muted">{opt.detail}</div>
          </div>
        </label>
      {/each}
    </div>
  </DashboardCard>

  <DashboardCard title="About" flush>
    <dl class="grid grid-cols-[auto,1fr] gap-x-6 gap-y-2 text-xs">
      <dt class="text-fg-muted">Version</dt>
      <dd class="text-fg font-mono">0.1.0</dd>
      <dt class="text-fg-muted">Phase</dt>
      <dd class="text-fg">2 (GUI MVP, in progress)</dd>
      <dt class="text-fg-muted">Source</dt>
      <dd>
        <a
          href="https://github.com/portbay-app/portbay"
          class="text-accent hover:text-accent-hover"
          target="_blank"
          rel="noopener noreferrer"
        >
          github.com/portbay-app/portbay
        </a>
      </dd>
    </dl>
  </DashboardCard>
</div>
