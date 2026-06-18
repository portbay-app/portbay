<!--
  StatusCards — the four-up status row at the top of the dashboard.

  Refactored from the old infra-centric row (Caddy / HTTPS / Hosts Helper /
  Active) to a developer-centric one that answers "what's the state of my
  local environment?" at a glance:

    1. Projects        — running / total, plus a live activity pulse that
                          only appears while something is running
    2. Local Access     — HTTPS domains served / total, with the local-CA
                          trust state (the access signal, not a project count)
    3. Services        — bundled sidecar health (the /services surface)
    4. Local AI         — Ollama running-state + the loaded model, so the
                          AI page isn't the only place to check

  Each tile is a tall, calm rectangle rendered by the shared `StatusTile`
  atom — icon top-left, title + subtitle stacked, value content bottom-left,
  a meaningful flourish bottom-right (the activity pulse, a trust badge, a
  health badge, an AI status badge). The Projects pulse is the one real
  time-series (the device's CPU trace while projects run); the others show a
  value we actually have.
-->
<script lang="ts">
  import { onMount } from "svelte";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import Sparkline from "$lib/components/atoms/Sparkline.svelte";
  import StatusTile from "$lib/components/atoms/StatusTile.svelte";

  import { sidecars } from "$lib/stores/sidecars.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import { metrics } from "$lib/stores/metrics.svelte";
  import { ollamaService } from "$lib/stores/ollama.svelte";

  onMount(() => {
    sidecars.start();
    metrics.start();
    // ollamaService is started app-wide by the root layout — read-only here.
    return () => {
      sidecars.stop();
      metrics.stop();
    };
  });

  // --- Card 1: Projects ---
  const total = $derived(projects.value.length);
  const runningCount = $derived(
    projects.value.filter(
      (p) => p.status === "running" || p.status === "starting",
    ).length,
  );
  const stoppedCount = $derived(
    projects.value.filter((p) => p.status === "stopped").length,
  );
  const attentionCount = $derived(
    projects.value.filter(
      (p) =>
        p.status === "crashed" ||
        p.status === "port_conflict" ||
        p.status === "unhealthy",
    ).length,
  );

  // --- Card 2: Local Access ---
  // A "domain" is a project's hostname (1:1). The headline counts domains
  // served over HTTPS (httpsCount / total) — distinct from the Projects
  // card's project count. HTTPS is "trusted" only when the mkcert local CA
  // is installed; otherwise the cert exists but browsers warn.
  const httpsCount = $derived(
    projects.value.filter((p) => p.https).length,
  );
  const mkcertTrusted = $derived(
    sidecars.value.mkcertCa.status === "running",
  );
  const domainsAttention = $derived(
    projects.value.filter(
      (p) => p.status === "port_conflict" || p.status === "unhealthy",
    ).length,
  );

  // --- Card 3: Services ---
  // Health across the bundled sidecars — the same set the /services page
  // expands. "Healthy" = running; "failing" = installed but unreachable.
  const serviceList = $derived(Object.values(sidecars.value));
  const servicesTotal = $derived(serviceList.length);
  const servicesHealthy = $derived(
    serviceList.filter((s) => s.status === "running").length,
  );
  const servicesFailing = $derived(
    serviceList.filter((s) => s.status === "unreachable").length,
  );

  // --- Card 4: Local AI (Ollama) ---
  // At-a-glance "is my local AI up, and what's loaded?" so the model doesn't
  // have to be checked on the AI page. running + loaded come from the app-wide
  // ollamaService store (cheap probes); loaded[] is empty while stopped.
  const ollamaRunning = $derived(ollamaService.running);
  const loadedCount = $derived(ollamaService.loaded.length);
  const primaryModel = $derived(ollamaService.loaded[0]?.name ?? null);
  const extraLoaded = $derived(Math.max(0, loadedCount - 1));
</script>

<div
  class="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-3"
  role="status"
  aria-live="polite"
  aria-label="Local environment status"
>
  <!-- Card 1: Projects -->
  <StatusTile
    icon="package"
    iconClass="bg-status-running/10 text-status-running"
    title="Projects"
    subtitle="Running now"
  >
    <p class="text-[20px] font-semibold text-fg tabular-nums">
      {runningCount}<span class="text-fg-subtle font-normal text-[15px]">
        / {total}</span
      >
    </p>
    <p class="text-[11px] truncate">
      <span class="text-fg-subtle">{stoppedCount} stopped</span>
      {#if attentionCount > 0}
        <span class="text-status-unhealthy">
          · {attentionCount} need attention</span
        >
      {/if}
    </p>
    {#snippet flourish()}
      <!-- Live activity pulse — the device's CPU trace while projects run.
           Hidden entirely when nothing is running, so an idle dashboard
           shows just the count, no flat line. -->
      {#if runningCount > 0}
        <div
          class="w-20 h-8 shrink-0 text-status-running"
          aria-hidden="true"
        >
          <Sparkline
            data={metrics.cpuHistory}
            color="var(--color-status-running)"
            label="Activity while projects run"
          />
        </div>
      {/if}
    {/snippet}
  </StatusTile>

  <!-- Card 2: Local Access -->
  <StatusTile
    icon="globe"
    iconClass="bg-status-starting/10 text-status-starting"
    title="Local Access"
    subtitle="Domains &amp; URLs"
  >
    <p class="text-[20px] font-semibold text-fg tabular-nums">
      {httpsCount}<span class="text-fg-subtle font-normal text-[15px]">
        / {total} HTTPS</span
      >
    </p>
    <p class="text-[11px] truncate">
      {#if mkcertTrusted}
        <span class="text-status-running">CA trusted</span>
      {:else}
        <span class="text-status-unhealthy">CA not trusted</span>
      {/if}
      {#if domainsAttention > 0}
        <span class="text-status-unhealthy">
          · {domainsAttention} need attention</span
        >
      {/if}
    </p>
    {#snippet flourish()}
      <span
        class="inline-flex items-center justify-center w-10 h-10 rounded-full
               {mkcertTrusted
          ? 'bg-status-running/15 text-status-running'
          : 'bg-status-unhealthy/15 text-status-unhealthy'}"
        aria-hidden="true"
      >
        <Icon name={mkcertTrusted ? "lock" : "circle-alert"} size={18} />
      </span>
    {/snippet}
  </StatusTile>

  <!-- Card 3: Services -->
  <StatusTile
    icon="server"
    iconClass="bg-accent/10 text-accent"
    title="Services"
    subtitle="Bundled sidecars"
  >
    <p class="text-[20px] font-semibold text-fg tabular-nums">
      {servicesHealthy}<span class="text-fg-subtle font-normal text-[15px]">
        / {servicesTotal}</span
      >
    </p>
    <p class="text-[11px] truncate">
      {#if servicesFailing > 0}
        <span class="text-status-crashed">{servicesFailing} failing</span>
      {:else}
        <span class="text-status-running">healthy</span>
      {/if}
    </p>
    {#snippet flourish()}
      <span
        class="inline-flex items-center justify-center w-10 h-10 rounded-full
               {servicesFailing > 0
          ? 'bg-status-crashed/15 text-status-crashed'
          : 'bg-status-running/15 text-status-running'}"
        aria-hidden="true"
      >
        <Icon
          name={servicesFailing > 0 ? "circle-alert" : "activity"}
          size={18}
        />
      </span>
    {/snippet}
  </StatusTile>

  <!-- Card 4: Local AI -->
  <StatusTile
    icon="sparkles"
    iconClass="bg-accent/10 text-accent"
    title="Local AI"
    subtitle="Ollama server"
  >
    <p
      class="text-[20px] font-semibold tabular-nums {ollamaRunning
        ? 'text-fg'
        : 'text-fg-subtle'}"
    >
      {loadedCount}<span class="text-fg-subtle font-normal text-[15px]">
        {loadedCount === 1 ? "model" : "models"}</span
      >
    </p>
    <p class="text-[11px] truncate">
      {#if ollamaRunning}
        {#if primaryModel}
          <span class="text-status-running">{primaryModel}</span>
          {#if extraLoaded > 0}
            <span class="text-fg-subtle"> · +{extraLoaded} more</span>
          {/if}
        {:else}
          <span class="text-fg-subtle">running · no model loaded</span>
        {/if}
      {:else}
        <span class="text-fg-subtle">stopped</span>
      {/if}
    </p>
    {#snippet flourish()}
      <!-- Load line — the device CPU trace while the AI server runs; hidden
           when stopped, mirroring the Projects pulse. -->
      {#if ollamaRunning}
        <div class="w-20 h-8 shrink-0 text-accent" aria-hidden="true">
          <Sparkline
            data={metrics.cpuHistory}
            color="var(--color-accent)"
            label="Local AI load"
          />
        </div>
      {/if}
    {/snippet}
  </StatusTile>
</div>
