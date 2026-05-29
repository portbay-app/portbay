<!--
  SignInSheet — the single account surface: sign in / sign up (GitHub or email
  magic-link) and upgrade to Pro. Store-driven (`account`), mounted once at the
  layout root. Adapts its copy to the `intent` a gate opened it with.

  Auth uses the flow+poll handshake in the entitlements store: we kick off the
  flow, the system browser (GitHub) or a magic-link email completes it, and we
  poll until the session lands — all without tokens ever touching the webview.
-->
<script lang="ts">
  import { openUrl } from "$lib/security/openUrl";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import LighthouseLogo from "$lib/components/atoms/LighthouseLogo.svelte";
  import { account } from "$lib/stores/account.svelte";
  import { entitlements } from "$lib/stores/entitlements.svelte";
  import { errorBus } from "$lib/stores/errors.svelte";
  import { PRO_PERKS, DONATE_URL, CONTRIBUTE_URL, PRICING_URL } from "$lib/data/proFeatures";

  type Phase = "idle" | "waiting-github" | "waiting-email" | "pro-busy";
  let phase = $state<Phase>("idle");
  let checkoutBusy = $state(false);
  let email = $state("");
  let notice = $state("");
  let dialogEl = $state<HTMLDivElement | null>(null);
  let lastFocused: HTMLElement | null = null;

  const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;
  const emailValid = $derived(EMAIL_RE.test(email.trim()));

  // The sheet shows the Pro acquisition path only when an already-signed-in
  // user is upgrading. Anyone not signed in sees the auth options first.
  const signedIn = $derived(entitlements.isSignedIn);
  const showPro = $derived(account.intent === "pro" && signedIn && !entitlements.isPro);

  const heading = $derived.by(() => {
    if (entitlements.isPro) return "You're on PortBay Pro";
    if (showPro) return "Go unlimited with PortBay Pro";
    if (account.intent === "signup") return "Keep adding projects";
    if (account.intent === "pro") return "Go unlimited with PortBay Pro";
    return "Sign in to PortBay";
  });

  const subhead = $derived.by(() => {
    if (account.reason) return account.reason;
    if (showPro) return "Upgrade to keep adding projects — plus sync, custom ports, and the full mail server.";
    if (account.intent === "signup")
      return "Sign in or create a free account to keep adding projects. It's free.";
    if (account.intent === "pro")
      return "Sign in or create a free account first — then unlock unlimited projects.";
    return "Sign in or create an account to sync your sites and unlock Pro.";
  });

  $effect(() => {
    if (account.isOpen) {
      lastFocused = document.activeElement as HTMLElement | null;
      notice = "";
      phase = "idle";
      queueMicrotask(() => dialogEl?.querySelector<HTMLElement>("[data-autofocus]")?.focus());
    } else if (lastFocused) {
      lastFocused.focus();
      lastFocused = null;
    }
  });

  function close() {
    if (phase === "waiting-github" || phase === "waiting-email") entitlements.cancelLogin();
    phase = "idle";
    account.close();
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      close();
    }
  }

  function toastSuccess(msg: string) {
    errorBus.push({
      code: "ACCOUNT",
      whatHappened: msg,
      whyItMatters: "",
      whoCausedIt: "user",
      severity: "success",
      actions: [],
    });
  }

  function handleResult(r: "ready" | "expired" | "timeout" | "error") {
    if (r === "ready") {
      const who = entitlements.account?.login;
      toastSuccess(entitlements.isPro ? "Signed in — Pro unlocked." : who ? `Signed in as ${who}.` : "You're signed in.");
      close();
      return;
    }
    phase = "idle";
    notice =
      r === "expired"
        ? "That sign-in link expired or was cancelled. Try again."
        : r === "timeout"
          ? "Timed out waiting for sign-in. Give it another go."
          : "Couldn't start sign-in. Check your connection and retry.";
  }

  async function startGithub() {
    notice = "";
    phase = "waiting-github";
    handleResult(await entitlements.login("github"));
  }

  async function startEmail() {
    if (!emailValid) {
      notice = "Enter a valid email address.";
      return;
    }
    notice = "";
    phase = "waiting-email";
    handleResult(await entitlements.login("email", email.trim()));
  }

  function cancelWaiting() {
    entitlements.cancelLogin();
    phase = "idle";
  }

  async function getPro() {
    checkoutBusy = true;
    try {
      await entitlements.startCheckout();
      // The browser opens the Paddle checkout; the user completes payment there,
      // then comes back — "Refresh my license" (or app restart) picks up Pro.
    } finally {
      checkoutBusy = false;
    }
  }

  async function refreshLicense() {
    phase = "pro-busy";
    notice = "";
    await entitlements.resync();
    phase = "idle";
    if (entitlements.isPro) {
      toastSuccess("Pro unlocked — thank you for supporting PortBay.");
      close();
    } else {
      notice =
        "No Pro license found. Contributions unlock once your PR is merged — it may take a moment to reflect.";
    }
  }

