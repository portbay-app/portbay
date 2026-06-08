<script lang="ts">
  import Icon from "$lib/components/atoms/Icon.svelte";

  let {
    message,
    onDismiss,
  }: {
    message: string;
    onDismiss?: () => void;
  } = $props();
</script>

<div class="border-t border-status-crashed/30 bg-status-crashed/5 px-6 py-3">
  <div class="mx-auto flex max-w-2xl items-start gap-2.5">
    <Icon name="circle-alert" size={15} class="mt-0.5 shrink-0 text-status-crashed" />
    <p class="flex-1 whitespace-pre-wrap text-[12px] leading-relaxed text-fg">{message}</p>
    {#if onDismiss}
      <button
        type="button"
        onclick={onDismiss}
        class="shrink-0 rounded p-1 text-fg-muted hover:bg-surface-2 hover:text-fg"
        aria-label="Dismiss error"
        title="Dismiss"
      >
        <Icon name="x" size={13} />
      </button>
    {/if}
  </div>
</div>
