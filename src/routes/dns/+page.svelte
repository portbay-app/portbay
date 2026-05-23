<!--
  /dns — DNS management.

  Two panes mirroring /databases. Left rail: the dnsmasq config item, the
  derived DNS records (the *.suffix wildcard + every project hostname), and
  the entries PortBay manages in /etc/hosts. Right pane: the selected item.

  The dnsmasq config card is where the editable tuning lives (cache size,
  local TTL, negative-cache) plus the domain suffix and the resolver-file
  install toggle. The records + hosts lists are read-only — hostnames are
  edited in the project detail panel; the reconciler owns /etc/hosts.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { openUrl } from "@tauri-apps/plugin-opener";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import StatusDot from "$lib/components/atoms/StatusDot.svelte";

  import { dns } from "$lib/stores/dns.svelte";
  import { confirmDialog } from "$lib/stores/confirm.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import { projectDetailPanel } from "$lib/stores/detailPanel.svelte";
  import { MAX_DNS_CACHE_SIZE, MAX_DNS_LOCAL_TTL } from "$lib/types/dns";
  import type { DnsRecord, ManagedHostsEntry } from "$lib/types/dns";

  type Selection =
    | { type: "config" }
    | { type: "record"; hostname: string }
    | { type: "hosts"; hostname: string };

  let selection = $state<Selection>({ type: "config" });
  let query = $state<string>("");
  let copied = $state<string | null>(null);

  // Editable form state, synced from the store on load / save.
  let cacheSize = $state<number>(150);
  let localTtl = $state<number>(0);
  let disableNeg = $state<boolean>(false);
  let suffixInput = $state<string>("");

  onMount(() => {
    void dns.refresh();
    void projects.start();
  });

  // Resync the form whenever the store's settings or suffix change (initial
  // load, after a save, after a suffix migration). While the user is editing,
  // these references are stable so their edits aren't clobbered.
  $effect(() => {
    const s = dns.settings;
    cacheSize = s.cacheSize;
    localTtl = s.localTtl;
    disableNeg = s.disableNegativeCache;
  });
  $effect(() => {
    suffixInput = dns.status?.suffix ?? "";
  });

  const settingsDirty = $derived(
    cacheSize !== dns.settings.cacheSize ||
      localTtl !== dns.settings.localTtl ||
      disableNeg !== dns.settings.disableNegativeCache,
  );

  const suffixDirty = $derived(
    suffixInput.trim() !== "" &&
      suffixInput.trim() !== (dns.status?.suffix ?? ""),
  );

  const filteredRecords = $derived.by<DnsRecord[]>(() => {
    const q = query.trim().toLowerCase();
    if (!q) return dns.records;
    return dns.records.filter(
      (r) =>
        r.hostname.toLowerCase().includes(q) ||
        (r.projectName?.toLowerCase().includes(q) ?? false),
    );
  });

  const filteredHosts = $derived.by<ManagedHostsEntry[]>(() => {
    const q = query.trim().toLowerCase();
    if (!q) return dns.hosts;
    return dns.hosts.filter(
      (h) => h.hostname.toLowerCase().includes(q) || h.ip.includes(q),
    );
  });

  const selectedRecord = $derived.by<DnsRecord | null>(() => {
    const sel = selection;
    if (sel.type !== "record") return null;
    return dns.records.find((r) => r.hostname === sel.hostname) ?? null;
  });

  const selectedHost = $derived.by<ManagedHostsEntry | null>(() => {
    const sel = selection;
    if (sel.type !== "hosts") return null;
    return dns.hosts.find((h) => h.hostname === sel.hostname) ?? null;
  });

  function projectFor(id: string | null) {
    if (!id) return null;
    return projects.value.find((p) => p.id === id) ?? null;
  }

  async function copy(text: string) {
    if (!text) return;
    try {
      await navigator.clipboard.writeText(text);
      copied = text;
      setTimeout(() => {
        if (copied === text) copied = null;
      }, 1500);
    } catch {
      /* no clipboard permission */
    }
  }

  function clampInt(v: number, max: number): number {
    if (!Number.isFinite(v) || v < 0) return 0;
    return Math.min(Math.floor(v), max);
  }

  function saveSettings() {
    void dns.saveSettings({
      cacheSize: clampInt(cacheSize, MAX_DNS_CACHE_SIZE),
      localTtl: clampInt(localTtl, MAX_DNS_LOCAL_TTL),
      disableNegativeCache: disableNeg,
    });
  }

  async function saveSuffix() {
    const next = suffixInput.trim().replace(/^\.+|\.+$/g, "").toLowerCase();
    if (!next || next === dns.status?.suffix) return;
    const choice = await confirmDialog.open({
      title: `Change the domain suffix to ".${next}"?`,
      message:
        `Every project hostname will be renamed to <project>.${next}, and any HTTPS certificates will be reissued on the next sync.\n\n` +
        `If you use DNS routing, you'll need to reinstall the resolver for the new suffix (one macOS prompt).`,
      actions: [
        { label: `Change to .${next}`, value: "confirm", tone: "primary" },
      ],
    });
    if (choice !== "confirm") return;
    void dns.setSuffix(next);
  }
