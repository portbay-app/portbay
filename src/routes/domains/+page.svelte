<!--
  /domains — every project hostname, the URL it resolves to, and which
  layer (dnsmasq wildcard vs /etc/hosts) is routing it.

  Read-only — hostname edits happen in the project detail panel. This
  page is the "what's wired up?" audit surface.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { openUrl } from "@tauri-apps/plugin-opener";

  import { DashboardCard, Icon, StatusDot } from "$lib/components/atoms";
  import { safeInvoke } from "$lib/ipc";
  import { errorBus } from "$lib/stores/errors.svelte";
  import { projectDetailPanel } from "$lib/stores/detailPanel.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import type { ResolverStatus } from "$lib/types/dns";

  let dnsStatus = $state<ResolverStatus | null>(null);
  let dnsBusy = $state<boolean>(false);

  async function refreshDns() {
    try {
      dnsStatus = await safeInvoke<ResolverStatus>("dnsmasq_resolver_status");
    } catch {
      dnsStatus = null;
    }
  }

  async function installDns() {
    dnsBusy = true;
    try {
      await safeInvoke("dnsmasq_install_resolver");
      errorBus.push({
        code: "DNS_INSTALLED",
        whatHappened: `DNS routing for .${dnsStatus?.suffix ?? "test"} installed.`,
        whyItMatters:
          "Subdomains of this suffix now resolve to 127.0.0.1 via dnsmasq. /etc/hosts edits are no longer needed.",
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });
      await refreshDns();
    } catch {
      /* toast already pushed by safeInvoke */
    } finally {
      dnsBusy = false;
    }
  }

  onMount(() => {
    void refreshDns();
  });

  /** True when dnsmasq's /etc/resolver/<suffix> is in place — the
   *  wildcard handles routing and /etc/hosts entries become
   *  redundant for that suffix. */
  const dnsRouting = $derived<boolean>(dnsStatus?.installed === true);

  const inSuffix = $derived(
    dnsStatus
      ? projects.value.filter((p) => p.hostname.endsWith(`.${dnsStatus!.suffix}`))
      : projects.value,
  );
  const outsideSuffix = $derived(
    dnsStatus
      ? projects.value.filter((p) => !p.hostname.endsWith(`.${dnsStatus!.suffix}`))
      : [],
  );
</script>

