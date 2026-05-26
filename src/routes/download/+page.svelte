<!--
  /download — marketing download page for the web demo (try.portbay.app).
  Detects the visitor's OS, highlights it, and offers the matching build.
  macOS is the only built target today; Windows + Linux read as "coming soon".
-->
<script lang="ts">
  import { onMount } from "svelte";

  import { detectOS, PLATFORMS, type DesktopOS } from "$lib/platform";
  import PlatformIcon from "$lib/components/marketing/PlatformIcon.svelte";

  let detected = $state<DesktopOS>("unknown");
  onMount(() => {
    detected = detectOS();
  });

  const order = ["macos", "windows", "linux"] as const;
</script>

<svelte:head>
  <title>Download PortBay</title>
</svelte:head>

<div class="h-full overflow-y-auto">
  <div class="mx-auto max-w-3xl px-6 py-12">
    <header class="text-center mb-10">
      <h1 class="text-2xl font-semibold text-fg">Download PortBay</h1>
      <p class="mt-2 text-[13px] text-fg-muted">
        A local development environment manager for your machine. Pick your
        platform to get started.
      </p>
    </header>

    <div class="grid gap-4 sm:grid-cols-3">
      {#each order as key (key)}
        {@const p = PLATFORMS[key]}
        {@const isYours = detected === key}
        <div
          class="relative flex flex-col items-center text-center gap-3 rounded-2xl
                 border bg-surface p-6 transition-colors
                 {isYours ? 'border-accent' : 'border-border'}"
        >
          {#if isYours}
            <span
              class="absolute -top-2.5 left-1/2 -translate-x-1/2 px-2 py-0.5
                     rounded-full bg-accent text-on-accent text-[10px] font-medium
                     whitespace-nowrap"
            >
              Your device
            </span>
          {/if}

          <span class="text-fg">
            <PlatformIcon os={key} size={40} />
          </span>
          <div>
            <div class="text-[14px] font-semibold text-fg">{p.label}</div>
            <div class="text-[11.5px] text-fg-subtle font-mono">{p.ext}</div>
          </div>

          {#if p.available}
            <a
              href={p.href}
              download
              class="mt-1 inline-flex items-center justify-center gap-1.5 h-9 w-full
                     rounded-lg text-[12.5px] font-medium text-on-accent bg-accent
                     hover:bg-accent-hover transition-colors"
            >
              Download {p.ext}
            </a>
          {:else}
            <span
              class="mt-1 inline-flex items-center justify-center h-9 w-full
                     rounded-lg text-[12px] text-fg-subtle border border-border
                     bg-surface-2/40 cursor-default"
            >
              {p.note}
            </span>
          {/if}
        </div>
      {/each}
    </div>

    <p class="mt-8 text-center text-[11.5px] text-fg-subtle">
      macOS 11+ · Apple Silicon. The download is served from PortBay's GitHub
      releases. Windows and Linux builds are on the way.
    </p>
  </div>
</div>
