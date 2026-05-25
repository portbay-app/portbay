<!--
  AboutLicenseDialog — the Pro License surface ("About License"). PortBay's own
  design, not a FlyEnv clone: a Community-vs-Pro matrix read from the canonical
  feature list, the current entitlement state, the two honest acquisition paths
  (donate / contribute), and the open-source honesty note. Store-driven
  (`licenseDialog`), mounted once at the layout root. The SignInSheet remains the
  focused upgrade CTA; this is the "what is Pro / how does licensing work" view.
-->
<script lang="ts">
  import { openUrl } from "$lib/security/openUrl";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import LighthouseLogo from "$lib/components/atoms/LighthouseLogo.svelte";
  import { licenseDialog } from "$lib/stores/licenseDialog.svelte";
  import { account } from "$lib/stores/account.svelte";
  import { entitlements } from "$lib/stores/entitlements.svelte";
  import {
    PRO_FEATURES,
    DONATE_URL,
    CONTRIBUTE_URL,
    PRIVACY_URL,
    TERMS_URL,
    LICENSE_URL,
  } from "$lib/data/proFeatures";

  let dialogEl = $state<HTMLDivElement | null>(null);
  let lastFocused: HTMLElement | null = null;

  const isPro = $derived(entitlements.isPro);
  const signedIn = $derived(entitlements.isSignedIn);
  const stateLabel = $derived.by(() => {
    switch (entitlements.state) {
      case "pro":
        return "You're on Pro";
      case "pro-grace":
        return "Pro (offline grace)";
      case "free":
        return "Free account";
      default:
        return "Not signed in";
    }
  });

  $effect(() => {
    if (licenseDialog.isOpen) {
      lastFocused = document.activeElement as HTMLElement | null;
      queueMicrotask(() => dialogEl?.querySelector<HTMLElement>("[data-autofocus]")?.focus());
    } else if (lastFocused) {
      lastFocused.focus();
      lastFocused = null;
    }
  });

  function close() {
    licenseDialog.close();
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      close();
    }
  }

  function upgrade() {
    close();
    account.open({ intent: signedIn ? "pro" : "signup" });
  }
</script>

<svelte:window onkeydown={licenseDialog.isOpen ? onKeydown : undefined} />