<div class="p-6 space-y-4">
  <header>
    <h2 class="text-lg font-semibold tracking-tight">Domains</h2>
    <p class="text-xs text-fg-muted mt-0.5">
      One row per project hostname. Click a row to open the project's
      detail panel; click the URL to open it in your default browser.
    </p>
  </header>

  <DashboardCard title="Resolution layer" flush>
    <div class="flex items-start gap-3 text-sm">
      <StatusDot status={dnsRouting ? "running" : "stopped"} size="lg" />
      <div class="flex-1 min-w-0 space-y-1">
        {#if dnsRouting && dnsStatus}
          <div class="font-medium">
            Wildcard DNS active — <code class="font-mono">*.{dnsStatus.suffix}</code>
            resolves via dnsmasq on port {dnsStatus.currentPort}.
          </div>
          <div class="text-xs text-fg-muted">
            <code class="font-mono">{dnsStatus.path}</code> is in place;
            <code>/etc/hosts</code> writes are skipped for this suffix.
            Manage from Settings → DNS routing.
          </div>
        {:else if dnsStatus}
          <div class="font-medium">
            Wildcard DNS not active — hostnames resolve via
            <code class="font-mono">/etc/hosts</code> entries.
          </div>
          <div class="text-xs text-fg-muted">
            Without resolver routing, browsers can't reach
            <code class="font-mono">*.{dnsStatus.suffix}</code> hostnames
            (they show <code>DNS_PROBE_POSSIBLE</code>). Install the
            resolver file now — one macOS authorisation prompt, no further
            <code>/etc/hosts</code> edits needed.
          </div>
          <div class="pt-2">
            <button
              type="button"
              onclick={installDns}
              disabled={dnsBusy}
              class="px-3 py-1.5 text-xs rounded-md text-accent border border-accent/40
                     hover:bg-accent/10 transition-colors disabled:opacity-50"
            >
              {dnsBusy ? "Installing…" : `Install DNS routing for *.${dnsStatus.suffix}`}
            </button>
          </div>
        {:else}
          <div class="text-xs text-fg-muted">Checking resolver state…</div>
        {/if}
      </div>
    </div>
  </DashboardCard>

  {#if projects.value.length === 0}
    <DashboardCard title="Hostnames" flush>
      <p class="text-sm text-fg-muted py-4 text-center">
        No projects registered yet. Add a project from the top bar to claim
        its first hostname.
      </p>
    </DashboardCard>
  {:else}
    <DashboardCard title="Hostnames ({projects.value.length})" flush>
      <div class="-mx-4">
        <table class="w-full text-sm">
          <thead
            class="text-[11px] uppercase tracking-wide text-fg-subtle bg-surface-2"
          >
            <tr>
              <th class="text-left font-medium px-4 py-2">Project</th>
              <th class="text-left font-medium px-4 py-2">Hostname</th>
              <th class="text-left font-medium px-4 py-2">URL</th>
              <th class="text-left font-medium px-4 py-2">Routed via</th>
              <th class="text-right font-medium px-4 py-2"></th>
            </tr>
          </thead>
          <tbody class="divide-y divide-border">
            {#each inSuffix as project (project.id)}
              <tr class="hover:bg-surface-2 transition-colors">
                <td class="px-4 py-2 align-middle">
                  <button
                    type="button"
                    onclick={() => projectDetailPanel.show(project.id)}
                    class="flex items-center gap-2 text-left"
                  >
                    <StatusDot status={project.status} />
                    <span class="font-medium">{project.name}</span>
                  </button>
                </td>
                <td class="px-4 py-2 font-mono text-xs text-fg-muted">
                  {project.hostname}
                </td>
                <td class="px-4 py-2">
                  <button
                    type="button"
                    onclick={() => openUrl(project.url)}
                    class="text-accent hover:text-accent-hover font-mono text-xs
                           inline-flex items-center gap-1"
                  >
                    {project.url}
                    <Icon name="external-link" size={10} />
                  </button>
                </td>
                <td class="px-4 py-2 text-xs text-fg-muted">
                  {dnsRouting ? "dnsmasq wildcard" : "/etc/hosts"}
                </td>
                <td class="px-4 py-2 text-right">
                  <button
                    type="button"
                    onclick={() => projectDetailPanel.show(project.id)}
                    title="Open project detail"
                    class="p-1 rounded-md text-fg-subtle hover:text-fg hover:bg-surface-2 transition-colors"
                  >
                    <Icon name="chevron-right" size={12} />
                  </button>
                </td>
              </tr>
            {/each}
            {#each outsideSuffix as project (project.id)}
              <tr class="hover:bg-surface-2 transition-colors">
                <td class="px-4 py-2 align-middle">
                  <button
                    type="button"
                    onclick={() => projectDetailPanel.show(project.id)}
                    class="flex items-center gap-2 text-left"
                  >
                    <StatusDot status={project.status} />
                    <span class="font-medium">{project.name}</span>
                  </button>
                </td>
                <td class="px-4 py-2 font-mono text-xs text-fg-muted">
                  {project.hostname}
                </td>
                <td class="px-4 py-2">
                  <button
                    type="button"
                    onclick={() => openUrl(project.url)}
                    class="text-accent hover:text-accent-hover font-mono text-xs
                           inline-flex items-center gap-1"
                  >
                    {project.url}
                    <Icon name="external-link" size={10} />
                  </button>
                </td>
                <td class="px-4 py-2 text-xs">
                  <span
                    class="inline-flex items-center gap-1 text-status-unhealthy"
                    title="Hostname suffix doesn't match the configured wildcard — only routed via /etc/hosts."
                  >
                    <Icon name="info" size={11} />
                    /etc/hosts only
                  </span>
                </td>
                <td class="px-4 py-2 text-right">
                  <button
                    type="button"
                    onclick={() => projectDetailPanel.show(project.id)}
                    title="Open project detail"
                    class="p-1 rounded-md text-fg-subtle hover:text-fg hover:bg-surface-2 transition-colors"
                  >
                    <Icon name="chevron-right" size={12} />
                  </button>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    </DashboardCard>
  {/if}
</div>
