<!--
  ColorSwatchGroup — the seven-dot accent-colour picker.

  Each swatch is a circular hit-target. The selected swatch gains a
  thin accent-coloured ring + a check mark; unselected swatches read
  as a plain dot. Hover lifts the dot by 1 px so the affordance is
  obvious even without animation.
-->
<script lang="ts">
  import type { AccentColor } from "$lib/stores/preferences.svelte";
  import Icon from "./Icon.svelte";

  interface Props {
    value: AccentColor;
    onchange?: (next: AccentColor) => void;
  }
  let { value, onchange }: Props = $props();

  // Each swatch ships its own hex so we don't compose Tailwind
  // arbitrary-value classes at runtime (JIT can't tree-shake them
  // and they'd silently fail).
  const swatches: { value: AccentColor; hex: string; label: string }[] = [
    { value: "blue", hex: "#4d9cff", label: "Blue" },
    { value: "purple", hex: "#a855f7", label: "Purple" },
    { value: "green", hex: "#22c55e", label: "Green" },
    { value: "orange", hex: "#f97316", label: "Orange" },
    { value: "red", hex: "#ef4444", label: "Red" },
    { value: "yellow", hex: "#eab308", label: "Yellow" },
    { value: "gray", hex: "#9ca3af", label: "Gray" },
  ];
</script>

<div role="radiogroup" aria-label="Accent colour" class="inline-flex items-center gap-2">
  {#each swatches as s (s.value)}
    {@const selected = s.value === value}
    <button
      type="button"
      role="radio"
      aria-checked={selected}
      aria-label={s.label}
      title={s.label}
      onclick={() => onchange?.(s.value)}
      class="relative inline-flex items-center justify-center w-6 h-6 rounded-full
             transition-transform hover:-translate-y-0.5
             focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-offset-1
             focus-visible:ring-offset-surface focus-visible:ring-accent/60
             {selected ? 'ring-2 ring-offset-2 ring-offset-surface' : ''}"
      style:background-color={s.hex}
      style:--tw-ring-color={s.hex}
    >
      {#if selected}
        <Icon name="check" size={12} class="text-white drop-shadow" />
      {/if}
    </button>
  {/each}
</div>
