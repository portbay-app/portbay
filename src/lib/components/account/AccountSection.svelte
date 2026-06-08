<!--
  AccountSection — the account + plan surface for Settings. Shows the current
  tier, signed-in identity, a project-usage meter, the right next step (sign in,
  upgrade, or sign out), and — when signed in — the profile (avatar + display
  name) as a section of this same card. Mirrors the other Settings <section> cards.
-->
<script lang="ts">
  import Icon from "$lib/components/atoms/Icon.svelte";
  import UserAvatar from "$lib/components/shell/UserAvatar.svelte";
  import { invokeQuiet, safeInvoke } from "$lib/ipc";
  import { entitlements } from "$lib/stores/entitlements.svelte";
  import { account } from "$lib/stores/account.svelte";
  import { licenseDialog } from "$lib/stores/licenseDialog.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import { confirmDialog } from "$lib/stores/confirm.svelte";
  import type { AccountStatus, SubscriptionStatus } from "$lib/types/entitlements";

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

  // ── Profile (avatar + display name) — folded into this card; the markup below
  //    only renders it when signed in. ──
  const acct = $derived(entitlements.account);
  // Only a custom upload routes through the issuer's /avatar/ endpoint; a GitHub
  // photo is a github CDN URL. Only offer "Remove" for a custom one.
  const hasCustomAvatar = $derived(!!acct?.avatar_url?.includes("/avatar/"));
  let nameDraft = $state("");
  let seededFor = $state<string | null>(null);
  let profileBusy = $state(false);

  // Seed the name field when the signed-in identity changes (sign in/out/switch)
  // — not on every entitlement resync — so it shows the current display name
  // without clobbering an in-progress edit. (ProfileSection got this for free by
  // only mounting while signed in; this card is always mounted.)
  $effect(() => {
    const login = acct?.login ?? null;
    if (login !== seededFor) {
      seededFor = login;
      nameDraft = acct?.display_name ?? "";
    }
  });

  async function saveName() {
    const next = nameDraft.trim();
    if (next === (acct?.display_name ?? "")) return;
    profileBusy = true;
    try {
      await entitlements.updateDisplayName(next || null);
    } finally {
      profileBusy = false;
    }
  }

  async function pickAvatar() {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const result = await open({
        multiple: false,
        directory: false,
        title: "Choose a profile picture",
        filters: [{ name: "Images", extensions: ["png", "jpg", "jpeg", "webp"] }],
      });
      if (typeof result !== "string") return;
      profileBusy = true;
      try {
        await entitlements.uploadAvatar(result);
      } finally {
        profileBusy = false;
      }
    } catch {
      /* dialog plugin already toasted */
    }
  }

  async function clearAvatar() {
    profileBusy = true;
    try {
      await entitlements.removeAvatar();
    } finally {
      profileBusy = false;
    }
  }

  async function signOut() {
    const ok = await confirmDialog.open({
      title: "Sign out of PortBay?",
      message:
        "Your projects stay on this Mac — signing out only disconnects this account. You'll drop to 3 projects until you sign back in.",
      actions: [{ label: "Sign out", value: "out", tone: "destructive", icon: "log-out" }],
    });
    if (ok === "out") await entitlements.logout();
  }

  // ── Billing (subscription-Pro only: renewal/cancel state + Paddle portal) ──
  const hasBilling = $derived(entitlements.hasManagedBilling);
  let subscription = $state<SubscriptionStatus | null>(null);
  let portalBusy = $state(false);

  // Pull the renewal/cancel state whenever billing management applies (sign-in,
  // upgrade, resync). Quiet — a fetch failure just leaves the status line off.
  $effect(() => {
    if (!hasBilling) {
      subscription = null;
      return;
    }
    void entitlements.fetchSubscription().then((s) => (subscription = s));
  });

  function periodEndLabel(iso: string | null): string | null {
    if (!iso) return null;
    const d = new Date(iso);
    if (Number.isNaN(d.getTime())) return null;
    return d.toLocaleDateString(undefined, { month: "long", day: "numeric", year: "numeric" });
  }

  async function openPortal(kind: "overview" | "cancel" | "payment") {
    portalBusy = true;
    try {
      await entitlements.openBillingPortal(kind);
    } catch {
      /* safeInvoke toasted */
    } finally {
      portalBusy = false;
    }
  }

  // ── Data & privacy (GDPR export + erasure) ──
  let exportBusy = $state(false);
  let exported = $state(false);
  let deleteBusy = $state(false);
  let cancelBusy = $state(false);

  // Pending-deletion status. A deletion request survives sign-in by design,
  // so after signing back in this card must show the erasure countdown and a
  // "Cancel deletion" action — not pretend nothing happened. Quiet fetch:
  // offline / signed-out just means no banner.
  let accStatus = $state<AccountStatus | null>(null);
  $effect(() => {
    if (!entitlements.isSignedIn) {
      accStatus = null;
      return;
    }
    void acct?.login; // re-fetch when the signed-in identity changes
    invokeQuiet<AccountStatus>("account_status")
      .then((s) => (accStatus = s))
      .catch(() => {});
  });
  const deletionPending = $derived(!!accStatus?.deletion_requested_at);
  const purgeDate = $derived(accStatus?.purge_after ? new Date(accStatus.purge_after) : null);
  // Whole days until the purge (floored at 0 — the cron may not have swept yet).
  const purgeDaysLeft = $derived(
    purgeDate ? Math.max(0, Math.ceil((purgeDate.getTime() - Date.now()) / 86_400_000)) : null,
  );

  async function cancelDeletion() {
    cancelBusy = true;
    try {
      accStatus = await safeInvoke<AccountStatus>("cancel_account_deletion");
    } catch {
      /* safeInvoke toasted; banner stays up */
    } finally {
      cancelBusy = false;
    }
  }

  async function exportData() {
    const { save } = await import("@tauri-apps/plugin-dialog");
    const dest = await save({
      title: "Export account data",
      defaultPath: "portbay-account-export.json",
      filters: [{ name: "JSON", extensions: ["json"] }],
    });
    if (typeof dest !== "string") return;
    exportBusy = true;
    try {
      await entitlements.exportAccountData(dest);
      exported = true;
      setTimeout(() => (exported = false), 2500);
    } catch {
      /* safeInvoke toasted */
    } finally {
      exportBusy = false;
    }
  }

  async function deleteAccount() {
    const ok = await confirmDialog.open({
      title: "Delete your PortBay account?",
      message:
        "You'll be signed out everywhere now, and your account, license, devices, and encrypted sync data " +
        "will be permanently erased from our servers 30 days from now. Changed your mind? Sign back in within " +
        "those 30 days and choose Cancel deletion here in Settings. " +
        "Your projects and everything on this Mac stay untouched — you'll just drop back to the anonymous 3-project tier. " +
        "An active Pro subscription should be cancelled first (Billing → Manage billing, or the link in your Paddle receipt email), or it will keep renewing.",
      actions: [
        { label: "Delete my account", value: "delete", tone: "destructive", icon: "trash-2" },
      ],
    });
    if (ok !== "delete") return;
    deleteBusy = true;
    try {
      await entitlements.deleteAccount();
    } catch {
      /* safeInvoke toasted; still signed in */
    } finally {
      deleteBusy = false;
    }
  }
