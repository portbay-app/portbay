<!-- DomainsPanel — default domain suffix (Pro), hosts management, CA, cert renew. -->
<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "$lib/components/atoms/Icon.svelte";
  import Toggle from "$lib/components/atoms/Toggle.svelte";
  import { preferences } from "$lib/stores/preferences.svelte";
  import { entitlements } from "$lib/stores/entitlements.svelte";
  import { account } from "$lib/stores/account.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import { errorBus } from "$lib/stores/errors.svelte";
  import { safeInvoke } from "$lib/ipc";
  import type { DomainMigration } from "$lib/types/dns";
  import SettingsPanel from "./SettingsPanel.svelte";

  interface DomainSettings {
    domainSuffix: string;
    projectCount: number;
  }
  let domainSettings = $state<DomainSettings | null>(null);
  let domainDraft = $state<string>("test");
  let domainBusy = $state<boolean>(false);

  // Changing the default domain suffix is a Pro capability. The community tiers
  // (anonymous / free) stay pinned to the configured suffix — important for the
  // beta, where every tester shares the same `.portbay.test` routing.
  const canEditDomainSuffix = $derived(entitlements.isPro);

  async function refreshDomainSettings() {
    try {
      domainSettings = await safeInvoke<DomainSettings>("get_domain_settings");
      domainDraft = domainSettings.domainSuffix;
    } catch {
      domainSettings = null;
    }
  }

  async function saveDomainSuffix() {
    if (!canEditDomainSuffix) return;
    const next = domainDraft.trim().replace(/^\./, "");
    if (!next || next === domainSettings?.domainSuffix) return;
    domainBusy = true;
    try {
      const migration = await safeInvoke<DomainMigration>("update_domain_suffix", {
        domainSuffix: next,
      });
      errorBus.push({
        code: "DOMAIN_SUFFIX_UPDATED",
        whatHappened: `Domain suffix changed from .${migration.oldSuffix} to .${migration.newSuffix}.`,
        whyItMatters: `${migration.changedProjects} project hostname(s) migrated and now resolve via /etc/hosts; Caddy + certificates update automatically. For wildcard *.${migration.newSuffix} subdomains, re-run “Set up local DNS” on the DNS page.`,
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });
      // Reflect the migrated hostnames everywhere (project list, detail panel,
      // domains page) — the registry changed under the running stores.
      await refreshDomainSettings();
      await projects.refresh();
    } catch {
      /* toast */
    } finally {
      domainBusy = false;
    }
  }

  onMount(() => {
    void refreshDomainSettings();
  });
</script>

<SettingsPanel
  title="Domains & HTTPS"
  description="The local domain suffix every project uses, plus hosts-file and certificate handling."
>
  <div class="divide-y divide-border/60">
    <div class="flex items-start justify-between gap-3 py-2.5 first:pt-0">
      <div class="min-w-0">
        <span class="text-[13px] text-fg">Default domain suffix</span>
        <p class="mt-1 text-[11px] text-fg-subtle leading-relaxed max-w-md">
          Applies to every project — existing hostnames migrate automatically.
          They resolve via <code class="font-mono">/etc/hosts</code> on any
          suffix; for wildcard <code class="font-mono">*.suffix</code>
          subdomains, run “Set up local DNS” on the DNS page. Use a local-only
          suffix like <code class="font-mono">test</code> or
          <code class="font-mono">portbay.test</code> — public TLDs are rejected.
        </p>
        {#if !canEditDomainSuffix}
          <p class="mt-1.5 text-[11px] text-accent/90">
            Changing the suffix is a <span class="font-medium">Pro</span> feature — community
            projects stay on
            <code class="font-mono">.{domainSettings?.domainSuffix ?? domainDraft}</code>.
          </p>
        {/if}
      </div>
      {#if canEditDomainSuffix}
        <div class="flex items-center gap-2 shrink-0">
          <div class="relative">
            <span
              class="absolute left-2.5 top-1/2 -translate-y-1/2 text-fg-subtle text-[12px]"
            >.</span>
            <input
              type="text"
              value={domainDraft}
              oninput={(e) =>
                (domainDraft = (e.currentTarget as HTMLInputElement).value)}
              onkeydown={(e) => {
                if (e.key === "Enter") void saveDomainSuffix();
              }}
              placeholder="test"
              class="h-8 w-44 rounded-md bg-bg border border-border pl-5 pr-2 text-[12px] text-fg font-mono focus:outline-none focus:border-accent/60"
            />
          </div>
          <button
            type="button"
            onclick={saveDomainSuffix}
            disabled={domainBusy || !domainDraft.trim() ||
              domainDraft.trim().replace(/^\./, "") === domainSettings?.domainSuffix}
            class="h-8 px-3 rounded-md text-[12px] text-accent border border-accent/40 hover:bg-accent/10 transition-colors disabled:opacity-50"
          >
            {domainBusy ? "Saving…" : "Apply"}
          </button>
        </div>
      {:else}
        <!-- Community tiers can't change the suffix — show it read-only and
             offer an upgrade. Mirrors the Sync (Pro) upsell. -->
        <div class="flex items-center gap-2 shrink-0">
          <span
            class="inline-flex items-center h-8 px-2.5 rounded-md bg-bg border border-border text-[12px] text-fg-muted font-mono"
            title="Changing the domain suffix is a Pro feature"
          >
            .{domainSettings?.domainSuffix ?? domainDraft}
          </span>
          <button
            type="button"
            onclick={() => account.open({ intent: "pro" })}
            class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md text-[12px] font-medium text-accent border border-accent/40 hover:bg-accent/10 transition-colors"
          >
            <Icon name="lock" size={12} /> Pro
          </button>
        </div>
      {/if}
    </div>

    <div class="flex items-center justify-between gap-3 py-2.5">
      <span class="text-[13px] text-fg">Manage hosts file automatically</span>
      <Toggle
        checked={preferences.value.manageHostsAutomatically}
        label="Manage hosts file automatically"
        onchange={(v) => preferences.update({ manageHostsAutomatically: v })}
      />
    </div>

    <div class="flex items-center justify-between gap-3 py-2.5">
      <span class="text-[13px] text-fg">Trust local CA</span>
      <span class="inline-flex items-center gap-1.5 text-[12px]">
        <span
          class="inline-flex items-center justify-center w-4 h-4 rounded-full
                 bg-status-running/15 text-status-running"
        >
          <Icon name="check" size={10} />
        </span>
        <span class="text-status-running font-medium">Trusted</span>
      </span>
    </div>

    <div class="flex items-center justify-between gap-3 py-2.5 last:pb-0">
      <span class="text-[13px] text-fg">Auto-renew local certificates</span>
      <Toggle
        checked={preferences.value.autoRenewCertificates}
        label="Auto-renew local certificates"
        onchange={(v) => preferences.update({ autoRenewCertificates: v })}
      />
    </div>
  </div>
</SettingsPanel>