</script>

<svelte:window onkeydown={account.isOpen ? onKeydown : undefined} />

{#if account.isOpen}
  <div class="fixed inset-0 z-[70] bg-black/45 backdrop-blur-sm" onclick={close} role="presentation"></div>
  <div
    bind:this={dialogEl}
    role="dialog"
    aria-modal="true"
    aria-labelledby="signin-title"
    class="fixed left-1/2 top-1/2 z-[71] w-[min(460px,calc(100vw-2rem))]
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
          <h2 id="signin-title" class="text-[16px] font-semibold text-fg tracking-tight">{heading}</h2>
          <p class="text-[12.5px] leading-snug text-fg-muted mt-0.5">{subhead}</p>
        </div>
      </div>
    </div>

    <div class="px-6 py-5">
      {#if entitlements.isPro}
        <!-- already Pro -->
        <div class="flex items-center gap-2.5 rounded-lg bg-accent/10 text-accent px-3.5 py-3">
          <Icon name="sparkles" size={16} />
          <p class="text-[13px] font-medium">
            Signed in as {entitlements.account?.login}. Every Pro feature is unlocked — thank you.
          </p>
        </div>
      {:else if showPro}
        <!-- upgrade to Pro (signed-in free user) -->
        <ul class="grid grid-cols-1 gap-2 mb-5">
          {#each PRO_PERKS as perk}
            <li class="flex items-center gap-2.5 text-[13px] text-fg">
              <span class="grid place-items-center w-6 h-6 rounded-md bg-accent/10 text-accent shrink-0">
                <Icon name={perk.icon} size={13} />
              </span>
              {perk.label}
            </li>
          {/each}
        </ul>
        <p class="text-[12.5px] leading-relaxed text-fg-muted mb-4">
          <span class="text-fg font-medium">$59/yr</span> — activates on up to 2 devices, renews annually, cancel
          anytime. OSS contributors earn Pro by merging a pull request.
        </p>
        <div class="flex flex-col gap-2.5">
          <div class="flex flex-col gap-1">
            <button
              type="button"
              data-autofocus
              onclick={getPro}
              disabled={checkoutBusy}
              class="inline-flex items-center justify-center gap-2 h-11 rounded-xl bg-accent text-on-accent text-[14px] font-semibold hover:brightness-110 active:brightness-95 transition shadow-sm disabled:opacity-60 disabled:cursor-not-allowed"
            >
              {#if checkoutBusy}
                <span class="spinner"></span> Opening checkout…
              {:else}
                <Icon name="sparkles" size={15} /> Get Pro — $59/yr
              {/if}
            </button>
            <div class="flex items-center justify-center">
              <button
                type="button"
                onclick={() => void openUrl(PRICING_URL)}
                class="text-[12px] text-fg-subtle hover:text-fg transition"
              >
                Learn more about Pro <Icon name="external-link" size={11} class="inline opacity-60" />
              </button>
            </div>
          </div>
          <button
            type="button"
            onclick={() => void openUrl(CONTRIBUTE_URL)}
            class="inline-flex items-center justify-center gap-2 h-11 rounded-xl border border-border bg-surface text-fg text-[14px] font-medium hover:bg-surface-2 transition"
          >
            <Icon name="terminal" size={15} /> Contribute a pull request
            <Icon name="external-link" size={13} class="opacity-60" />
          </button>
          <div class="flex items-center justify-between mt-1">
            <button
              type="button"
              onclick={() => void openUrl(DONATE_URL)}
              class="text-[12px] text-fg-subtle hover:text-fg transition"
            >
              Tip the project <Icon name="external-link" size={11} class="inline opacity-60" />
            </button>
            <button
              type="button"
              onclick={refreshLicense}
              disabled={phase === "pro-busy"}
              class="inline-flex items-center gap-1.5 text-[12px] font-medium text-fg-muted hover:text-fg transition disabled:opacity-60"
            >
              {#if phase === "pro-busy"}
                <span class="spinner"></span> Checking…
              {:else}
                <Icon name="refresh-cw" size={12} /> Refresh my license
              {/if}
            </button>
          </div>
        </div>
      {:else if phase === "waiting-github"}
        <div class="text-center py-4">
          <span class="spinner-lg mx-auto"></span>
          <p class="text-[13.5px] text-fg font-medium mt-4">Waiting for you to authorize in your browser…</p>
          <p class="text-[12px] text-fg-muted mt-1">Approve PortBay on GitHub, then come back here.</p>
          <button
            type="button"
            onclick={cancelWaiting}
            class="mt-4 h-8 px-3.5 rounded-md text-[12px] font-medium text-fg-muted hover:text-fg hover:bg-surface-2 transition"
          >
            Cancel
          </button>
        </div>
      {:else if phase === "waiting-email"}
        <div class="text-center py-4">
          <span class="spinner-lg mx-auto"></span>
          <p class="text-[13.5px] text-fg font-medium mt-4">Check your inbox</p>
          <p class="text-[12px] text-fg-muted mt-1">
            We sent a sign-in link to <span class="text-fg">{email.trim()}</span>. Keep this window open — you'll be
            signed in automatically.
          </p>
          <button
            type="button"
            onclick={cancelWaiting}
            class="mt-4 h-8 px-3.5 rounded-md text-[12px] font-medium text-fg-muted hover:text-fg hover:bg-surface-2 transition"
          >
            Use a different email
          </button>
        </div>
      {:else}
        <!-- idle: auth options -->
        <button
          type="button"
          data-autofocus
          onclick={startGithub}
          class="w-full inline-flex items-center justify-center gap-2.5 h-11 rounded-xl bg-[#1f2328] text-white text-[14px] font-semibold hover:brightness-125 active:brightness-95 transition shadow-sm"
        >
          <svg width="17" height="17" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">
            <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.01 8.01 0 0 0 16 8c0-4.42-3.58-8-8-8Z"/>
          </svg>
          Continue with GitHub
        </button>

        <div class="flex items-center gap-3 my-4">
          <span class="h-px flex-1 bg-border"></span>
          <span class="text-[11px] uppercase tracking-wide text-fg-subtle">or</span>
          <span class="h-px flex-1 bg-border"></span>
        </div>

        <form onsubmit={(e) => { e.preventDefault(); void startEmail(); }}>
          <div class="relative">
            <span class="absolute left-3 top-1/2 -translate-y-1/2 text-fg-subtle">
              <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true">
                <rect x="2" y="4" width="20" height="16" rx="2"/><path d="m22 7-10 5L2 7"/>
              </svg>
            </span>
            <input
              type="email"
              bind:value={email}
              placeholder="you@example.com"
              autocomplete="email"
              class="w-full h-11 pl-9 pr-3 rounded-xl bg-surface border border-border text-[14px] text-fg
                     placeholder:text-fg-subtle focus:outline-none focus:border-accent focus:ring-1 focus:ring-accent/40 transition"
            />
          </div>
          <button
            type="submit"
            disabled={!emailValid}
            class="w-full mt-2.5 inline-flex items-center justify-center gap-2 h-11 rounded-xl bg-accent text-on-accent text-[14px] font-semibold hover:brightness-110 active:brightness-95 transition shadow-sm disabled:opacity-50 disabled:cursor-not-allowed"
          >
            Email me a sign-in link <Icon name="arrow-right" size={14} />
          </button>
        </form>
      {/if}

      {#if notice}
        <p class="mt-3 text-[12px] leading-relaxed text-status-crashed flex items-start gap-1.5">
          <Icon name="circle-alert" size={13} class="mt-px shrink-0" />
          <span>{notice}</span>
        </p>
      {/if}
    </div>

    {#if !entitlements.isPro && !showPro && phase === "idle"}
      <footer class="px-6 py-3.5 border-t border-border bg-surface/40">
        <p class="text-[11.5px] leading-relaxed text-fg-subtle text-center">
          Free forever — 6 projects, automatic <span class="text-fg-muted">.test</span> domains, trusted HTTPS, and a
          built-in mail catcher. No card, no spam.
        </p>
      </footer>
    {/if}
  </div>
{/if}

<style>
  .spinner,
  .spinner-lg {
    display: inline-block;
    border-radius: 9999px;
    border: 2px solid color-mix(in srgb, var(--color-accent) 30%, transparent);
    border-top-color: var(--color-accent);
    animation: spin 0.7s linear infinite;
  }
  .spinner {
    width: 13px;
    height: 13px;
  }
  .spinner-lg {
    width: 30px;
    height: 30px;
    border-width: 3px;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
