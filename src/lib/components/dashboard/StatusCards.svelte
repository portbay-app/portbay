<!--
  StatusCards — the four-up status row at the top of the dashboard.

  Order matches the design reference:
    1. Caddy        (reverse proxy)        — port + sparkline
    2. HTTPS        (mkcert local CA)      — trust state + check icon
    3. Hosts Helper (managed /etc/hosts)   — entry count + bar
    4. Active       (running projects)     — count + CPU sparkline

  Compared with the old SidecarRow (six small cards):
    - Removes processCompose, dnsmasq, mailpit from the dashboard.
      Those still surface in the sidebar status pill and on /services.
    - Promotes "Active Processes" (projects-derived, not a sidecar)
      because that's the metric a developer hits the dashboard for.

  Each card is a tall, calm rectangle — icon top-left, title +
  subtitle stacked, status content centered-bottom-left, visual
  flourish bottom-right.
-->
<script lang="ts">
  import { onMount } from "svelte";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import Sparkline from "$lib/components/atoms/Sparkline.svelte";
  import StatusPill from "$lib/components/atoms/StatusPill.svelte";

  import { sidecars } from "$lib/stores/sidecars.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import { metrics } from "$lib/stores/metrics.svelte";

  import type { PortbayStatus } from "$lib/types/status";

  onMount(() => {
    sidecars.start();
    return () => sidecars.stop();
  });

  // --- Card 1: Caddy ---
  const caddyPill = $derived.by<PortbayStatus>(() => {
    switch (sidecars.value.caddy.status) {
      case "running":
        return "running";
      case "stopped":
        return "stopped";
      case "not_installed":
        return "port_conflict";
      case "unreachable":
        return "crashed";
    }
  });

  const caddyPort = $derived.by<string>(() => {
    const m = sidecars.value.caddy.detail?.match(/(\d+)/);
    return m ? m[1] : "—";
  });

  // --- Card 2: HTTPS (mkcert CA) ---
  const mkcertTrusted = $derived(
    sidecars.value.mkcertCa.status === "running",
  );
  const mkcertLabel = $derived.by(() => {
    switch (sidecars.value.mkcertCa.status) {
      case "running":
        return "Trusted";
      case "stopped":
        return "Not Installed";
      case "not_installed":
        return "Missing";
      case "unreachable":
        return "Error";
    }
  });

  // --- Card 3: Hosts Helper ---
  const hostsEntries = $derived.by<number | null>(() => {
    const detail = sidecars.value.hostsHelper.detail;
    if (!detail) return null;
    const m = detail.match(/(\d+)/);
    return m ? Number(m[1]) : null;
  });

  const hostsPill = $derived.by<PortbayStatus>(() => {
    switch (sidecars.value.hostsHelper.status) {
      case "running":
        return "running";
      case "stopped":
        return "stopped";
      case "not_installed":
        return "port_conflict";
      case "unreachable":
        return "crashed";
    }
  });

  // --- Card 4: Active Processes ---
  const activeCount = $derived(
    projects.value.filter(
      (p) => p.status === "running" || p.status === "starting",
    ).length,
  );

  // The CPU history (60 samples × 2 min) is reused as the sparkline
  // data source for cards 1 and 4. Caddy's spark is informational —
  // we don't actually meter Caddy's per-process CPU, but the system
  // pulse is a faithful visual indicator that "the app is alive."
</script>

