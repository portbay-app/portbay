<!--
  ErrorEnvelope — renders one `CommandError` in the four-line shape from
  docs/UX_DESIGN.md §5.4.

  Three tones, picked by the caller:
    - inline : embedded in a row or section; no dismiss, no width
    - toast  : floating card with dismiss (×) and self-dismiss policy
    - modal  : full overlay (use sparingly; blocks the UI)

  Density awareness: in compact density, the "Why it matters" line and the
  "Show details" expander collapse to a single line for power users who
  prefer to fix-fast.
-->
<script lang="ts" module>
  export type EnvelopeTone = "inline" | "toast" | "modal";
</script>

<script lang="ts">
  import type { CommandError, ErrorAction } from "$lib/types/error";
  import Icon from "$lib/components/atoms/Icon.svelte";
  import { density } from "$lib/stores/density";

  interface Props {
    envelope: CommandError;
    tone?: EnvelopeTone;
    /** Called when the user clicks the × dismiss button. Only meaningful
        for toast / modal tones. */
    onDismiss?: () => void;
    /** Called when the user clicks an action button. The host can
        dispatch the action's command via `safeInvoke` (or open a URL via
        the opener plugin). Returning a truthy value dismisses the toast. */
    onAction?: (action: ErrorAction) => void | boolean | Promise<void | boolean>;
  }
  let { envelope, tone = "inline", onDismiss, onAction }: Props = $props();

  let detailsOpen = $state(false);

  const isCompact = $derived(density.value === "compact");

  // Iconography per "who" — user errors get a softer color than system.
  const accentClass = $derived(
    envelope.whoCausedIt === "user"
      ? "text-status-unhealthy border-status-unhealthy/40"
      : "text-status-crashed border-status-crashed/40",
  );

  // Container shape per tone.
  const containerClass = $derived.by(() => {
    const base = "bg-surface text-sm";
    switch (tone) {
      case "toast":
        return `${base} border ${accentClass} rounded-lg shadow-lg shadow-black/40 p-3 w-80`;
      case "modal":
        return `${base} border ${accentClass} rounded-xl shadow-2xl p-5 max-w-lg w-full`;
      case "inline":
      default:
        return `${base} border ${accentClass} rounded-md p-3`;
    }
  });

  async function handleAction(a: ErrorAction) {
    if (!onAction) return;
    const shouldDismiss = await onAction(a);
    if (shouldDismiss && onDismiss) onDismiss();
  }
</script>

<div
  class={containerClass}
  role="alert"
  aria-live={tone === "modal" ? "assertive" : "polite"}
>
  <div class="flex items-start gap-2.5">
    <span class="shrink-0 mt-0.5 {accentClass} border-0">
      <Icon name="circle-alert" size={16} />
    </span>

    <div class="flex-1 min-w-0 space-y-1">
      <p class="font-medium text-fg leading-snug break-words">
        {envelope.whatHappened}
      </p>

      {#if !isCompact}
        <p class="text-fg-muted text-xs leading-snug">
          {envelope.whyItMatters}
        </p>
      {/if}

      {#if envelope.actions.length > 0}
        <div class="flex flex-wrap gap-1.5 pt-1">
          {#each envelope.actions as action (action.label)}
            <button
              type="button"
              onclick={() => handleAction(action)}
              class="inline-flex items-center gap-1 px-2 py-1 rounded-md text-xs
                     border border-border text-fg
                     hover:border-border-strong hover:bg-surface-2 transition-colors"
            >
              {action.label}
            </button>
          {/each}
        </div>
      {/if}

      {#if envelope.details && !isCompact}
        <button
          type="button"
          onclick={() => (detailsOpen = !detailsOpen)}
          class="text-[11px] text-fg-subtle hover:text-fg-muted inline-flex items-center gap-1 pt-1"
        >
          <Icon name={detailsOpen ? "chevron-down" : "chevron-right"} size={11} />
          {detailsOpen ? "Hide details" : "Show details"}
        </button>
        {#if detailsOpen}
          <pre
            class="mt-1 text-[11px] font-mono text-fg-muted bg-bg/60 border border-border rounded-md p-2 overflow-x-auto whitespace-pre-wrap"
          >{envelope.details}</pre>
        {/if}
      {/if}
    </div>

    {#if onDismiss && tone !== "inline"}
      <button
        type="button"
        onclick={onDismiss}
        title="Dismiss"
        aria-label="Dismiss"
        class="shrink-0 p-1 -m-1 rounded-md text-fg-subtle hover:text-fg hover:bg-surface-2 transition-colors"
      >
        <Icon name="x" size={14} />
      </button>
    {/if}
  </div>
</div>
