<!--
  Segmented — pill-button group of mutually exclusive options.

  Used for Theme (System / Light / Dark) and Density (Compact /
  Comfortable). Keeps the selected segment filled with the accent
  token; unselected segments stay tonally flat so the active option
  is unmistakable at a glance.

  The component is typed over a generic key so callers can pass any
  string-union. Pass the option set in the form
  `{ value: K, label: string }[]`.
-->
<script lang="ts" generics="K extends string">
  interface Option {
    value: K;
    label: string;
  }
  interface Props {
    value: K;
    options: Option[];
    /** Accessible label for the group. */
    label?: string;
    onchange?: (next: K) => void;
  }
  let { value, options, label, onchange }: Props = $props();
</script>

<div
  role="group"
  aria-label={label}
  class="inline-flex items-center bg-surface-2 border border-border rounded-lg p-0.5 gap-0.5"
>
  {#each options as opt (opt.value)}
    {@const selected = opt.value === value}
    <button
      type="button"
      onclick={() => onchange?.(opt.value)}
      aria-pressed={selected}
      class="px-3 py-1 rounded-md text-[12px] font-medium transition-colors
             focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40
             {selected
        ? 'bg-accent text-on-accent shadow-sm'
        : 'text-fg-muted hover:text-fg hover:bg-surface'}"
    >
      {opt.label}
    </button>
  {/each}
</div>
