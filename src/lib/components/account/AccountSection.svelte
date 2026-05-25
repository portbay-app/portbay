<!--
  AccountSection — the account + plan surface for Settings. Shows the current
  tier, signed-in identity, a project-usage meter, and the right next step
  (sign in, upgrade, or sign out). Mirrors the other Settings <section> cards.
-->
<script lang="ts">
  import Icon from "$lib/components/atoms/Icon.svelte";
  import { entitlements } from "$lib/stores/entitlements.svelte";
  import { account } from "$lib/stores/account.svelte";
  import { licenseDialog } from "$lib/stores/licenseDialog.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import { confirmDialog } from "$lib/stores/confirm.svelte";

  const projectCount = $derived(projects.value.length);
  const cap = $derived(entitlements.maxProjects); // null = unlimited
  const tier = $derived(entitlements.tier);
  const isGrace = $derived(entitlements.state === "pro-grace");

  const tierLabel = $derived(
    tier === "pro" ? (isGrace ? "Pro (offline)" : "Pro") : tier === "free" ? "Free" : "Not signed in",
  );

  // Usage meter fill (0–1). Unlimited shows a calm full-but-muted bar.
  const usagePct = $derived(cap === null ? 100 : Math.min(100, Math.round((projectCount / cap) * 100)));
  const nearCap = $derived(cap !== null && projectCount >= cap);

  async function signOut() {
    const ok = await confirmDialog.open({
      title: "Sign out of PortBay?",
      message:
        "Your projects stay on this Mac — signing out only disconnects this account. You'll drop to 3 projects until you sign back in.",
      actions: [{ label: "Sign out", value: "out", tone: "destructive", icon: "log-out" }],
    });
    if (ok === "out") await entitlements.logout();
  }
</script>

<section
  class="bg-surface border border-border rounded-2xl p-5
         grid grid-cols-[180px,1fr] gap-x-6"
>
  <div class="flex items-start gap-2.5">
    <span class="inline-flex items-center justify-center w-8 h-8 rounded-lg bg-fg-muted/10 text-fg-muted">
      <Icon name="users" size={15} />
    </span>
    <span class="text-[14px] font-semibold text-fg pt-1">Account</span>
  </div>

  <div class="space-y-4">
    <!-- identity + tier -->
    <div class="flex items-center justify-between gap-3">
      <div class="min-w-0">
        {#if entitlements.isSignedIn}
          <div class="text-[13.5px] font-medium text-fg truncate">
            {entitlements.account?.login}
          </div>
          <div class="text-[12px] text-fg-muted">
            {tier === "pro" ? "Thanks for supporting PortBay." : "Free account"}
          </div>
        {:else}
          <div class="text-[13.5px] font-medium text-fg">Using PortBay anonymously</div>
          <div class="text-[12px] text-fg-muted">No account — up to 3 projects on this Mac.</div>
        {/if}
      </div>
      <span
        class="shrink-0 inline-flex items-center gap-1.5 h-6 px-2.5 rounded-full text-[11.5px] font-semibold
               {tier === 'pro'
          ? 'bg-accent/15 text-accent'
          : tier === 'free'
            ? 'bg-status-running/15 text-status-running'
            : 'bg-fg-muted/15 text-fg-muted'}"
      >
        {#if tier === "pro"}<Icon name="sparkles" size={12} />{/if}
        {tierLabel}
      </span>
    </div>

    {#if isGrace}
      <p class="text-[12px] leading-relaxed text-status-unhealthy flex items-start gap-1.5">
        <Icon name="circle-alert" size={13} class="mt-px shrink-0" />
        <span>We can't reach the license server. Pro stays active during the offline grace window.</span>
      </p>
    {/if}

    <!-- usage meter -->
    <div>
      <div class="flex items-center justify-between text-[12px] mb-1.5">
        <span class="text-fg-muted">Projects</span>
        <span class="text-fg tabular-nums">
          {projectCount}{cap === null ? "" : ` / ${cap}`}
          {#if cap === null}<span class="text-fg-subtle"> · unlimited</span>{/if}
        </span>
      </div>
      <div class="h-1.5 rounded-full bg-surface-2 overflow-hidden">
        <div
          class="h-full rounded-full transition-[width] {nearCap ? 'bg-status-unhealthy' : 'bg-accent'} {cap ===
          null
            ? 'opacity-40'
            : ''}"
          style:width="{usagePct}%"
        ></div>
      </div>
      {#if nearCap}
        <p class="mt-1.5 text-[11.5px] text-fg-muted">
          {tier === "anonymous"
            ? "Sign in or create a free account to keep adding projects."
            : "You're at your plan's limit — go Pro for unlimited projects."}
        </p>
      {/if}
    </div>

    <!-- primary action -->
    <div class="flex flex-wrap items-center gap-2 pt-1">
      {#if tier === "anonymous"}
        <button
          type="button"
          onclick={() => account.open({ intent: "signin" })}
          class="inline-flex items-center gap-1.5 h-9 px-4 rounded-lg bg-accent text-on-accent text-[13px] font-semibold hover:brightness-110 active:brightness-95 transition shadow-sm"
        >
          Sign in or sign up
        </button>
        <span class="text-[12px] text-fg-subtle">Free — unlocks 6 projects + sync-ready account.</span>
      {:else if tier === "free"}
        <button
          type="button"
          onclick={() => account.open({ intent: "pro" })}
          class="inline-flex items-center gap-1.5 h-9 px-4 rounded-lg bg-accent text-on-accent text-[13px] font-semibold hover:brightness-110 active:brightness-95 transition shadow-sm"
        >
          <Icon name="sparkles" size={13} /> Upgrade to Pro
        </button>
        <button
          type="button"
          onclick={signOut}
          class="inline-flex items-center gap-1.5 h-9 px-3 rounded-lg text-[13px] font-medium text-fg-muted hover:text-fg hover:bg-surface-2 transition"
        >
          <Icon name="log-out" size={13} /> Sign out
        </button>
      {:else}
        <!-- pro -->
        <button
          type="button"
          onclick={signOut}
          class="inline-flex items-center gap-1.5 h-9 px-3 rounded-lg text-[13px] font-medium text-fg-muted hover:text-fg hover:bg-surface-2 transition"
        >
          <Icon name="log-out" size={13} /> Sign out
        </button>
      {/if}
    </div>

    <!-- about the license -->
    <div class="border-t border-border/60 pt-3">
      <p class="text-[11.5px] leading-relaxed text-fg-subtle">
        PortBay Pro is perpetual and pay-what-you-want — earned with a donation or a merged pull request, never a
        subscription. Your projects and data are always yours; a lapsed or revoked license only blocks new gated
        actions, never your existing work.
      </p>
      <button
        type="button"
        onclick={() => licenseDialog.open()}
        class="mt-2 inline-flex items-center gap-1 text-[11.5px] font-medium text-accent hover:underline"
      >
        <Icon name="info" size={12} /> What's in Pro &amp; how licensing works
      </button>
    </div>
  </div>
</section>