{#if licenseDialog.isOpen}
  <div class="fixed inset-0 z-[70] bg-black/45 backdrop-blur-sm" onclick={close} role="presentation"></div>
  <div
    bind:this={dialogEl}
    role="dialog"
    aria-modal="true"
    aria-labelledby="license-title"
    class="fixed left-1/2 top-1/2 z-[71] w-[min(540px,calc(100vw-2rem))] max-h-[calc(100vh-3rem)]
           -translate-x-1/2 -translate-y-1/2 rounded-2xl bg-bg border border-border
           shadow-2xl flex flex-col overflow-hidden"
  >
    <!-- header -->
    <div class="relative px-6 pt-6 pb-5 border-b border-border bg-surface/40">
      <button
        type="button"
        onclick={close}
        aria-label="Close"
        class="absolute right-4 top-4 grid place-items-center w-7 h-7 rounded-md text-fg-subtle hover:text-fg hover:bg-surface-2 transition-colors"
      >
        <Icon name="x" size={15} />
      </button>
      <div class="flex items-center gap-3">
        <LighthouseLogo size={34} />
        <div class="min-w-0">
          <h2 id="license-title" class="text-[16px] font-semibold text-fg tracking-tight">PortBay Pro</h2>
          <p class="text-[12.5px] leading-snug text-fg-muted mt-0.5">
            Everything in PortBay is free. Pro unlocks the hosted conveniences and funds the project.
          </p>
        </div>
        <span
          class="ml-auto shrink-0 inline-flex items-center gap-1.5 h-6 px-2.5 rounded-full text-[11px] font-semibold
                 {isPro ? 'bg-accent/15 text-accent' : 'bg-fg-muted/15 text-fg-muted'}"
        >
          {#if isPro}<Icon name="sparkles" size={11} />{/if}{stateLabel}
        </span>
      </div>
    </div>

    <div class="px-6 py-5 overflow-y-auto">
      <!-- feature matrix -->
      <div class="rounded-xl border border-border overflow-hidden">
        <div class="grid grid-cols-[1fr_auto_auto] items-center gap-x-4 px-3.5 py-2 bg-surface/50 text-[11px] uppercase tracking-wide text-fg-subtle">
          <span>Feature</span>
          <span class="text-right w-20">Free</span>
          <span class="text-right w-24 text-accent">Pro</span>
        </div>
        {#each PRO_FEATURES as f (f.key)}
          <div class="grid grid-cols-[1fr_auto_auto] items-center gap-x-4 px-3.5 py-2.5 border-t border-border/60">
            <span class="flex items-center gap-2 text-[13px] text-fg">
              <span class="grid place-items-center w-6 h-6 rounded-md bg-fg-muted/10 text-fg-muted shrink-0">
                <Icon name={f.icon} size={13} />
              </span>
              {f.label}
            </span>
            <span class="text-right w-20 text-[12px] {f.community === '—' ? 'text-fg-subtle' : 'text-fg-muted'} tabular-nums">
              {f.community}
            </span>
            <span class="text-right w-24 text-[12px] font-medium text-fg tabular-nums">{f.pro}</span>
          </div>
        {/each}
      </div>

      <!-- honest framing -->
      <p class="mt-4 text-[12.5px] leading-relaxed text-fg-muted">
        PortBay is open source (Apache-2.0) — you can build any Pro feature yourself for free. Pro is the
        <span class="text-fg font-medium">pay-what-you-want, perpetual</span> way to support the project and get the
        hosted account + sync conveniences without the work. No subscription.
      </p>

      <!-- acquisition / state -->
      {#if isPro}
        <div class="mt-4 flex items-center gap-2.5 rounded-lg bg-accent/10 text-accent px-3.5 py-3">
          <Icon name="sparkles" size={16} />
          <p class="text-[13px] font-medium">Pro is active — thank you for supporting PortBay.</p>
        </div>
      {:else}
        <div class="mt-4 flex flex-col gap-2.5">
          <div class="flex gap-2.5">
            <button
              type="button"
              data-autofocus
              onclick={() => void openUrl(DONATE_URL)}
              class="flex-1 inline-flex items-center justify-center gap-2 h-10 rounded-xl bg-accent text-on-accent text-[13.5px] font-semibold hover:brightness-110 active:brightness-95 transition shadow-sm"
            >
              <Icon name="sparkles" size={14} /> Donate
              <Icon name="external-link" size={12} class="opacity-70" />
            </button>
            <button
              type="button"
              onclick={() => void openUrl(CONTRIBUTE_URL)}
              class="flex-1 inline-flex items-center justify-center gap-2 h-10 rounded-xl border border-border bg-surface text-fg text-[13.5px] font-medium hover:bg-surface-2 transition"
            >
              <Icon name="terminal" size={14} /> Contribute
              <Icon name="external-link" size={12} class="opacity-60" />
            </button>
          </div>
          <button
            type="button"
            onclick={upgrade}
            class="inline-flex items-center justify-center gap-1.5 h-9 rounded-lg text-[12.5px] font-medium text-fg-muted hover:text-fg hover:bg-surface-2 transition"
          >
            {signedIn ? "Upgrade — already donated or contributed?" : "Sign in or create a free account first"}
            <Icon name="arrow-right" size={13} />
          </button>
        </div>
      {/if}
    </div>

    <!-- links / management -->
    <footer class="px-6 py-3.5 border-t border-border bg-surface/40 flex items-center justify-between gap-3 text-[11.5px]">
      <div class="flex items-center gap-3 text-fg-subtle">
        <button type="button" onclick={() => void openUrl(LICENSE_URL)} class="hover:text-fg transition-colors">License</button>
        <button type="button" onclick={() => void openUrl(PRIVACY_URL)} class="hover:text-fg transition-colors">Privacy</button>
        <button type="button" onclick={() => void openUrl(TERMS_URL)} class="hover:text-fg transition-colors">Terms</button>
      </div>
      <span class="text-fg-subtle">Manage account &amp; devices in Settings</span>
    </footer>
  </div>
{/if}
