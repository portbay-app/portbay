<!--
  Toggle — sliding pill switch. Used wherever a setting is binary
  on/off and the row reads cleanly as "Label … toggle".

  The thumb travels with a short ease so the state change reads as a
  physical motion rather than a flicker. Disabled toggles dim to half
  opacity and don't react to clicks.
-->
<script lang="ts">
  interface Props {
    checked: boolean;
    disabled?: boolean;
    /** Accessible label — required for screen readers. */
    label?: string;
    /** Fires with the next state. */
    onchange?: (next: boolean) => void;
  }
  let { checked, disabled = false, label, onchange }: Props = $props();

  function toggle() {
    if (disabled) return;
    onchange?.(!checked);
  }

  function onKey(e: KeyboardEvent) {
    if (disabled) return;
    if (e.key === " " || e.key === "Enter") {
      e.preventDefault();
      onchange?.(!checked);
    }
  }
</script>

<button
  type="button"
  role="switch"
  aria-checked={checked}
  aria-label={label}
  aria-disabled={disabled}
  tabindex={disabled ? -1 : 0}
  onclick={toggle}
  onkeydown={onKey}
  class="inline-flex items-center w-9 h-5 rounded-full transition-colors
         disabled:opacity-50 disabled:cursor-not-allowed
         focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40
         {checked ? 'bg-accent' : 'bg-surface-2 border border-border'}"
  {disabled}
>
  <span
    class="inline-block w-4 h-4 rounded-full bg-white shadow
           border border-border-strong/40
           transition-transform duration-200 ease-out
           {checked ? 'translate-x-[18px]' : 'translate-x-0.5'}"
    aria-hidden="true"
  ></span>
</button>
