<!--
  /php — installed PHP versions + per-version metadata.

  Detection-only for v1. Missing versions are shown with a copy-able
  `brew install` command; bundled installer is a follow-up card.
-->
<script lang="ts">
  import { onMount } from "svelte";

  import { DashboardCard, Icon, StatusDot } from "$lib/components/atoms";
  import { php } from "$lib/stores/php.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import { COMMON_EXTENSIONS } from "$lib/types/php";
  import type { PhpInstall } from "$lib/types/php";

  /** Versions Homebrew exposes as `php@<ver>` taps. Used by the
   *  "missing" section so we can show install hints for anything the
   *  detector didn't find. */
  const HOMEBREW_VERSIONS = ["7.4", "8.0", "8.1", "8.2", "8.3", "8.4"];

  let copied = $state<string | null>(null);

  onMount(() => {
    void php.refresh();
  });

  /** Which versions in HOMEBREW_VERSIONS aren't installed locally? */
  const missing = $derived<string[]>(
    HOMEBREW_VERSIONS.filter((v) => !php.isInstalled(v)),
  );

  /** For each installed version, how many registered projects use it. */
  function projectsUsing(version: string): number {
    return projects.value.filter((p) => p.phpVersion === version).length;
  }

  async function copyHint(version: string) {
    try {
      await navigator.clipboard.writeText(`brew install php@${version}`);
      copied = version;
      setTimeout(() => {
        if (copied === version) copied = null;
      }, 1500);
    } catch {
      /* clipboard unavailable — ignore */
    }
  }

  function loadedHits(install: PhpInstall): string[] {
    const lc = new Set(install.loadedExtensions.map((e) => e.toLowerCase()));
    return COMMON_EXTENSIONS.filter((e) => lc.has(e.toLowerCase()));
  }
</script>

