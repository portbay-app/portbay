<script lang="ts">
  import Icon from "$lib/components/atoms/Icon.svelte";

  let {
    providerLabel,
    onSignIn,
  }: {
    providerLabel: string;
    onSignIn: () => void;
  } = $props();
</script>

<div class="border-t border-border/60 bg-surface-2/30 px-6 py-3">
  <div class="mx-auto flex max-w-2xl items-center gap-3">
    <Icon name="shield-check" size={15} class="shrink-0 text-fg-muted" />
    <p class="flex-1 text-[12px] text-fg-muted leading-relaxed">
      {providerLabel} isn't signed in on this host. Sign in with your subscription — the official
      CLI handles it; no keys touch PortBay.
    </p>
    <button
      type="button"
      onclick={onSignIn}
      class="inline-flex shrink-0 items-center gap-1.5 h-8 px-3 rounded-md text-[12px] font-medium bg-accent text-on-accent hover:brightness-110"
    >
      <Icon name="shield-check" size={13} /> Sign in to {providerLabel}
    </button>
  </div>
</div>
