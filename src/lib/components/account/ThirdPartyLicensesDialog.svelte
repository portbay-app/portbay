<!--
  ThirdPartyLicensesDialog — renders the NOTICE file embedded in the app
  binary (third-party attribution: bundled sidecars, statically linked AI
  engine libraries, lifted UI components, the dnsmasq GPL source offer).
  MIT/Apache notices must accompany the distributed binary, so this view —
  not just the repository file — is the compliance surface.
-->
<script lang="ts">
  import { invokeQuiet } from "$lib/ipc";
  import Icon from "$lib/components/atoms/Icon.svelte";

  interface Props {
    open?: boolean;
    onClose?: () => void;
  }
  let { open = false, onClose }: Props = $props();

  let text = $state<string | null>(null);

  $effect(() => {
    if (open && text === null) {
      invokeQuiet<string>("legal_notices")
        .then((t) => (text = t))
        .catch(() => (text = "Couldn't load the bundled NOTICE file."));
    }
  });

  function onBackdrop(e: MouseEvent) {
    if (e.target === e.currentTarget) onClose?.();
  }
  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") onClose?.();
  }
</script>

<svelte:window onkeydown={onKeydown} />

{#if open}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div
    class="fixed inset-0 z-[60] flex items-center justify-center bg-black/55 backdrop-blur-sm"
    role="dialog"
    aria-modal="true"
    aria-label="Third-party licenses"
    tabindex="-1"
    onclick={onBackdrop}
  >
    <div class="flex max-h-[80vh] w-[640px] max-w-[92vw] flex-col rounded-2xl border border-border bg-surface shadow-2xl">
      <header class="flex items-center justify-between gap-3 border-b border-border px-5 py-3.5">
        <div class="flex items-center gap-2">
          <Icon name="file-text" size={15} class="text-fg-muted" />
          <h2 class="text-[14px] font-semibold text-fg">Third-party licenses</h2>
        </div>
        <button
          type="button"
          class="grid h-7 w-7 place-items-center rounded-md text-fg-muted hover:bg-surface-2 hover:text-fg"
          aria-label="Close"
          onclick={() => onClose?.()}
        >
          <Icon name="x" size={14} />
        </button>
      </header>
      <div class="min-h-0 flex-1 overflow-y-auto px-5 py-4">
        {#if text === null}
          <p class="text-[12px] text-fg-subtle">Loading…</p>
        {:else}
          <pre class="whitespace-pre-wrap font-mono text-[11px] leading-relaxed text-fg-muted">{text}</pre>
        {/if}
      </div>
    </div>
  </div>
{/if}
