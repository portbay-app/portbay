<!--
  /web-servers — the web servers PortBay can serve PHP projects with.

  PortBay's model is not ServBay's "install a global Apache/Nginx service and
  edit its httpd.conf". Here:
    • Caddy is the always-on edge — host routing, local HTTPS, reverse proxy.
    • Nginx / Apache are optional per-project PHP backends. PortBay generates
      their configs at reconcile time and Caddy proxies the hostname to them.

  So this page surfaces *reality* rather than a global config form: for each
  server, its role, whether the binary is present, its version, and which
  projects use it. The one writable knob is "default for new PHP projects",
  persisted to preferences and read by the Add Project wizard.
-->
<script lang="ts">
  import { onMount } from "svelte";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import type { IconName } from "$lib/components/atoms/Icon.svelte";

  import { safeInvoke } from "$lib/ipc";
  import { preferences } from "$lib/stores/preferences.svelte";
  import { sidecars } from "$lib/stores/sidecars.svelte";
  import type { WebServer } from "$lib/types/projects";
  import type { WebServerInfo } from "$lib/types/webservers";

  let servers = $state<WebServerInfo[]>([]);
  let loading = $state<boolean>(true);
  let selectedId = $state<WebServer>("caddy");
  let savingDefault = $state<boolean>(false);

  const ICON: Record<WebServer, IconName> = {
    caddy: "globe",
    nginx: "server",
    apache: "server",
  };

  onMount(() => {
    sidecars.start();
    void preferences.load();
    void refresh();
    return () => sidecars.stop();
  });

  async function refresh() {
    loading = true;
    try {
      servers = await safeInvoke<WebServerInfo[]>("webserver_overview");
    } finally {
      loading = false;
    }
  }

  const selected = $derived<WebServerInfo | undefined>(
    servers.find((s) => s.id === selectedId),
  );

  // The current default is read live from preferences so the badge flips the
  // instant the user clicks "Set as default" — null means Caddy.
  const defaultServer = $derived<WebServer>(
    preferences.value.defaultWebServer ?? "caddy",
  );

  // Caddy's running state is live from the sidecar-health store; the other two
  // run per-project under Process Compose, so they have no single daemon state.
  const caddyState = $derived(sidecars.value.caddy.status);

  type Tone = "running" | "idle" | "missing";

  function serverTone(s: WebServerInfo): Tone {
    if (s.edge) {
      return caddyState === "running" ? "running" : "idle";
    }
    return s.installed ? "idle" : "missing";
  }

  function statusLabel(s: WebServerInfo): string {
    if (s.edge) {
      switch (caddyState) {
        case "running":
          return "Edge · running";
        case "stopped":
          return "Edge · stopped";
        case "unreachable":
          return "Edge · unreachable";
        default:
          return "Edge";
      }
    }
    if (!s.installed) return "Not detected";
    return s.version ? `Detected · v${s.version}` : "Detected";
  }

  const toneDot: Record<Tone, string> = {
    running: "bg-status-running",
    idle: "bg-fg-subtle/60",
    missing: "bg-status-unhealthy",
  };

  const tonePill: Record<Tone, string> = {
    running: "bg-status-running/15 text-status-running",
    idle: "bg-surface-2 text-fg-muted",
    missing: "bg-status-unhealthy/15 text-status-unhealthy",
  };

  /** Whether picking this server as the new-project default is allowed. */
  function canBeDefault(s: WebServerInfo): boolean {
    return s.edge || s.installed;
  }

  async function setDefault(s: WebServerInfo) {
    if (!canBeDefault(s) || s.id === defaultServer || savingDefault) return;
    savingDefault = true;
    try {
      // Caddy is the implicit default — store null so the preference stays
      // "unset → Caddy" rather than pinning a value that could drift.
      await preferences.update({
        defaultWebServer: s.id === "caddy" ? null : s.id,
      });
    } finally {
      savingDefault = false;
    }
  }
</script>

