<!--
  /tunnels — public sharing via Cloudflare Tunnel.

  Two-pane layout mirroring /web-servers and /dns: a left rail listing the
  available tunnel providers, and a right pane showing the selected provider's
  management surface. Today there's one provider — Cloudflare Tunnel — but the
  rail is the seam other providers (ngrok, Tailscale Funnel, …) slot into
  later without reworking the page.

  Cloudflare pane: every registered project with its live tunnel state. "Share
  publicly" starts an ephemeral cloudflared tunnel (the backend resolves the
  project's own URL); once Cloudflare assigns a trycloudflare.com URL it
  appears here with a copy button. All state flows from the `tunnels` store,
  which the TopBar and command palette also read — one source of truth.
-->
<script lang="ts">
  import { onMount } from "svelte";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import type { IconName } from "$lib/components/atoms/Icon.svelte";
  import StatusDot from "$lib/components/atoms/StatusDot.svelte";
  import ProjectAvatar from "$lib/components/atoms/ProjectAvatar.svelte";
  import CustomTunnelField from "$lib/components/projects/CustomTunnelField.svelte";

  import { projects } from "$lib/stores/projects.svelte";
  import { tunnels } from "$lib/stores/tunnels.svelte";
  import { errorBus } from "$lib/stores/errors.svelte";

  type ProviderId = "cloudflare";

  // The tunnel providers PortBay can route through. One today; the rail is
  // built as a list so adding the next provider is a one-line change here.
  const PROVIDERS: {
    id: ProviderId;
    name: string;
    icon: IconName;
    tagline: string;
  }[] = [
    {
      id: "cloudflare",
      name: "Cloudflare Tunnel",
      icon: "cloud",
      tagline: "trycloudflare.com",
    },
  ];

  let selectedProvider = $state<ProviderId>("cloudflare");
  let copied = $state<string | null>(null);

  onMount(() => {
    void projects.start();
    void tunnels.refresh();
  });

  const sorted = $derived(
    [...projects.value].sort((a, b) => a.name.localeCompare(b.name)),
  );
  const activeCount = $derived(tunnels.count);

  async function copy(url: string) {
    try {
      await navigator.clipboard.writeText(url);
      copied = url;
      setTimeout(() => {
        if (copied === url) copied = null;
      }, 1500);
      errorBus.push({
        code: "COPIED",
        whatHappened: "Public URL copied.",
        whyItMatters: "Paste it anywhere — a phone, a colleague, a webhook.",
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });
    } catch {
      /* no clipboard permission */
    }
  }
</script>