<div class="p-6 space-y-4">
  <header class="flex items-center justify-between">
    <div>
      <h2 class="text-lg font-semibold tracking-tight">PHP versions</h2>
      <p class="text-xs text-fg-muted mt-0.5">
        Detected from Homebrew. PortBay does not bundle a PHP compiler —
        install missing versions via the Homebrew formulas listed below.
      </p>
    </div>
    <button
      type="button"
      onclick={() => php.refresh()}
      disabled={php.loading}
      class="inline-flex items-center gap-1.5 text-xs text-fg-muted
             border border-border hover:text-fg hover:bg-surface-2
             rounded-md px-2.5 py-1.5 transition-colors disabled:opacity-50"
    >
      <Icon
        name="refresh-cw"
        size={12}
        class={php.loading ? "animate-spin" : ""}
      />
      Re-detect
    </button>
  </header>

  {#if php.value.length === 0 && !php.loading}
    <DashboardCard title="Nothing detected" flush>
      <p class="text-sm text-fg-muted">
        No PHP install was found under
        <code class="font-mono">/opt/homebrew/opt/php@*</code> or
        <code class="font-mono">/usr/local/opt/php@*</code>, and
        <code class="font-mono">php</code> isn't on your PATH. Install a
        version below and re-detect.
      </p>
    </DashboardCard>
  {/if}

  {#each php.value as install (install.version)}
    {@const usedBy = projectsUsing(install.version)}
    {@const fpmOk = install.phpFpmBin !== null}
    {@const commonLoaded = loadedHits(install)}
    <DashboardCard title="PHP {install.version}" flush>
      {#snippet badge()}
        <span class="inline-flex items-center gap-1.5 text-xs">
          <StatusDot status={fpmOk ? "running" : "unhealthy"} />
          <span class="text-fg-muted">
            {fpmOk ? "FPM available" : "CLI only — no php-fpm found"}
          </span>
          {#if usedBy > 0}
            <span class="text-fg-subtle">·</span>
            <span class="text-fg-muted">
              used by {usedBy} project{usedBy === 1 ? "" : "s"}
            </span>
          {/if}
        </span>
      {/snippet}

      <div class="space-y-3 text-xs">
        <dl class="grid grid-cols-[140px,1fr] gap-x-4 gap-y-1.5">
          <dt class="text-fg-muted">php binary</dt>
          <dd class="font-mono text-fg-muted truncate" title={install.phpBin}>
            {install.phpBin}
          </dd>
          {#if install.phpFpmBin}
            <dt class="text-fg-muted">php-fpm</dt>
            <dd
              class="font-mono text-fg-muted truncate"
              title={install.phpFpmBin}
            >
              {install.phpFpmBin}
            </dd>
          {/if}
          {#if install.phpIni}
            <dt class="text-fg-muted">php.ini</dt>
            <dd class="font-mono text-fg-muted truncate" title={install.phpIni}>
              {install.phpIni}
            </dd>
          {/if}
          {#if install.additionalIniDir}
            <dt class="text-fg-muted">extra .ini dir</dt>
            <dd
              class="font-mono text-fg-muted truncate"
              title={install.additionalIniDir}
            >
              {install.additionalIniDir}
            </dd>
          {/if}
          {#if install.extensionDir}
            <dt class="text-fg-muted">extension dir</dt>
            <dd
              class="font-mono text-fg-muted truncate"
              title={install.extensionDir}
            >
              {install.extensionDir}
            </dd>
          {/if}
        </dl>

        <div>
          <div class="text-fg-subtle uppercase tracking-wide text-[10px] mb-1.5">
            Loaded extensions ({install.loadedExtensions.length})
          </div>
          {#if commonLoaded.length > 0}
            <div class="flex flex-wrap gap-1.5 mb-2">
              {#each commonLoaded as ext (ext)}
                <span
                  class="inline-flex items-center gap-1 px-2 py-0.5 rounded-md
                         bg-accent/10 text-accent text-[11px] border border-accent/30"
                  title="Common extension — recognised by name"
                >
                  <Icon name="check" size={10} />
                  {ext}
                </span>
              {/each}
            </div>
          {/if}
          <details class="text-fg-muted">
            <summary class="cursor-pointer hover:text-fg select-none text-[11px]">
              Full extension list
            </summary>
            <div
              class="mt-2 p-2 rounded-md bg-surface border border-border
                     font-mono text-[11px] max-h-32 overflow-y-auto leading-relaxed"
            >
              {install.loadedExtensions.join(", ")}
            </div>
          </details>
        </div>

        {#if !fpmOk}
          <div
            class="px-2.5 py-2 rounded-md border border-status-unhealthy/40
                   bg-status-unhealthy/5 text-fg-muted leading-relaxed"
          >
            <Icon
              name="info"
              size={11}
              class="inline-block mr-1 text-status-unhealthy"
            />
            <code>php-fpm</code> wasn't found alongside this install.
            PortBay can run PHP projects that use this version only if the
            FPM binary is present. Reinstall the formula with
            <code class="font-mono">brew reinstall php@{install.version}</code>
            and re-detect.
          </div>
        {/if}
      </div>
    </DashboardCard>
  {/each}

  {#if missing.length > 0}
    <DashboardCard title="Available to install ({missing.length})" flush>
      <p class="text-xs text-fg-muted mb-3">
        These Homebrew formulas aren't installed yet. Run the command for
        the version you need, then re-detect.
      </p>
      <div class="grid grid-cols-1 md:grid-cols-2 gap-2">
        {#each missing as v (v)}
          <div
            class="flex items-center justify-between gap-2 p-2.5 rounded-md
                   border border-border bg-surface"
          >
            <div class="flex items-center gap-2 min-w-0">
              <StatusDot status="stopped" />
              <span class="font-medium text-sm">PHP {v}</span>
            </div>
            <button
              type="button"
              onclick={() => copyHint(v)}
              class="inline-flex items-center gap-1.5 text-[11px]
                     font-mono px-2 py-1 rounded-md
                     text-fg-muted bg-bg border border-border
                     hover:text-fg hover:bg-surface-2 transition-colors"
            >
              {#if copied === v}
                <Icon name="check" size={10} class="text-status-running" />
                copied
              {:else}
                brew install php@{v}
              {/if}
            </button>
          </div>
        {/each}
      </div>
    </DashboardCard>
  {/if}
</div>