<div class="h-full flex">
  <!-- Left rail — server sub-nav -->
  <aside
    class="w-[260px] shrink-0 border-r border-border bg-surface/40
           overflow-y-auto flex flex-col"
    aria-label="Web servers"
  >
    <header
      class="sticky top-0 z-10 px-4 pt-4 pb-3 bg-surface/40 backdrop-blur-sm
             border-b border-border/40"
    >
      <h2 class="text-[13px] font-semibold text-fg">Web Server</h2>
      <p class="mt-1 text-[11px] text-fg-subtle leading-relaxed">
        How PortBay serves your PHP projects.
      </p>
    </header>

    <nav class="px-2 py-2 space-y-1 flex-1 min-h-0" aria-label="Servers">
      {#each servers as s (s.id)}
        {@const tone = serverTone(s)}
        {@const isActive = selectedId === s.id}
        <button
          type="button"
          onclick={() => (selectedId = s.id)}
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
            <Icon name={ICON[s.id]} size={16} />
          </span>
          <span class="min-w-0 flex-1 leading-tight">
            <span class="flex items-center gap-1.5">
              <span
                class="inline-block w-1.5 h-1.5 rounded-full shrink-0 {toneDot[
                  tone
                ]}"
                aria-hidden="true"
              ></span>
              <span class="text-[13px] font-semibold text-fg truncate">
                {s.name}
              </span>
              {#if s.id === defaultServer}
                <span
                  class="shrink-0 text-[9.5px] font-semibold uppercase tracking-wide
                         px-1 py-px rounded bg-accent/15 text-accent"
                >
                  Default
                </span>
              {/if}
            </span>
            <span class="block text-[11px] text-fg-subtle truncate">
              {statusLabel(s)}
            </span>
          </span>
          {#if s.projects.length > 0}
            <span class="shrink-0 text-[10.5px] tabular-nums text-fg-subtle">
              {s.projects.length}
            </span>
          {/if}
        </button>
      {/each}

      {#if loading && servers.length === 0}
        <p class="px-2 py-4 text-[12px] text-fg-subtle">Loading…</p>
      {/if}
    </nav>
  </aside>

  <!-- Right pane — selected server detail -->
  <section class="flex-1 min-w-0 overflow-y-auto">
    {#if !selected}
      <div class="h-full flex items-center justify-center">
        <div class="text-center max-w-sm px-6">
          <Icon name="server-cog" size={28} class="text-fg-subtle mx-auto" />
          <p class="mt-3 text-[13px] text-fg-muted">
            Select a server from the sidebar.
          </p>
        </div>
      </div>
    {:else}
      {@const tone = serverTone(selected)}
      <div class="max-w-3xl px-6 py-6 space-y-6">
        <!-- Header -->
        <div class="flex items-start gap-4">
          <span
            class="shrink-0 grid place-items-center w-12 h-12 rounded-xl
                   bg-surface-2 text-fg-muted"
          >
            <Icon name={ICON[selected.id]} size={24} />
          </span>
          <div class="min-w-0 flex-1">
            <div class="flex items-center gap-2.5 flex-wrap">
              <h1 class="text-[20px] font-semibold text-fg leading-none">
                {selected.name}
              </h1>
              <span
                class="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-md
                       text-[11px] font-medium {tonePill[tone]}"
              >
                <span
                  class="inline-block w-1.5 h-1.5 rounded-full {toneDot[tone]}"
                  aria-hidden="true"
                ></span>
                {statusLabel(selected)}
              </span>
              {#if selected.id === defaultServer}
                <span
                  class="text-[10px] font-semibold uppercase tracking-wide
                         px-1.5 py-0.5 rounded bg-accent/15 text-accent"
                >
                  Default for new PHP projects
                </span>
              {/if}
            </div>
            <p class="mt-2 text-[13px] text-fg-muted leading-relaxed">
              {selected.role}
            </p>
          </div>
        </div>

        <!-- Facts -->
        <dl class="grid grid-cols-2 gap-x-6 gap-y-4 text-[12.5px]">
          <div class="space-y-0.5">
            <dt class="text-fg-subtle">Binary</dt>
            <dd class="font-mono text-fg break-all">
              {#if selected.bundled}
                Bundled with PortBay
              {:else if selected.binaryPath}
                {selected.binaryPath}
              {:else}
                <span class="text-fg-subtle">— not found on this Mac</span>
              {/if}
            </dd>
          </div>
          <div class="space-y-0.5">
            <dt class="text-fg-subtle">Version</dt>
            <dd class="font-mono text-fg">
              {selected.version ? `v${selected.version}` : "—"}
            </dd>
          </div>
        </dl>

        <!-- Default control -->
        <div
          class="flex items-center justify-between gap-4 rounded-xl border
                 border-border bg-surface px-4 py-3"
        >
          <div class="min-w-0">
            <p class="text-[13px] font-medium text-fg">
              Default for new PHP projects
            </p>
            <p class="mt-0.5 text-[11.5px] text-fg-subtle leading-relaxed">
              {#if selected.id === defaultServer}
                The Add Project wizard pre-selects {selected.name} for new PHP
                projects.
              {:else if canBeDefault(selected)}
                Make {selected.name} the wizard's pre-selected server. Existing
                projects keep their current setting.
              {:else}
                Install {selected.name} first — a default that isn't present
                would make new projects fail to start.
              {/if}
            </p>
          </div>
          <button
            type="button"
            onclick={() => setDefault(selected)}
            disabled={!canBeDefault(selected) ||
              selected.id === defaultServer ||
              savingDefault}
            class="shrink-0 inline-flex items-center gap-1.5 h-8 px-3 rounded-lg
                   text-[12px] font-medium transition
                   {selected.id === defaultServer
              ? 'bg-surface-2 text-fg-muted cursor-default'
              : 'bg-accent text-on-accent hover:brightness-110 active:brightness-95'}
                   disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {#if selected.id === defaultServer}
              <Icon name="check" size={13} />
              Current default
            {:else}
              Set as default
            {/if}
          </button>
        </div>

        <!-- Projects using this server -->
        <div>
          <h2 class="text-[13px] font-semibold text-fg mb-2">
            Projects using {selected.name}
            {#if selected.projects.length > 0}
              <span class="text-fg-subtle font-normal">
                ({selected.projects.length})
              </span>
            {/if}
          </h2>
          {#if selected.projects.length === 0}
            <p
              class="text-[12px] text-fg-subtle leading-relaxed rounded-lg
                     border border-dashed border-border px-3 py-4"
            >
              {#if selected.edge}
                No PHP projects are served directly by Caddy yet. Caddy still
                fronts every project as the edge.
              {:else if !selected.installed}
                {selected.name} isn't installed, so no projects can use it.
              {:else}
                No PHP projects use {selected.name}. Pick it per project in the
                Add Project wizard or a project's Advanced settings.
              {/if}
            </p>
          {:else}
            <ul class="space-y-1">
              {#each selected.projects as p (p.id)}
                <li>
                  <a
                    href="/?project={p.id}"
                    class="flex items-center gap-2.5 px-3 py-2 rounded-lg
                           hover:bg-surface-2/60 transition-colors"
                  >
                    <Icon name="file-code" size={14} class="text-fg-subtle" />
                    <span class="text-[13px] text-fg truncate">{p.name}</span>
                    <span class="ml-auto text-[11px] font-mono text-fg-subtle">
                      {p.id}
                    </span>
                  </a>
                </li>
              {/each}
            </ul>
          {/if}
        </div>
      </div>
    {/if}
  </section>
</div>
