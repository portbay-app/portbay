<!--
  LanguageMark — small brand glyph for a runtime language.

  Inline SVG, no external assets. Colours are baked so the mark reads
  on any background. New languages slot in by adding a branch here; the
  caller passes the runtime `id` ("php", "node", "python", "go", "ruby").
-->
<script lang="ts">
  interface Props {
    id: string;
    size?: number;
    class?: string;
  }
  let { id, size = 28, class: cls = "" }: Props = $props();
</script>

{#if id === "php"}
  <svg
    width={size}
    height={size}
    viewBox="0 0 32 32"
    fill="none"
    class={cls}
    aria-hidden="true"
  >
    <ellipse cx="16" cy="16" rx="14" ry="9" fill="#787CB5" />
    <text
      x="16"
      y="20"
      text-anchor="middle"
      fill="#fff"
      font-family="Inter, system-ui, sans-serif"
      font-size="9"
      font-weight="800"
      letter-spacing="0.5"
    >php</text>
  </svg>
{:else if id === "node"}
  <svg
    width={size}
    height={size}
    viewBox="0 0 32 32"
    fill="none"
    class={cls}
    aria-hidden="true"
  >
    <path d="M16 2.5 L28 9.3 V22.7 L16 29.5 L4 22.7 V9.3 Z" fill="#5FA04E" />
    <text
      x="16"
      y="20"
      text-anchor="middle"
      fill="#fff"
      font-family="Inter, system-ui, sans-serif"
      font-size="9"
      font-weight="800"
    >JS</text>
  </svg>
{:else if id === "python"}
  <svg
    width={size}
    height={size}
    viewBox="0 0 32 32"
    fill="none"
    class={cls}
    aria-hidden="true"
  >
    <defs>
      <linearGradient id="py-blue" x1="0" x2="1" y1="0" y2="1">
        <stop offset="0%" stop-color="#387EB8" />
        <stop offset="100%" stop-color="#306998" />
      </linearGradient>
      <linearGradient id="py-yellow" x1="0" x2="1" y1="0" y2="1">
        <stop offset="0%" stop-color="#FFE873" />
        <stop offset="100%" stop-color="#FFD43B" />
      </linearGradient>
    </defs>
    <path
      d="M16 3c-3 0-5 1.2-5 3.5V11h6v1H8c-2.5 0-4 1.7-4 4.5S5.5 21 8 21h2v-3.5c0-2.3 2-3.5 5-3.5h4c2.5 0 4-1.6 4-4V6.5C23 4.2 21 3 18 3h-2zm-2 2.5a1 1 0 1 1 0 2 1 1 0 0 1 0-2z"
      fill="url(#py-blue)"
    />
    <path
      d="M16 29c3 0 5-1.2 5-3.5V21h-6v-1h9c2.5 0 4-1.7 4-4.5s-1.5-4.5-4-4.5h-2v3.5c0 2.3-2 3.5-5 3.5h-4c-2.5 0-4 1.6-4 4v4.5C9 27.8 11 29 14 29h2zm2-2.5a1 1 0 1 1 0-2 1 1 0 0 1 0 2z"
      fill="url(#py-yellow)"
    />
  </svg>
{:else if id === "go"}
  <svg
    width={size}
    height={size}
    viewBox="0 0 32 32"
    fill="none"
    class={cls}
    aria-hidden="true"
  >
    <rect x="3" y="6" width="26" height="20" rx="5" fill="#00ADD8" />
    <text
      x="16"
      y="20"
      text-anchor="middle"
      fill="#fff"
      font-family="Inter, system-ui, sans-serif"
      font-size="11"
      font-weight="800"
      letter-spacing="0.5"
    >GO</text>
  </svg>
{:else if id === "ruby"}
  <svg
    width={size}
    height={size}
    viewBox="0 0 32 32"
    fill="none"
    class={cls}
    aria-hidden="true"
  >
    <path d="M6 9 L16 3 L26 9 L23 25 L9 25 Z" fill="#CC342D" />
    <text
      x="16"
      y="20"
      text-anchor="middle"
      fill="#fff"
      font-family="Inter, system-ui, sans-serif"
      font-size="9"
      font-weight="800"
    >Rb</text>
  </svg>
{:else if id === "bun"}
  <svg
    width={size}
    height={size}
    viewBox="0 0 32 32"
    fill="none"
    class={cls}
    aria-hidden="true"
  >
    <rect x="3" y="3" width="26" height="26" rx="8" fill="#FBF0DF" />
    <text
      x="16"
      y="20"
      text-anchor="middle"
      fill="#14151A"
      font-family="Inter, system-ui, sans-serif"
      font-size="9"
      font-weight="800"
      letter-spacing="0.3"
    >bun</text>
  </svg>
{:else}
  <svg
    width={size}
    height={size}
    viewBox="0 0 32 32"
    fill="none"
    class={cls}
    aria-hidden="true"
  >
    <rect x="3" y="3" width="26" height="26" rx="6" fill="#374151" />
    <path
      d="M10 12 L16 8 L22 12 V20 L16 24 L10 20 Z"
      stroke="#9CA3AF"
      stroke-width="1.5"
      fill="none"
    />
  </svg>
{/if}