</script>

<div class="h-full flex">
  <!-- Left rail -->
  <aside
    class="w-[300px] shrink-0 border-r border-border bg-surface/40
           overflow-y-auto flex flex-col"
    aria-label="DNS"
  >
    <header
      class="sticky top-0 z-10 px-4 pt-4 pb-3 bg-surface/40 backdrop-blur-sm border-b border-border/40"
    >
      <h2 class="text-[13px] font-semibold text-fg mb-2.5">DNS</h2>
      <div class="relative">
        <Icon
          name="search"
          size={12}
          class="absolute left-2.5 top-1/2 -translate-y-1/2 text-fg-subtle pointer-events-none"
        />
        <input
          type="search"
          bind:value={query}
          placeholder="Search records…"
          aria-label="Search DNS records"
          class="w-full pl-7 pr-2 h-8 rounded-md bg-surface/80 border border-border/60
                 text-[12px] text-fg placeholder:text-fg-subtle
                 focus:outline-none focus:ring-1 focus:ring-accent/60
                 focus:border-accent/40 transition-colors"
        />
      </div>
    </header>

    <div class="px-2 py-2 space-y-3 flex-1 min-h-0">
      <!-- dnsmasq config item -->
      <div class="space-y-1">
        <button
          type="button"
          onclick={() => (selection = { type: "config" })}
          class="w-full flex items-center gap-3 px-2.5 py-2 rounded-lg text-left
                 transition-colors cursor-pointer
                 {selection.type === 'config'
            ? 'bg-accent/10 ring-1 ring-inset ring-accent/40'
            : 'hover:bg-surface-2/60'}"
        >
          <span
            class="shrink-0 inline-flex items-center justify-center w-8 h-8 rounded-lg
                   bg-surface-2 text-accent"
          >
            <Icon name="globe" size={16} />
          </span>
          <div class="min-w-0 flex-1 leading-tight">
            <div class="flex items-center gap-1.5">
              <StatusDot status={dns.dnsRouting ? "running" : "stopped"} size="sm" />
              <span class="text-[13px] font-semibold text-fg truncate">dnsmasq</span>
            </div>
            <p class="text-[11px] font-mono text-fg-subtle truncate">
              {dns.dnsRouting ? "wildcard active" : "resolver not installed"}
            </p>
          </div>
        </button>
      </div>

      <!-- DNS records -->
      <div class="space-y-0.5">
        <p
          class="px-2 py-1 text-[11px] uppercase tracking-wide text-fg-subtle flex items-center gap-1.5"
        >
          DNS Records
          <span class="font-mono">{dns.records.length}</span>
        </p>
        {#if filteredRecords.length === 0}
          <p class="px-2 py-1.5 text-[11px] text-fg-subtle">No matching records.</p>
        {:else}
          {#each filteredRecords as rec (rec.hostname)}
            {@const active =
              selection.type === "record" && selection.hostname === rec.hostname}
            <button
              type="button"
              onclick={() =>
                (selection = { type: "record", hostname: rec.hostname })}
              class="w-full flex items-center gap-2 px-2.5 py-1.5 rounded-md text-left
                     transition-colors {active
                ? 'bg-accent/10 ring-1 ring-inset ring-accent/40'
                : 'hover:bg-surface-2/60'}"
            >
              <Icon
                name={rec.kind === "wildcard" ? "star" : "link"}
                size={12}
                class="shrink-0 text-fg-subtle"
              />
              <span class="flex-1 min-w-0 font-mono text-[12px] text-fg truncate">
                {rec.hostname}
              </span>
              <span
                class="shrink-0 text-[9.5px] uppercase tracking-wide px-1.5 py-0.5 rounded
                       {rec.routedVia === 'dnsmasq'
                  ? 'bg-status-running/15 text-status-running'
                  : 'bg-fg-subtle/15 text-fg-subtle'}"
              >
                {rec.routedVia === "dnsmasq" ? "dns" : "hosts"}
              </span>
            </button>
          {/each}
        {/if}
      </div>

      <!-- Hosts file -->
      <div class="space-y-0.5">
        <p
          class="px-2 py-1 text-[11px] uppercase tracking-wide text-fg-subtle flex items-center gap-1.5"
        >
          Hosts file
          <span class="font-mono">{dns.hosts.length}</span>
        </p>
        {#if dns.hosts.length === 0}
          <p class="px-2 py-1.5 text-[11px] text-fg-subtle leading-relaxed">
            No PortBay-managed <code class="font-mono">/etc/hosts</code> entries.
            {dns.dnsRouting ? "DNS routing handles resolution." : ""}
          </p>
        {:else if filteredHosts.length === 0}
          <p class="px-2 py-1.5 text-[11px] text-fg-subtle">No matching entries.</p>
        {:else}
          {#each filteredHosts as h (h.hostname)}
            {@const active =
              selection.type === "hosts" && selection.hostname === h.hostname}
            <button
              type="button"
              onclick={() => (selection = { type: "hosts", hostname: h.hostname })}
              class="w-full flex items-center gap-2 px-2.5 py-1.5 rounded-md text-left
                     transition-colors {active
                ? 'bg-accent/10 ring-1 ring-inset ring-accent/40'
                : 'hover:bg-surface-2/60'}"
            >
              <Icon name="file-text" size={12} class="shrink-0 text-fg-subtle" />
              <span class="flex-1 min-w-0 font-mono text-[12px] text-fg truncate">
                {h.hostname}
              </span>
              <span class="shrink-0 font-mono text-[10.5px] text-fg-subtle">{h.ip}</span>
            </button>
          {/each}
        {/if}
      </div>
    </div>
  </aside>

  <!-- Right pane -->
  <section class="flex-1 min-w-0 overflow-y-auto">
    {#if selection.type === "config"}
      <!-- dnsmasq config -->
      <header
        class="px-8 pt-7 pb-5 border-b border-border/70 flex items-start justify-between gap-4 flex-wrap"
      >
        <div class="min-w-0 flex items-start gap-3">
          <span
            class="shrink-0 inline-flex items-center justify-center w-9 h-9 rounded-lg bg-surface-2 text-accent mt-0.5"
          >
            <Icon name="globe" size={18} />
          </span>
          <div class="min-w-0">
            <h1
              class="text-[20px] font-semibold tracking-tight text-fg flex items-center gap-2.5 flex-wrap"
            >
              dnsmasq
              <span
                class="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-full text-[11px] font-medium
                       {dns.dnsRouting
                  ? 'bg-status-running/15 text-status-running'
                  : 'bg-fg-subtle/15 text-fg-subtle'}"
              >
                <StatusDot status={dns.dnsRouting ? "running" : "stopped"} size="sm" />
                {dns.dnsRouting ? "Wildcard active" : "Resolver not installed"}
              </span>
            </h1>
            <p class="mt-1 text-[12px] text-fg-muted">
              Loopback resolver for <code class="font-mono">*.{dns.status?.suffix ?? "portbay.test"}</code>
              on port {dns.status?.currentPort ?? "—"}.
            </p>
          </div>
        </div>
        <div class="flex items-center gap-1.5 shrink-0 flex-wrap">
          {#if dns.dnsRouting}
            <button
              type="button"
              onclick={() => dns.uninstallResolver()}
              disabled={dns.isBusy("resolver")}
              class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md border border-status-crashed/40
                     text-status-crashed hover:bg-status-crashed/10 transition-colors text-[12px]
                     disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {#if dns.isBusy("resolver")}
                <Icon name="refresh-cw" size={11} class="animate-spin" />
              {:else}
                <Icon name="x" size={11} />
              {/if}
              Uninstall resolver
            </button>
          {:else}
            <button
              type="button"
              onclick={() => dns.installResolver()}
              disabled={dns.isBusy("resolver")}
              class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md text-[12px] font-medium
                     text-on-accent bg-accent hover:brightness-110 active:brightness-95
                     disabled:opacity-50 disabled:cursor-not-allowed transition shadow-sm"
            >
              {#if dns.isBusy("resolver")}
                <Icon name="refresh-cw" size={11} class="animate-spin" />
              {:else}
                <Icon name="lock" size={11} />
              {/if}
              Install resolver
            </button>
          {/if}
          <button
            type="button"
            onclick={() => dns.restart()}
            disabled={dns.isBusy("restart")}
            class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md border border-border bg-surface
                   text-[12px] text-fg-muted hover:bg-surface-2 hover:text-fg transition-colors
                   disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {#if dns.isBusy("restart")}
              <Icon name="refresh-cw" size={11} class="animate-spin" />
            {:else}
              <Icon name="refresh-cw" size={11} />
            {/if}
            Restart
          </button>
        </div>
      </header>

      <div class="px-8 py-6 space-y-4 max-w-3xl">
        <!-- First-run setup / routing health -->
        {#if dns.preflight}
          {@const pf = dns.preflight}
          <article
            class="rounded-2xl px-5 py-4 border {pf.ready
              ? 'bg-status-running/5 border-status-running/30'
              : 'bg-status-unhealthy/5 border-status-unhealthy/30'}"
          >
            <header class="flex items-center justify-between gap-3 mb-3">
              <div class="flex items-center gap-2 min-w-0">
                <Icon
                  name={pf.ready ? "circle-check" : "circle-alert"}
                  size={15}
                  class={pf.ready ? "text-status-running" : "text-status-unhealthy"}
                />
                <h3 class="text-[13px] font-semibold text-fg">
                  {pf.ready ? "Local DNS is set up" : "Set up local DNS"}
                </h3>
              </div>
              {#if !pf.ready}
                <button
                  type="button"
                  onclick={() => dns.setupLocalDns()}
                  disabled={dns.isBusy("setup")}
                  class="shrink-0 inline-flex items-center gap-1.5 h-8 px-3.5 rounded-md text-[12px] font-medium
                         text-on-accent bg-accent hover:brightness-110 active:brightness-95
                         disabled:opacity-50 disabled:cursor-not-allowed transition shadow-sm"
                >
                  {#if dns.isBusy("setup")}
                    <Icon name="refresh-cw" size={11} class="animate-spin" />
                    Setting up…
                  {:else}
                    <Icon name="lock" size={11} />
                    Set up local DNS
                  {/if}
                </button>
              {/if}
            </header>

            <p class="text-[11.5px] text-fg-muted leading-relaxed mb-3">
              {#if pf.ready}
                <code class="font-mono">*.{pf.suffix}</code> resolves to this machine
                via PortBay's resolver on port {pf.dnsmasqPort}.
              {:else}
                One macOS password prompt installs PortBay's privileged helper; it
                then routes <code class="font-mono">*.{pf.suffix}</code> here with no
                further prompts.
              {/if}
            </p>

            {#snippet check(label: string, ok: boolean)}
              <div class="flex items-center gap-2 text-[12px]">
                <Icon
                  name={ok ? "circle-check" : "circle-stop"}
                  size={13}
                  class={ok ? "text-status-running" : "text-fg-subtle"}
                />
                <span class={ok ? "text-fg" : "text-fg-muted"}>{label}</span>
              </div>
            {/snippet}
            <div class="grid grid-cols-1 sm:grid-cols-3 gap-1.5">
              {@render check("Privileged helper", pf.helperInstalled)}
              {@render check("Resolver installed", pf.resolverInstalled)}
              {@render check("dnsmasq running", pf.dnsmasqRunning)}
            </div>

            {#if !pf.ready && (pf.port80InUse || pf.port443InUse)}
              <p
                class="mt-3 text-[11px] text-status-port-conflict leading-relaxed flex items-start gap-1.5"
              >
                <Icon name="info" size={12} class="mt-0.5 shrink-0" />
                <span>
                  Ports {pf.port80InUse ? "80" : ""}{pf.port80InUse && pf.port443InUse
                    ? " and "
                    : ""}{pf.port443InUse ? "443" : ""} are already in use. If your
                  site shows another app's page, stop the other local web server
                  (e.g. ServBay/Herd/Valet) so PortBay can serve it.
                </span>
              </p>
            {/if}
          </article>
        {/if}

        <!-- Domain suffix -->
        <article class="bg-surface border border-border/70 rounded-2xl px-5 py-4">
          <header class="flex items-center gap-2 mb-3.5">
            <Icon name="link" size={13} class="text-fg-muted" />
            <h3 class="text-[13px] font-semibold text-fg">Domain suffix</h3>
          </header>
          <p class="text-[11.5px] text-fg-muted mb-3 leading-relaxed">
            Projects resolve at <code class="font-mono">&lt;project&gt;.{dns.status?.suffix ?? "portbay.test"}</code>.
            Changing this renames every hostname and reissues HTTPS certs.
          </p>
          <div class="flex items-stretch gap-1.5">
            <input
              type="text"
              bind:value={suffixInput}
              spellcheck="false"
              autocapitalize="off"
              autocorrect="off"
              class="flex-1 min-w-0 px-3 h-9 rounded-md bg-surface-2/60 border border-border/60
                     text-[12px] font-mono text-fg focus:outline-none focus:ring-1 focus:ring-accent/50"
            />
            <button
              type="button"
              onclick={saveSuffix}
              disabled={!suffixDirty || dns.isBusy("suffix")}
              class="shrink-0 inline-flex items-center gap-1.5 h-9 px-3 rounded-md text-[12px] font-medium
                     text-on-accent bg-accent hover:brightness-110 active:brightness-95
                     disabled:opacity-40 disabled:cursor-not-allowed transition"
            >
              {#if dns.isBusy("suffix")}
                <Icon name="refresh-cw" size={11} class="animate-spin" />
              {/if}
              Apply
            </button>
          </div>
        </article>

        <!-- Editable tuning -->
        <article class="bg-surface border border-border/70 rounded-2xl px-5 py-4">
          <header class="flex items-center justify-between gap-2 mb-3.5">
            <div class="flex items-center gap-2">
              <Icon name="settings" size={13} class="text-fg-muted" />
              <h3 class="text-[13px] font-semibold text-fg">Resolver tuning</h3>
            </div>
            <button
              type="button"
              onclick={saveSettings}
              disabled={!settingsDirty || dns.isBusy("settings")}
              class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md text-[12px] font-medium
                     text-on-accent bg-accent hover:brightness-110 active:brightness-95
                     disabled:opacity-40 disabled:cursor-not-allowed transition"
            >
              {#if dns.isBusy("settings")}
                <Icon name="refresh-cw" size={11} class="animate-spin" />
              {/if}
              Save
            </button>
          </header>
          <div class="grid grid-cols-1 md:grid-cols-2 gap-x-5 gap-y-4">
            <label class="min-w-0 block">
              <span class="block text-[11px] font-medium text-fg-muted mb-1.5">
                Cache size
              </span>
              <input
                type="number"
                min="0"
                max={MAX_DNS_CACHE_SIZE}
                bind:value={cacheSize}
                class="w-full px-3 h-9 rounded-md bg-surface-2/60 border border-border/60
                       text-[12px] font-mono text-fg focus:outline-none focus:ring-1 focus:ring-accent/50"
              />
              <span class="block text-[10.5px] text-fg-subtle mt-1">
                Names dnsmasq caches. 0 disables caching.
              </span>
            </label>
            <label class="min-w-0 block">
              <span class="block text-[11px] font-medium text-fg-muted mb-1.5">
                Local TTL (seconds)
              </span>
              <input
                type="number"
                min="0"
                max={MAX_DNS_LOCAL_TTL}
                bind:value={localTtl}
                class="w-full px-3 h-9 rounded-md bg-surface-2/60 border border-border/60
                       text-[12px] font-mono text-fg focus:outline-none focus:ring-1 focus:ring-accent/50"
              />
              <span class="block text-[10.5px] text-fg-subtle mt-1">
                TTL reported for the wildcard. 0 is the safe default.
              </span>
            </label>
          </div>
          <label class="flex items-center gap-2.5 cursor-pointer select-none mt-4">
            <input type="checkbox" bind:checked={disableNeg} class="accent-accent" />
            <span class="text-[12.5px] text-fg">Disable negative cache</span>
            <span class="text-[10.5px] text-fg-subtle">
              (don't cache NXDOMAIN — useful while wiring up a new hostname)
            </span>
          </label>
        </article>

        <!-- Fixed config (read-only) -->
        <article class="bg-surface border border-border/70 rounded-2xl px-5 py-4">
          <header class="flex items-center gap-2 mb-1.5">
            <Icon name="lock" size={13} class="text-fg-muted" />
            <h3 class="text-[13px] font-semibold text-fg">Fixed configuration</h3>
          </header>
          <p class="text-[11px] text-fg-subtle mb-3.5 leading-relaxed">
            PortBay's resolver answers only for its wildcard suffix on loopback.
            These are locked for safety and can't be changed.
          </p>
          <dl class="space-y-2.5 text-[12px]">
            {#snippet fixed(label: string, value: string, note: string)}
              <div class="flex items-start justify-between gap-4">
                <div class="min-w-0">
                  <dt class="text-fg">{label}</dt>
                  <dd class="text-[10.5px] text-fg-subtle leading-snug">{note}</dd>
                </div>
                <span class="shrink-0 font-mono text-[11.5px] text-fg-muted">{value}</span>
              </div>
            {/snippet}
            {@render fixed(
              "DNS port",
              String(dns.status?.currentPort ?? "—"),
              "Auto-picked free loopback port; the resolver file is kept in sync.",
            )}
            {@render fixed(
              "Listening address",
              "127.0.0.1",
              "Loopback only — never exposed to the LAN.",
            )}
            {@render fixed("Bind interface", "on", "bind-interfaces — stays off other interfaces.")}
            {@render fixed("Read hosts file", "off", "no-hosts — PortBay manages /etc/hosts separately.")}
            {@render fixed(
              "Upstream server",
              "none",
              "no-resolv — only *.suffix is answered; nothing is forwarded.",
            )}
          </dl>
        </article>
      </div>
    {:else if selection.type === "record" && selectedRecord}
      {@const rec = selectedRecord}
      {@const proj = projectFor(rec.projectId)}
      <header class="px-8 pt-7 pb-5 border-b border-border/70">
        <div class="flex items-center gap-3">
          <span
            class="shrink-0 inline-flex items-center justify-center w-9 h-9 rounded-lg bg-surface-2 text-accent"
          >
            <Icon name={rec.kind === "wildcard" ? "star" : "link"} size={18} />
          </span>
          <div class="min-w-0">
            <h1 class="text-[18px] font-semibold tracking-tight text-fg font-mono truncate">
              {rec.hostname}
            </h1>
            <p class="mt-0.5 text-[12px] text-fg-muted">
              {rec.kind === "wildcard"
                ? "Wildcard record — matches every subdomain of the suffix."
                : "Project hostname."}
            </p>
          </div>
        </div>
      </header>
      <div class="px-8 py-6 space-y-4 max-w-2xl">
        <article class="bg-surface border border-border/70 rounded-2xl px-5 py-4">
          <dl class="space-y-3 text-[12.5px]">
            <div class="flex items-center justify-between gap-4">
              <dt class="text-fg-muted">Resolves to</dt>
              <dd class="font-mono text-fg">{rec.target}</dd>
            </div>
            <div class="flex items-center justify-between gap-4">
              <dt class="text-fg-muted">Routed via</dt>
              <dd>
                <span
                  class="inline-flex items-center px-2 py-0.5 rounded-md text-[11px] font-medium
                         {rec.routedVia === 'dnsmasq'
                    ? 'bg-status-running/15 text-status-running'
                    : 'bg-fg-subtle/15 text-fg-subtle'}"
                >
                  {rec.routedVia === "dnsmasq" ? "dnsmasq wildcard" : "/etc/hosts"}
                </span>
              </dd>
            </div>
            {#if proj}
              <div class="flex items-center justify-between gap-4">
                <dt class="text-fg-muted">Project</dt>
                <dd class="flex items-center gap-2">
                  <StatusDot status={proj.status} size="sm" />
                  <span class="text-fg">{proj.name}</span>
                </dd>
              </div>
            {/if}
          </dl>
          {#if proj}
            <div class="flex items-center gap-2 mt-4 pt-4 border-t border-border/60">
              <button
                type="button"
                onclick={() => openUrl(proj.url)}
                class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md text-[12px] font-medium
                       text-on-accent bg-accent hover:brightness-110 transition shadow-sm"
              >
                <Icon name="external-link" size={11} />
                Open {proj.url}
              </button>
              <button
                type="button"
                onclick={() => projectDetailPanel.show(proj.id)}
                class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md border border-border bg-surface
                       text-[12px] text-fg-muted hover:bg-surface-2 hover:text-fg transition-colors"
              >
                <Icon name="chevron-right" size={11} />
                Project details
              </button>
            </div>
          {:else if rec.kind === "wildcard"}
            <p class="text-[11.5px] text-fg-subtle mt-4 pt-4 border-t border-border/60 leading-relaxed">
              {dns.dnsRouting
                ? "Active — install of the resolver file routes this suffix to dnsmasq."
                : "Inactive until the resolver file is installed. Until then, hostnames resolve via /etc/hosts."}
            </p>
          {/if}
        </article>
      </div>
    {:else if selection.type === "hosts" && selectedHost}
      {@const h = selectedHost}
      <header class="px-8 pt-7 pb-5 border-b border-border/70">
        <div class="flex items-center gap-3">
          <span
            class="shrink-0 inline-flex items-center justify-center w-9 h-9 rounded-lg bg-surface-2 text-accent"
          >
            <Icon name="file-text" size={18} />
          </span>
          <div class="min-w-0">
            <h1 class="text-[18px] font-semibold tracking-tight text-fg font-mono truncate">
              {h.hostname}
            </h1>
            <p class="mt-0.5 text-[12px] text-fg-muted">
              Managed <code class="font-mono">/etc/hosts</code> entry.
            </p>
          </div>
        </div>
      </header>
      <div class="px-8 py-6 space-y-4 max-w-2xl">
        <article class="bg-surface border border-border/70 rounded-2xl px-5 py-4">
          <span class="block text-[11px] font-medium text-fg-muted mb-1.5">Entry</span>
          <div class="flex items-stretch gap-1.5">
            <input
              type="text"
              value={`${h.ip}  ${h.hostname}`}
              readonly
              class="flex-1 min-w-0 px-3 h-9 rounded-md bg-surface-2/60 border border-border/60
                     text-[12px] font-mono text-fg focus:outline-none focus:ring-1 focus:ring-accent/50"
            />
            <button
              type="button"
              onclick={() => copy(`${h.ip}  ${h.hostname}`)}
              title={copied === `${h.ip}  ${h.hostname}` ? "Copied!" : "Copy"}
              aria-label="Copy entry"
              class="shrink-0 inline-flex items-center justify-center w-9 h-9 rounded-md border border-border/60
                     text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors
                     {copied === `${h.ip}  ${h.hostname}` ? 'text-status-running' : ''}"
            >
              <Icon name={copied === `${h.ip}  ${h.hostname}` ? "check" : "link"} size={13} />
            </button>
          </div>
          <p class="text-[11px] text-fg-subtle mt-3 leading-relaxed">
            PortBay writes these inside its <code class="font-mono"># BEGIN/END PortBay</code>
            block. They're reconciled from your projects — edit hostnames in the project
            detail panel, not here.
          </p>
        </article>
      </div>
    {:else}
      <div class="h-full flex items-center justify-center">
        <div class="text-center max-w-sm px-6">
          <Icon name="globe" size={28} class="text-fg-subtle mx-auto" />
          <p class="mt-3 text-[13px] text-fg-muted">Select an item from the sidebar.</p>
        </div>
      </div>
    {/if}
  </section>
</div>
