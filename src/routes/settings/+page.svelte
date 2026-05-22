<!--
  Settings — Phase 2 deliverable is intentionally minimal: density toggle
  and the version line. Full settings (registry path override, sidecar
  versions, log location, etc.) come in Phase 3.
-->
<script lang="ts">
  import { DashboardCard } from "$lib/components/atoms";
  import { density, type Density } from "$lib/stores/density.svelte";
  import { theme, type Theme } from "$lib/stores/theme.svelte";
  import { safeInvoke } from "$lib/ipc";

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

  const themeOptions: { value: Theme; label: string; detail: string }[] = [
    {
      value: "dark",
      label: "Dark",
      detail: "Default PortBay theme for local-dev work sessions.",
    },
    {
      value: "light",
      label: "Light",
      detail: "Higher ambient-light theme with the same status taxonomy.",
    },
  ];

  async function triggerDemoError() {
    // Calls a real command with input that's guaranteed to fail. The Rust
    // side returns AppError::NotFound, which round-trips through the
    // CommandError envelope and lands as a toast in the bottom-right.
    try {
      await safeInvoke("start_project", { id: "this-project-does-not-exist" });
    } catch {
      // safeInvoke already pushed the toast.
    }
  }
</script>

<div class="p-6 max-w-2xl space-y-4">
  <DashboardCard title="Theme" flush>
    <div class="space-y-3">
      {#each themeOptions as opt (opt.value)}
        <label
          class="flex items-start gap-3 p-3 rounded-md border cursor-pointer transition-colors
                 {theme.value === opt.value
            ? 'border-accent/60 bg-accent/8'
            : 'border-border hover:border-border-strong'}"
        >
          <input
            type="radio"
            name="theme"
            value={opt.value}
            checked={theme.value === opt.value}
            onchange={() => theme.set(opt.value)}
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

  <DashboardCard title="Diagnostics" flush>
    <p class="text-xs text-fg-muted mb-3">
      Smoke-test the error envelope round-trip — calls a command with a
      bogus id; the toast should appear in the bottom-right with a
      "system" error envelope.
    </p>
    <button
      type="button"
      onclick={triggerDemoError}
      class="text-xs px-3 py-1.5 rounded-md border border-border text-fg-muted hover:text-fg hover:border-border-strong transition-colors"
    >
      Trigger demo error
    </button>
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
