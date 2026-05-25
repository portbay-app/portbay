<!--
  UserMenu — anchored dropdown triggered from the topbar avatar.

  PortBay is single-user today; the avatar is deterministic (gradient
  + "P" initial) rather than tied to an account. The menu still earns
  its place as the natural home for app-level commands:
    - About PortBay (version + GitHub)
    - Settings (navigates to /settings)
    - Quit (Tauri exit; ⌘Q already works, but discoverability matters)
-->
<script lang="ts">
  import { goto } from "$app/navigation";
  import { openUrl } from "@tauri-apps/plugin-opener";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import { safeInvoke } from "$lib/ipc";
  import { errorBus } from "$lib/stores/errors.svelte";
  import { entitlements } from "$lib/stores/entitlements.svelte";
  import { account } from "$lib/stores/account.svelte";
  import { licenseDialog } from "$lib/stores/licenseDialog.svelte";

  const tier = $derived(entitlements.tier);

  function openLicense() {
    onclose();
    licenseDialog.open();
  }

  function openAccount(intent: "signin" | "pro") {
    onclose();
    account.open({ intent });
  }

  interface Props {
    open: boolean;
    onclose: () => void;
  }
  let { open, onclose }: Props = $props();

  let menuEl: HTMLDivElement | undefined = $state();

  function onWindowKey(e: KeyboardEvent) {
    if (open && e.key === "Escape") onclose();
  }

  function onWindowClick(e: MouseEvent) {
    if (!open || !menuEl) return;
    const target = e.target as Node | null;
    if (target && !menuEl.contains(target)) onclose();
  }

  function gotoSettings() {
    onclose();
    void goto("/settings");
  }

  function aboutPortbay() {
    onclose();
    // Until we have an About modal, route the "About" action through
    // the toast system as an informational notice. Replaces a TODO
    // and gives the menu item a real effect.
    errorBus.push({
      code: "ABOUT",
      whatHappened: "PortBay 0.1.0 — local-by-default dev environment.",
      whyItMatters: "Open the docs or repo for release notes and roadmap.",
      whoCausedIt: "system",
      severity: "info",
      actions: [
        { label: "GitHub", url: "https://github.com/portbay-app/portbay" },
        { label: "Docs", url: "https://docs.portbay.app" },
      ],
    });
  }

  async function quit() {
    onclose();
    try {
      await safeInvoke("quit_app");
    } catch {
      // safeInvoke pushes a toast on failure; swallow here so the menu
      // closes cleanly even if the IPC layer surfaced an error.
    }
  }

  function openGithub() {
    onclose();
    void openUrl("https://github.com/portbay-app/portbay");
  }
</script>

<svelte:window onkeydown={onWindowKey} onclick={onWindowClick} />

{#if open}
  <div
    bind:this={menuEl}
    class="absolute right-0 top-12 z-50 w-56
           rounded-xl border border-border bg-surface shadow-2xl
           py-1 overflow-hidden"
    role="menu"
    aria-label="User menu"
  >
    <div class="px-3 py-2.5 border-b border-border">
      {#if entitlements.isSignedIn}
        <p class="text-[12px] font-medium text-fg truncate">{entitlements.account?.login}</p>
        <p class="text-[11px] {tier === 'pro' ? 'text-accent' : 'text-fg-subtle'}">
          {tier === "pro" ? "PortBay Pro" : "Free account"}
        </p>
      {:else}
        <p class="text-[12px] font-medium text-fg">PortBay</p>
        <p class="text-[11px] text-fg-subtle">Not signed in</p>
      {/if}
    </div>

    {#if tier === "anonymous"}
      <button
        type="button"
        onclick={() => openAccount("signin")}
        class="w-full flex items-center gap-2.5 px-3 py-2 text-[13px]
               text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
        role="menuitem"
      >
        <Icon name="users" size={13} /> Sign in or sign up
      </button>
    {:else if tier === "free"}
      <button
        type="button"
        onclick={() => openAccount("pro")}
        class="w-full flex items-center gap-2.5 px-3 py-2 text-[13px]
               text-accent hover:bg-surface-2 transition-colors"
        role="menuitem"
      >
        <Icon name="sparkles" size={13} /> Upgrade to Pro
      </button>
    {/if}

    <button
      type="button"
      onclick={openLicense}
      class="w-full flex items-center gap-2.5 px-3 py-2 text-[13px]
             text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
      role="menuitem"
    >
      <Icon name="sparkles" size={13} /> PortBay Pro
    </button>
    <button
      type="button"
      onclick={aboutPortbay}
      class="w-full flex items-center gap-2.5 px-3 py-2 text-[13px]
             text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
      role="menuitem"
    >
      <Icon name="info" size={13} /> About PortBay
    </button>
    <button
      type="button"
      onclick={openGithub}
      class="w-full flex items-center gap-2.5 px-3 py-2 text-[13px]
             text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
      role="menuitem"
    >
      <!-- Inline GitHub mark (lucide doesn't ship brand logos). -->
      <svg
        width="13"
        height="13"
        viewBox="0 0 16 16"
        fill="currentColor"
        aria-hidden="true"
      >
        <path
          d="M8 0C3.58 0 0 3.58 0 8a8 8 0 0 0 5.47 7.59c.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.01 8.01 0 0 0 16 8c0-4.42-3.58-8-8-8z"
        />
      </svg>
      View on GitHub
    </button>
    <button
      type="button"
      onclick={gotoSettings}
      class="w-full flex items-center gap-2.5 px-3 py-2 text-[13px]
             text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
      role="menuitem"
    >
      <Icon name="settings" size={13} /> Settings
    </button>

    <div class="my-1 border-t border-border/80"></div>

    <button
      type="button"
      onclick={quit}
      class="w-full flex items-center gap-2.5 px-3 py-2 text-[13px]
             text-fg-muted hover:text-status-crashed hover:bg-surface-2
             transition-colors"
      role="menuitem"
    >
      <Icon name="log-out" size={13} /> Quit PortBay
    </button>
  </div>
{/if}