</script>

<section class="bg-surface border border-border rounded-2xl p-5">
  {#snippet tierBadge()}
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
  {/snippet}

  <div class="space-y-4">
    {#if entitlements.isSignedIn}
      <!-- Profile first: avatar + display name + tier, then the avatar controls.
           The editable display-name field follows directly below. -->
      <div class="flex items-start gap-4">
        <UserAvatar size={56} />
        <div class="min-w-0 flex-1 space-y-2">
          <div class="flex items-start justify-between gap-3">
            <div class="min-w-0">
              <div class="text-[14px] font-semibold text-fg truncate">
                {acct?.display_name || acct?.login}
              </div>
              <div class="text-[12px] text-fg-muted truncate">
                {tier === "pro" ? "Thanks for supporting PortBay." : "Free account"}
              </div>
            </div>
            {@render tierBadge()}
          </div>
          <div class="flex flex-wrap items-center gap-x-2 gap-y-1">
            <button
              type="button"
              onclick={pickAvatar}
              disabled={profileBusy}
              class="h-8 px-3 rounded-md border border-border text-[12px] text-fg-muted
                     hover:text-fg hover:bg-surface-2 transition-colors disabled:opacity-50"
            >
              Upload picture
            </button>
            {#if hasCustomAvatar}
              <button
                type="button"
                onclick={clearAvatar}
                disabled={profileBusy}
                class="h-8 px-3 rounded-md text-[12px] text-fg-subtle
                       hover:text-status-crashed transition-colors disabled:opacity-50"
              >
                Remove
              </button>
            {/if}
            <span class="text-[11px] text-fg-subtle">PNG, JPEG, or WebP — up to 256 KB.</span>
          </div>
        </div>
      </div>

      <!-- display name -->
      <div class="flex items-center justify-between gap-3">
        <div class="min-w-0">
          <span class="text-[13px] text-fg">Display name</span>
          <p class="text-[11px] text-fg-subtle mt-0.5">
            Shown in the app and used for your initials when there's no picture.
          </p>
        </div>
        <input
          type="text"
          bind:value={nameDraft}
          onblur={saveName}
          onkeydown={(e) => {
            if (e.key === "Enter") (e.currentTarget as HTMLInputElement).blur();
          }}
          disabled={profileBusy}
          maxlength="60"
          placeholder={acct?.login ?? "Your name"}
          class="h-8 w-56 rounded-md bg-bg border border-border px-2.5 text-[12px] text-fg
                 focus:outline-none focus:border-accent/60 disabled:opacity-50"
        />
      </div>
    {:else}
      <!-- anonymous identity + tier -->
      <div class="flex items-center justify-between gap-3">
        <div class="min-w-0">
          <div class="text-[13.5px] font-medium text-fg">Using PortBay anonymously</div>
          <div class="text-[12px] text-fg-muted">No account — up to 3 projects on this Mac.</div>
        </div>
        {@render tierBadge()}
      </div>
    {/if}

    {#if deletionPending}
      <!-- Pending erasure: a deletion request survives sign-in, so show the
           countdown and the explicit way out instead of a business-as-usual card. -->
      <div class="rounded-xl border border-status-crashed/40 bg-status-crashed/10 p-3.5 space-y-2.5">
        <div class="flex items-start gap-2">
          <Icon name="trash-2" size={14} class="mt-0.5 shrink-0 text-status-crashed" />
          <p class="min-w-0 text-[12.5px] leading-relaxed text-fg-muted">
            <span class="font-semibold text-status-crashed">Account deletion scheduled.</span>
            Your account, license, devices, and encrypted sync data will be permanently erased
            {#if purgeDaysLeft !== null && purgeDate}
              in <span class="font-semibold text-fg tabular-nums">{purgeDaysLeft}
                {purgeDaysLeft === 1 ? "day" : "days"}</span>
              — on {purgeDate.toLocaleDateString(undefined, {
                year: "numeric",
                month: "long",
                day: "numeric",
              })}.
            {:else}
              after the {accStatus?.grace_days ?? 30}-day grace window.
            {/if}
            Projects on this Mac are never touched.
          </p>
        </div>
        <button
          type="button"
          onclick={cancelDeletion}
          disabled={cancelBusy}
          class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md bg-accent text-on-accent text-[12px] font-semibold
                 hover:brightness-110 active:brightness-95 transition shadow-sm disabled:opacity-50"
        >
          <Icon name="rotate-ccw" size={12} />
          {cancelBusy ? "Cancelling…" : "Cancel deletion — keep my account"}
        </button>
      </div>
    {/if}

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

    {#if hasBilling}
      <!-- billing: subscription state + Paddle customer portal (MoR) -->
      <div class="border-t border-border/60 pt-3">
        <div class="text-[12px] font-medium text-fg-muted mb-2">Billing</div>
        {#if subscription?.status === "past_due"}
          <p class="text-[12px] leading-relaxed text-status-unhealthy flex items-start gap-1.5 mb-2">
            <Icon name="circle-alert" size={13} class="mt-px shrink-0" />
            <span>Payment past due — update your payment method to keep Pro.</span>
          </p>
        {:else if subscription?.cancelAtPeriodEnd}
          <p class="text-[12px] text-fg-muted mb-2">
            Your subscription is set to cancel{#if periodEndLabel(subscription.currentPeriodEnd)}
              &nbsp;— Pro stays active until
              <span class="text-fg">{periodEndLabel(subscription.currentPeriodEnd)}</span>{/if}.
          </p>
        {:else if subscription?.status === "trialing" && periodEndLabel(subscription.currentPeriodEnd)}
          <p class="text-[12px] text-fg-muted mb-2">
            Free trial — first charge on
            <span class="text-fg">{periodEndLabel(subscription.currentPeriodEnd)}</span>
            · $10/mo
          </p>
        {:else if subscription?.status === "active" && periodEndLabel(subscription.currentPeriodEnd)}
          <p class="text-[12px] text-fg-muted mb-2">
            Renews on <span class="text-fg">{periodEndLabel(subscription.currentPeriodEnd)}</span>
            · $10/mo
          </p>
        {/if}
        <div class="flex flex-wrap items-center gap-x-2 gap-y-1">
          {#if subscription?.status === "past_due"}
            <button
              type="button"
              onclick={() => openPortal("payment")}
              disabled={portalBusy || isGrace}
              class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md bg-accent text-on-accent text-[12px] font-semibold
                     hover:brightness-110 active:brightness-95 transition shadow-sm disabled:opacity-50"
            >
              <Icon name="credit-card" size={12} /> Update payment method
            </button>
          {/if}
          <button
            type="button"
            onclick={() => openPortal("overview")}
            disabled={portalBusy || isGrace}
            class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md border border-border text-[12px] text-fg-muted
                   hover:text-fg hover:bg-surface-2 transition-colors disabled:opacity-50"
          >
            <Icon name="credit-card" size={12} />
            {portalBusy ? "Opening…" : "Manage billing"}
          </button>
          {#if !subscription?.cancelAtPeriodEnd}
            <button
              type="button"
              onclick={() => openPortal("cancel")}
              disabled={portalBusy || isGrace}
              class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md text-[12px] text-fg-subtle
                     hover:text-status-crashed transition-colors disabled:opacity-50"
            >
              Cancel subscription
            </button>
          {/if}
        </div>
        <p class="mt-1.5 text-[11px] text-fg-subtle">
          {#if isGrace}
            Billing needs a connection — reconnect to manage your subscription.
          {:else}
            Opens the secure Paddle portal in your browser — payment method, invoices, and
            cancellation. Cancelling stops the next renewal; Pro stays active until the end of
            the period you've paid for.
          {/if}
        </p>
      </div>
    {:else if tier === "pro" && entitlements.source && entitlements.source !== "subscription"}
      <!-- perpetual Pro (contribute / donate / manual) — nothing to bill -->
      <div class="border-t border-border/60 pt-3">
        <div class="text-[12px] font-medium text-fg-muted mb-1">Billing</div>
        <p class="text-[11.5px] text-fg-subtle">
          Your Pro was granted
          {entitlements.source === "contribute"
            ? "for a merged contribution"
            : entitlements.source === "donate"
              ? "for a donation"
              : "manually"} — it doesn't renew and there's no billing to manage.
        </p>
      </div>
    {/if}

    {#if entitlements.isSignedIn}
      <!-- data & privacy: GDPR export (Art. 20) + account erasure (Art. 17) -->
      <div class="border-t border-border/60 pt-3">
        <div class="text-[12px] font-medium text-fg-muted mb-2">Data &amp; privacy</div>
        <div class="flex flex-wrap items-center gap-x-2 gap-y-1">
          <button
            type="button"
            onclick={exportData}
            disabled={exportBusy || deleteBusy}
            class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md border border-border text-[12px] text-fg-muted
                   hover:text-fg hover:bg-surface-2 transition-colors disabled:opacity-50"
          >
            <Icon name={exported ? "check" : "save"} size={12} />
            {exported ? "Exported" : exportBusy ? "Exporting…" : "Export my data"}
          </button>
          {#if !deletionPending}
            <button
              type="button"
              onclick={deleteAccount}
              disabled={exportBusy || deleteBusy}
              class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md text-[12px] text-fg-subtle
                     hover:text-status-crashed transition-colors disabled:opacity-50"
            >
              <Icon name="trash-2" size={12} />
              {deleteBusy ? "Deleting…" : "Delete account"}
            </button>
          {/if}
        </div>
        <p class="mt-1.5 text-[11px] text-fg-subtle">
          {#if deletionPending}
            Export downloads everything we store about your account as JSON — still available
            while the deletion above is pending.
          {:else}
            Export downloads everything we store about your account as JSON. Deleting signs you
            out and erases it from our servers after a 30-day grace window — sign back in and
            choose Cancel deletion here to keep it. Projects on this Mac are never touched.
          {/if}
        </p>
      </div>
    {/if}

    <!-- about the license -->
    <div class="border-t border-border/60 pt-3">
      <button
        type="button"
        onclick={() => licenseDialog.open()}
        class="inline-flex items-center gap-1 text-[11.5px] font-medium text-accent hover:underline"
      >
        <Icon name="info" size={12} /> What's in Pro &amp; how licensing works
      </button>
    </div>
  </div>
</section>
