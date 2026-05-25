<!--
  EarlyAccessSection — opt into experimental features ahead of the stable
  channel (Pro). Matches the other Settings <section> cards. The toggle writes
  the `earlyAccessOptIn` preference (single-writer via the preferences store);
  `flags.enabled(...)` reads entitlement + this opt-in to decide what's live.
  Gated on the `early_access` entitlement.
-->
<script lang="ts">
  import Icon from "$lib/components/atoms/Icon.svelte";
  import Toggle from "$lib/components/atoms/Toggle.svelte";
  import { entitlements } from "$lib/stores/entitlements.svelte";
  import { account } from "$lib/stores/account.svelte";
  import { preferences } from "$lib/stores/preferences.svelte";

  const isPro = $derived(entitlements.allows("early_access"));
  const optedIn = $derived(preferences.value.earlyAccessOptIn);

  async function setOptIn(next: boolean) {
    await preferences.update({ earlyAccessOptIn: next });
  }
</script>

<section class="bg-surface border border-border rounded-2xl p-5 grid grid-cols-[180px,1fr] gap-x-6">
  <div class="flex items-start gap-2.5">
    <span class="inline-flex items-center justify-center w-8 h-8 rounded-lg bg-fg-muted/10 text-fg-muted">
      <Icon name="sparkles" size={15} />
    </span>
    <div class="pt-1">
      <span class="text-[14px] font-semibold text-fg">Early Access</span>
      <span class="block text-[11px] text-fg-subtle mt-0.5">Pro</span>
    </div>
  </div>

  <div class="space-y-4">
    {#if !isPro}
      <div class="flex items-start gap-3">
        <p class="text-[13px] leading-relaxed text-fg-muted flex-1">
          Try new features before they reach everyone. Early access to in-development features is part of
          <span class="text-fg font-medium">PortBay Pro</span>.
        </p>
        <button
          type="button"
          onclick={() => account.open({ intent: "pro" })}
          class="shrink-0 inline-flex items-center gap-1.5 h-9 px-4 rounded-lg bg-accent text-on-accent text-[13px] font-semibold hover:brightness-110 transition shadow-sm"
        >
          <Icon name="sparkles" size={13} /> Upgrade
        </button>
      </div>
    {:else}
      <div class="flex items-center justify-between gap-3">
        <p class="text-[13px] leading-relaxed text-fg-muted flex-1">
          Turn on experimental features as they land. These are still in development — expect rough edges, and
          tell us what breaks.
        </p>
        <Toggle
          checked={optedIn}
          label="Enable early-access features"
          onchange={setOptIn}
        />
      </div>
      {#if optedIn}
        <p class="text-[11.5px] text-fg-subtle">
          Early-access features are on for this account. No experimental features are live yet — you'll see them
          here as they ship.
        </p>
      {/if}
    {/if}
  </div>
</section>
