<!--
  SshCredentialPrompt — the single VS Code-style credential input, driven by the
  `credentialPrompt` store and mounted once at the layout root.

  It descends from the top of the window (like VS Code's Quick Input) and asks
  for exactly the secret the host was set up with — a key passphrase or a host
  password — then resolves the store's promise so the caller can retry the
  connect. Enter submits, Escape / backdrop / Cancel resolve as cancelled, and
  focus opens on the field and is restored on close.
-->
<script lang="ts">
  import Icon from "$lib/components/atoms/Icon.svelte";
  import QuickSheet from "$lib/components/connections/QuickSheet.svelte";
  import { credentialPrompt } from "$lib/stores/credentialPrompt.svelte";

  let inputEl = $state<HTMLInputElement | null>(null);
  let lastFocused: HTMLElement | null = null;

  let secret = $state("");
  let remember = $state(false);

  const heading = $derived(
    credentialPrompt.kind === "passphrase" ? "Key passphrase" : "Host password",
  );
  const hint = $derived(
    credentialPrompt.kind === "passphrase"
      ? "Enter this key's passphrase — or leave blank and Skip if it has none."
      : "Enter the password for this host to connect.",
  );

  // Reset the field each time the prompt opens and focus it.
  $effect(() => {
    if (credentialPrompt.isOpen) {
      secret = "";
      remember = false;
      lastFocused = document.activeElement as HTMLElement | null;
      queueMicrotask(() => inputEl?.focus());
    } else if (lastFocused) {
      lastFocused.focus();
      lastFocused = null;
    }
  });

  // A passphrase prompt is skippable: an empty field means "this key has no
  // passphrase", which the backend treats as declined and falls through to the
  // password prompt. A password prompt still requires a value.
  const canSkipEmpty = $derived(credentialPrompt.kind === "passphrase");

  function submit() {
    if (secret) {
      credentialPrompt.submit(secret, remember);
    } else if (canSkipEmpty) {
      credentialPrompt.skip();
    }
    // Empty password: do nothing (button is disabled).
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      submit();
    }
    // Esc is handled by QuickSheet → ondismiss → credentialPrompt.cancel()
  }
</script>

<QuickSheet
  open={credentialPrompt.isOpen}
  {heading}
  hostLabel={credentialPrompt.hostLabel}
  icon="key"
  iconClass="text-accent"
  ondismiss={() => credentialPrompt.cancel()}
>
  {#snippet body()}
    <p class="text-[11.5px] text-fg-subtle leading-relaxed">{hint}</p>
    <!-- svelte-ignore a11y_autofocus -->
    <input
      bind:this={inputEl}
      bind:value={secret}
      type="password"
      autocomplete="off"
      autocapitalize="off"
      spellcheck="false"
      placeholder={credentialPrompt.kind === "passphrase" ? "Key passphrase" : "Password"}
      class="mt-2 w-full h-9 rounded-md border border-border bg-surface-2 px-2.5 text-[12.5px] text-fg
             focus:outline-none focus:ring-2 focus:ring-accent/50"
      onkeydown={onKeydown}
    />

    <!-- Remember checkbox -->
    <label class="mt-2.5 flex items-center gap-2 cursor-pointer select-none">
      <input
        type="checkbox"
        bind:checked={remember}
        class="h-3.5 w-3.5 rounded border-border accent-accent"
      />
      <span class="text-[11px] text-fg-subtle">Remember on this device</span>
    </label>

    <p class="mt-1 text-[11px] text-fg-subtle">
      {#if remember}
        Saved to your OS keychain for this host.
      {:else}
        Used only for this connection — never saved.
      {/if}
    </p>

    <div class="mt-3 flex items-center justify-end gap-2">
      <button
        type="button"
        onclick={() => credentialPrompt.cancel()}
        class="inline-flex items-center justify-center h-8 px-3 rounded-md text-[12px]
               font-medium border border-border text-fg-muted hover:text-fg hover:bg-surface-2"
      >
        Cancel
      </button>
      <button
        type="button"
        onclick={submit}
        disabled={!secret && !canSkipEmpty}
        class="inline-flex items-center justify-center gap-1.5 h-8 px-3.5 rounded-md text-[12px]
               font-medium bg-accent text-on-accent hover:brightness-110 disabled:opacity-50"
      >
        <Icon name={!secret && canSkipEmpty ? "chevron-right" : "play"} size={12} />
        {!secret && canSkipEmpty ? "Skip" : "Connect"}
      </button>
    </div>
  {/snippet}
</QuickSheet>
