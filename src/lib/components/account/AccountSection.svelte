<!--
  AccountSection — the account + plan surface for Settings. Shows the current
  tier, signed-in identity, a project-usage meter, the right next step (sign in,
  upgrade, or sign out), and — when signed in — the profile (avatar + display
  name) as a section of this same card. Mirrors the other Settings <section> cards.
-->
<script lang="ts">
  import Icon from "$lib/components/atoms/Icon.svelte";
  import UserAvatar from "$lib/components/shell/UserAvatar.svelte";
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
