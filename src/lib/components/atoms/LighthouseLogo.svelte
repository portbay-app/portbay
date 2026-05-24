<!--
  LighthouseLogo — PortBay brand mark. A minimal lighthouse silhouette
  with a glowing lamp; renders crisply at any size via inline SVG.

  Tokens used:
    - currentColor for the structure (inherits text colour)
    - --accent for the lamp glow

  The component is decorative — callers should provide aria-label on a
  wrapping element if they want it announced.
-->
<script lang="ts">
  interface Props {
    size?: number;
    class?: string;
  }
  let { size = 36, class: cls = "" }: Props = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 36 36"
  fill="none"
  xmlns="http://www.w3.org/2000/svg"
  class={cls}
  aria-hidden="true"
>
  <!-- Lamp glow -->
  <circle cx="18" cy="7.5" r="4.5" fill="currentColor" opacity="0.12" />
  <circle cx="18" cy="7.5" r="2.5" fill="var(--color-accent, #3b82f6)" />

  <!-- Lamp housing -->
  <rect x="14.5" y="9.5" width="7" height="3.5" rx="0.6" fill="currentColor" />
  <!-- Gallery / walkway -->
  <rect x="13.5" y="13" width="9" height="1.6" rx="0.4" fill="currentColor" />

  <!-- Tower body — slight taper -->
  <path
    d="M14 14.6 L22 14.6 L22.8 25 L13.2 25 Z"
    fill="currentColor"
    opacity="0.85"
  />
  <!-- Tower stripe -->
  <rect
    x="13.7"
    y="18.7"
    width="8.6"
    height="1.8"
    fill="var(--color-accent, #3b82f6)"
    opacity="0.7"
  />

  <!-- Base / pier -->
  <rect x="11.5" y="25" width="13" height="2.2" rx="0.5" fill="currentColor" />
  <!-- Water -->
  <path
    d="M5 30 Q9 28.5 13 30 T21 30 T29 30 T33 30 V33 H5 Z"
    fill="currentColor"
    opacity="0.35"
  />
</svg>
