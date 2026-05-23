<!--
  /services — full health surface for the bundled sidecars.

  The dashboard row (`/`) shows the same sidecars in a compact pill
  layout for at-a-glance scanning. This page expands each into a card
  with a status-aware action, the raw detail string, and a "What is
  this?" expander explaining what the sidecar does and why we bundle it.
-->
<script lang="ts">
  import { onMount } from "svelte";

  import { DashboardCard, Icon } from "$lib/components/atoms";
  import { SidecarCard } from "$lib/components/sidecars";
  import { sidecars } from "$lib/stores/sidecars.svelte";
  import type { SidecarKey } from "$lib/types/sidecars";

  /** Top-to-bottom critical-path order: PC + Caddy first (without them
   *  nothing runs), then mkcert (HTTPS), then the optional helpers. */
  const SIDECAR_ORDER: Array<{
    key: SidecarKey;
    why: string;
  }> = [
    {
      key: "processCompose",
      why: "Spawns your dev servers and tails their logs. Without it, no project can start. Bundled — never relies on your $PATH.",
    },
    {
      key: "caddy",
      why: "Reverse-proxies project hostnames (e.g. https://myapp.test) to their localhost ports. Local TLS handled automatically.",
    },
    {
      key: "mkcertCa",
      why: "Installs a locally-trusted certificate authority so your browser accepts .test HTTPS certs without warnings.",
    },
    {
      key: "dnsmasq",
      why: "Wildcard DNS for your domain suffix (e.g. *.test → 127.0.0.1). Optional — /etc/hosts entries fall back when absent.",
    },
    {
      key: "mailpit",
      why: "Catches outgoing SMTP from local apps so you can inspect emails without sending them. Frameworks auto-pick up via injected MAIL_* env vars.",
    },
    {
      key: "hostsHelper",
      why: "Writes managed entries for each hostname inside a # BEGIN PortBay block. Only touched when dnsmasq isn't routing.",
    },
  ];

  let expandedKey = $state<SidecarKey | null>(null);

  onMount(() => {
    sidecars.start();
    return () => sidecars.stop();
  });

  function toggleWhy(k: SidecarKey) {
    expandedKey = expandedKey === k ? null : k;
  }
</script>

<div class="p-6 space-y-4">
  <header class="flex items-center justify-between">
    <div>
      <h2 class="text-lg font-semibold tracking-tight">Services</h2>
      <p class="text-xs text-fg-muted mt-0.5">
        Bundled sidecars that make local HTTPS, hostname routing, and email
        catching work. Polled every 3 seconds.
      </p>
    </div>
    <button
      type="button"
      onclick={() => sidecars.refresh()}
      disabled={sidecars.loading}
      class="inline-flex items-center gap-1.5 text-xs text-fg-muted
             border border-border hover:text-fg hover:bg-surface-2
             rounded-md px-2.5 py-1.5 transition-colors disabled:opacity-50"
    >
      <Icon
        name="refresh-cw"
        size={12}
        class={sidecars.loading ? "animate-spin" : ""}
      />
      Refresh
    </button>
  </header>

  <div class="grid grid-cols-1 lg:grid-cols-2 gap-4">
    {#each SIDECAR_ORDER as item (item.key)}
      {@const info = sidecars.value[item.key]}
      <div class="space-y-2">
        <SidecarCard sidecarKey={item.key} {info} />
        <button
          type="button"
          onclick={() => toggleWhy(item.key)}
          class="inline-flex items-center gap-1 text-[11px] text-fg-subtle
                 hover:text-fg-muted px-1 transition-colors"
        >
          <Icon
            name={expandedKey === item.key ? "chevron-down" : "chevron-right"}
            size={10}
          />
          What is this?
        </button>
        {#if expandedKey === item.key}
          <div
            class="text-xs text-fg-muted leading-relaxed px-3 py-2.5
                   rounded-md border border-border bg-surface"
          >
            {item.why}
          </div>
        {/if}
      </div>
    {/each}
  </div>

  {#if sidecars.lastErrorAt}
    <DashboardCard title="Status" flush>
      <p class="text-xs text-status-unhealthy">
        Couldn't refresh sidecar status — last toast has the details. Will
        retry on the next poll tick.
      </p>
    </DashboardCard>
  {/if}
</div>