<div class="h-full flex">
  <!-- Left rail — tunnel providers -->
  <aside
    class="w-[260px] shrink-0 border-r border-border bg-surface/40
           overflow-y-auto flex flex-col"
    aria-label="Tunnel providers"
  >
    <header
      class="sticky top-0 z-10 px-4 pt-4 pb-3 bg-surface/95
             border-b border-border/40"
    >
      <h2 class="text-[13px] font-semibold text-fg">Tunnels</h2>
      <p class="mt-1 text-[11px] text-fg-subtle leading-relaxed">
        Expose a local project on a public URL.
      </p>
    </header>

    <nav class="px-2 py-2 space-y-1 flex-1 min-h-0" aria-label="Providers">
      {#each PROVIDERS as provider (provider.id)}
        {@const isActive = selectedProvider === provider.id}
        <button
          type="button"
          onclick={() => (selectedProvider = provider.id)}
          aria-current={isActive ? "true" : undefined}
          class="w-full flex items-center gap-3 px-2.5 py-2 rounded-lg
                 text-left transition-colors cursor-pointer
                 focus-visible:outline-none focus-visible:ring-2
                 focus-visible:ring-accent/40
                 {isActive
            ? 'bg-accent/10 ring-1 ring-inset ring-accent/40'
            : 'hover:bg-surface-2/60'}"
        >
          <span
            class="shrink-0 grid place-items-center w-8 h-8 rounded-lg
                   bg-surface-2 text-fg-muted"
          >
            <Icon name={provider.icon} size={16} />
          </span>
          <span class="min-w-0 flex-1 leading-tight">
            <span class="block text-[13px] font-semibold text-fg truncate">
              {provider.name}
            </span>
            <span class="block text-[11px] text-fg-subtle truncate">
              {activeCount > 0
                ? `${activeCount} active tunnel${activeCount === 1 ? "" : "s"}`
                : provider.tagline}
            </span>
          </span>
          {#if activeCount > 0}
            <span
              class="shrink-0 inline-flex items-center h-5 px-1.5 rounded-full
                     text-[10.5px] font-medium tabular-nums bg-accent/10 text-accent"
            >
              {activeCount}
            </span>
          {/if}
        </button>
      {/each}
    </nav>
  </aside>

  <!-- Right pane — selected provider -->
  <section class="flex-1 min-w-0 overflow-y-auto">
    <header class="px-8 pt-8 pb-5 border-b border-border/60">
      <div class="flex items-center gap-2.5">
        <Icon name="cloud" size={18} class="text-accent" />
        <h1 class="text-[17px] font-semibold tracking-tight text-fg">
          Cloudflare Tunnel
        </h1>
        {#if activeCount > 0}
          <span
            class="ml-1 inline-flex items-center h-5 px-2 rounded-full text-[11px]
                   font-medium tabular-nums bg-accent/10 text-accent"
          >
            {activeCount} live
          </span>
        {/if}
      </div>
      <p class="mt-1.5 text-[12.5px] text-fg-muted leading-relaxed max-w-2xl">
        Share a local project on a public <code class="font-mono">trycloudflare.com</code>
        URL through Cloudflare Tunnel — no account, no port-forwarding. Tunnels are
        ephemeral: they disappear when you stop sharing or quit PortBay.
      </p>
    </header>

    <div class="px-8 py-6 space-y-2">
      {#if sorted.length === 0}
        <div
          class="rounded-2xl border border-dashed border-border/70 px-6 py-12 text-center"
        >
          <Icon name="cloud" size={22} class="text-fg-subtle mx-auto mb-2" />
          <p class="text-[13px] text-fg">No projects to share yet</p>
          <p class="mt-1 text-[12px] text-fg-muted">
            Add a project, then come back to expose it publicly.
          </p>
        </div>
      {:else}
        {#each sorted as project (project.id)}
          {@const tunnel = tunnels.statusFor(project.id)}
          {@const sharing = tunnel !== null}
          {@const establishing = sharing && !tunnel?.publicUrl}
          {@const busy = tunnels.isBusy(project.id)}
          <article
            class="rounded-2xl border px-5 py-4 transition-colors {sharing
              ? 'bg-accent/[0.04] border-accent/30'
              : 'bg-surface border-border/70'}"
          >
            <div class="flex items-center gap-3">
              <ProjectAvatar
                id={project.id}
                name={project.name}
                type={project.type}
                size={32}
              />
              <div class="min-w-0 flex-1">
                <div class="flex items-center gap-2">
                  <h2 class="text-[13px] font-semibold text-fg truncate">
                    {project.name}
                  </h2>
                  <StatusDot status={project.status} size="sm" />
                </div>
                <p class="text-[11px] text-fg-subtle font-mono truncate">
                  {project.url}
                </p>
              </div>

              {#if sharing}
                <button
                  type="button"
                  onclick={() => tunnels.stopSharing(project.id)}
                  disabled={busy}
                  class="shrink-0 inline-flex items-center gap-1.5 h-8 px-3.5 rounded-md
                         text-[12px] font-medium text-status-crashed border border-status-crashed/40
                         hover:bg-status-crashed/10 disabled:opacity-50 disabled:cursor-not-allowed
                         transition-colors"
                >
                  {#if busy}
                    <Icon name="refresh-cw" size={11} class="animate-spin" />
                    Stopping…
                  {:else}
                    <Icon name="circle-stop" size={12} />
                    Stop sharing
                  {/if}
                </button>
              {:else}
                <button
                  type="button"
                  onclick={() => tunnels.share(project.id)}
                  disabled={busy}
                  class="shrink-0 inline-flex items-center gap-1.5 h-8 px-3.5 rounded-md
                         text-[12px] font-medium text-on-accent bg-accent shadow-sm
                         hover:brightness-110 active:brightness-95
                         disabled:opacity-50 disabled:cursor-not-allowed transition"
                >
                  {#if busy}
                    <Icon name="refresh-cw" size={11} class="animate-spin" />
                    Starting…
                  {:else}
                    <Icon name="cloud" size={12} />
                    Share publicly
                  {/if}
                </button>
              {/if}
            </div>

            {#if establishing}
              <div
                class="mt-3 flex items-center gap-2 text-[11.5px] text-fg-muted"
              >
                <Icon name="refresh-cw" size={12} class="animate-spin" />
                Establishing tunnel — Cloudflare is assigning a public URL…
              </div>
            {:else if sharing && tunnel?.publicUrl}
              <div
                class="mt-3 flex items-center gap-2 px-3 py-2 rounded-md bg-bg/60 border border-border"
              >
                <Icon name="globe" size={13} class="text-accent shrink-0" />
                <code class="flex-1 min-w-0 text-[12px] font-mono text-fg truncate">
                  {tunnel.publicUrl}
                </code>
                <button
                  type="button"
                  onclick={() => tunnel.publicUrl && copy(tunnel.publicUrl)}
                  title="Copy public URL"
                  aria-label="Copy public URL"
                  class="shrink-0 p-1 rounded text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
                >
                  <Icon name={copied === tunnel.publicUrl ? "check" : "link"} size={13} />
                </button>
              </div>
              {#if project.status !== "running"}
                <p class="mt-2 text-[11px] text-status-unhealthy">
                  This project isn't running — visitors will see PortBay's
                  "waking up" page until you start it.
                </p>
              {/if}
            {/if}

            {#if !sharing}
              <div class="mt-3 pt-3 border-t border-border/60">
                <CustomTunnelField {project} />
              </div>
            {/if}
          </article>
        {/each}
      {/if}
    </div>
  </section>
</div>