<div class="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-3">
  <!-- Card 1: Caddy -->
  <div
    class="bg-surface border border-border rounded-2xl p-4
           flex flex-col gap-3 min-h-[112px]"
  >
    <div class="flex items-start justify-between gap-2">
      <div class="flex items-center gap-2.5 min-w-0">
        <span
          class="inline-flex items-center justify-center w-8 h-8 rounded-lg
                 bg-accent/10 text-accent shrink-0"
        >
          <Icon name="layers" size={15} />
        </span>
        <div class="min-w-0 leading-tight">
          <p class="text-[13px] font-semibold text-fg truncate">Caddy</p>
          <p class="text-[11px] text-fg-subtle truncate">Reverse Proxy</p>
        </div>
      </div>
    </div>
    <div class="flex items-end justify-between gap-2">
      <div class="leading-tight">
        <p class="text-[20px] font-semibold text-fg tabular-nums">
          {caddyPort}
        </p>
        <StatusPill status={caddyPill} />
      </div>
      <div class="w-20 h-8 shrink-0 text-accent">
        <Sparkline
          data={metrics.cpuHistory}
          color="var(--color-accent)"
          label="Caddy activity"
        />
      </div>
    </div>
  </div>

  <!-- Card 2: HTTPS / mkcert CA -->
  <div
    class="bg-surface border border-border rounded-2xl p-4
           flex flex-col gap-3 min-h-[112px]"
  >
    <div class="flex items-start justify-between gap-2">
      <div class="flex items-center gap-2.5 min-w-0">
        <span
          class="inline-flex items-center justify-center w-8 h-8 rounded-lg
                 bg-status-running/10 text-status-running shrink-0"
        >
          <Icon name="lock" size={15} />
        </span>
        <div class="min-w-0 leading-tight">
          <p class="text-[13px] font-semibold text-fg truncate">HTTPS</p>
          <p class="text-[11px] text-fg-subtle truncate">Local CA</p>
        </div>
      </div>
    </div>
    <div class="flex items-end justify-between gap-2">
      <div class="leading-tight">
        <p
          class="text-[15px] font-semibold tabular-nums {mkcertTrusted
            ? 'text-status-running'
            : 'text-status-unhealthy'}"
        >
          {mkcertLabel}
        </p>
        <p class="text-[11px] text-fg-subtle">Certificate Authority</p>
      </div>
      <span
        class="inline-flex items-center justify-center w-10 h-10 rounded-full
               {mkcertTrusted
          ? 'bg-status-running/15 text-status-running'
          : 'bg-status-unhealthy/15 text-status-unhealthy'}"
        aria-hidden="true"
      >
        <Icon
          name={mkcertTrusted ? "check" : "circle-alert"}
          size={18}
        />
      </span>
    </div>
  </div>

  <!-- Card 3: Hosts Helper -->
  <div
    class="bg-surface border border-border rounded-2xl p-4
           flex flex-col gap-3 min-h-[112px]"
  >
    <div class="flex items-start justify-between gap-2">
      <div class="flex items-center gap-2.5 min-w-0">
        <span
          class="inline-flex items-center justify-center w-8 h-8 rounded-lg
                 bg-fg-muted/10 text-fg-muted shrink-0"
        >
          <Icon name="users" size={15} />
        </span>
        <div class="min-w-0 leading-tight">
          <p class="text-[13px] font-semibold text-fg truncate">
            Hosts Helper
          </p>
          <p class="text-[11px] text-fg-subtle truncate">/etc/hosts entries</p>
        </div>
      </div>
    </div>
    <div class="flex items-end justify-between gap-2">
      <div class="leading-tight">
        <p class="text-[20px] font-semibold text-fg tabular-nums">
          {hostsEntries ?? "—"}
        </p>
        <StatusPill status={hostsPill} />
      </div>
      <div class="w-20 h-8 shrink-0 text-fg-muted flex items-end gap-0.5">
        <!-- Bar-chart impression: faux histogram of the entry count
             distributed into seven bars. Reads as data without
             implying we actually meter time-series here. -->
        {#each [0.35, 0.5, 0.4, 0.65, 0.55, 0.8, 0.6] as h, i (i)}
          <span
            class="flex-1 bg-fg-muted/40 rounded-sm"
            style:height="{h * 100}%"
          ></span>
        {/each}
      </div>
    </div>
  </div>

  <!-- Card 4: Active Processes -->
  <div
    class="bg-surface border border-border rounded-2xl p-4
           flex flex-col gap-3 min-h-[112px]"
  >
    <div class="flex items-start justify-between gap-2">
      <div class="flex items-center gap-2.5 min-w-0">
        <span
          class="inline-flex items-center justify-center w-8 h-8 rounded-lg
                 bg-status-running/10 text-status-running shrink-0"
        >
          <Icon name="activity" size={15} />
        </span>
        <div class="min-w-0 leading-tight">
          <p class="text-[13px] font-semibold text-fg truncate">
            Active Processes
          </p>
          <p class="text-[11px] text-fg-subtle truncate">
            Currently running
          </p>
        </div>
      </div>
    </div>
    <div class="flex items-end justify-between gap-2">
      <div class="leading-tight">
        <p class="text-[20px] font-semibold text-fg tabular-nums">
          {activeCount}
        </p>
        <p
          class="text-[11px] {activeCount > 0
            ? 'text-status-running'
            : 'text-fg-subtle'}"
        >
          {activeCount === 1 ? "project" : "projects"}
        </p>
      </div>
      <div class="w-20 h-8 shrink-0 text-status-running">
        <Sparkline
          data={metrics.cpuHistory}
          color="var(--color-status-running)"
          label="System CPU"
        />
      </div>
    </div>
  </div>
</div>
