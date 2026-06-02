<!--
  ChannelChips — a compact group of on/off pills for picking notification
  delivery surfaces per category. Replaces a wall of bare switches: each chip
  is a labelled toggle, so the channel is read by name, not column position.
  Carries an on-dot in addition to the accent tint so state isn't colour-only.
-->
<script lang="ts">
  import type { NotificationChannel } from "$lib/notifications/prefs";

  interface Chip {
    id: NotificationChannel;
    label: string;
  }

  interface Props {
    channels: Chip[];
    value: Record<NotificationChannel, boolean>;
    label: string;
    onchange: (id: NotificationChannel, on: boolean) => void;
  }

  let { channels, value, label, onchange }: Props = $props();
</script>

<div role="group" aria-label={label} class="inline-flex flex-wrap items-center gap-1">
  {#each channels as chip (chip.id)}
    <button
      type="button"
      role="switch"
      aria-checked={value[chip.id]}
      aria-label={`${label}: ${chip.label}`}
      onclick={() => onchange(chip.id, !value[chip.id])}
      class="inline-flex items-center gap-1.5 h-7 px-2.5 rounded-md border text-[12px] transition-colors
             focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40
             {value[chip.id]
        ? 'border-accent/50 bg-accent/12 text-fg'
        : 'border-border text-fg-muted hover:text-fg hover:bg-surface-2'}"
    >
      <span
        aria-hidden="true"
        class="inline-block w-1.5 h-1.5 rounded-full {value[chip.id]
          ? 'bg-accent'
          : 'bg-border-strong/50'}"
      ></span>
      {chip.label}
    </button>
  {/each}
</div>
