<!--
  StackIcon — small inline-SVG framework mark per ProjectType.

  The set is intentionally tiny (next, vite, php, static, node, custom).
  Future detector work can extend ProjectType (e.g. "laravel", "go",
  "remix") — drop the SVG path in here and the row picks it up.

  Each mark renders at the caller-provided pixel size on a transparent
  background; colours are baked in so the icon reads even in compact
  monochrome rows.
-->
<script lang="ts">
  import type { ProjectType } from "$lib/types/projects";

  interface Props {
    type: ProjectType;
    size?: number;
    class?: string;
  }
  let { type, size = 16, class: cls = "" }: Props = $props();
</script>

{#if type === "next"}
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    class={cls}
    aria-hidden="true"
  >
    <circle cx="12" cy="12" r="11" fill="#000" />
    <path
      d="M9 6.5 V17.5 M9 6.5 L16 17.5 M14 6.5 V14"
      stroke="#fff"
      stroke-width="1.4"
      stroke-linecap="round"
    />
  </svg>
{:else if type === "vite"}
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    class={cls}
    aria-hidden="true"
  >
    <defs>
      <linearGradient id="vite-grad" x1="0" x2="0" y1="0" y2="1">
        <stop offset="0%" stop-color="#41D1FF" />
        <stop offset="100%" stop-color="#BD34FE" />
      </linearGradient>
      <linearGradient id="vite-grad2" x1="0" x2="0" y1="0" y2="1">
        <stop offset="0%" stop-color="#FFEA83" />
        <stop offset="100%" stop-color="#FFDD35" />
      </linearGradient>
    </defs>
    <path
      d="M2 5 L12 22 L22 5 L17 5 L12 13.5 L7 5 Z"
      fill="url(#vite-grad)"
    />
    <path
      d="M8.5 5.5 L12 11 L15.5 5.5 L13 6 L12 5 L11 6 Z"
      fill="url(#vite-grad2)"
    />
  </svg>
{:else if type === "node"}
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    class={cls}
    aria-hidden="true"
  >
    <path
      d="M12 1.8 L21.5 7.3 V16.7 L12 22.2 L2.5 16.7 V7.3 Z"
      fill="#5FA04E"
    />
    <path
      d="M12 6.5 V17.5 M8 8 V16 M16 8 V16"
      stroke="#fff"
      stroke-width="1.2"
      stroke-linecap="round"
    />
  </svg>
{:else if type === "php"}
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    class={cls}
    aria-hidden="true"
  >
    <ellipse cx="12" cy="12" rx="11" ry="6.5" fill="#787CB5" />
    <text
      x="12"
      y="15.5"
      text-anchor="middle"
      fill="#fff"
      font-family="Inter, system-ui, sans-serif"
      font-size="6.5"
      font-weight="700"
    >php</text>
  </svg>
{:else if type === "static"}
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    class={cls}
    aria-hidden="true"
  >
    <rect x="2.5" y="2.5" width="19" height="19" rx="3" fill="#E34F26" />
    <path
      d="M6 6.5 L18 6.5 L17 17 L12 18.5 L7 17 Z"
      fill="#fff"
      opacity="0.95"
    />
    <path d="M9 9.5 L15 9.5 L14.7 11.5 L9.7 11.5 L9.9 13.5 L14.5 13.5 L14.3 15.5 L12 16 L9.7 15.5 L9.6 14 L9 14" stroke="#E34F26" stroke-width="0.6" fill="none" />
  </svg>
{:else if type === "flutter"}
  <!-- Flutter — official mark (blue on transparent; reads on the dark shell) -->
  <img
    src="/stacks/flutter.png"
    width={size}
    height={size}
    class={cls}
    style="object-fit:contain"
    alt=""
    aria-hidden="true"
  />
{:else if type === "xcode"}
  <!-- Xcode — official app icon (square, self-contained background) -->
  <img
    src="/stacks/xcode.png"
    width={size}
    height={size}
    class={cls}
    style="object-fit:contain"
    alt=""
    aria-hidden="true"
  />
{:else if type === "android"}
  <!-- Android — official robot mark (wide, transparent; contained in a square box) -->
  <img
    src="/stacks/android.png"
    width={size}
    height={size}
    class={cls}
    style="object-fit:contain"
    alt=""
    aria-hidden="true"
  />
{:else if type === "expo"}
  <!-- Expo — official "A" mark (black on transparent); white tile keeps it legible on the dark shell -->
  <img
    src="/stacks/expo.png"
    width={size}
    height={size}
    class={cls}
    style="object-fit:contain;background:#fff;border-radius:22%"
    alt=""
    aria-hidden="true"
  />
{:else if type === "python"}
  <!-- Python — official two-snake mark. The logo has 180° rotational
       symmetry: the lower yellow snake is the upper blue snake rotated
       about the centre, so the same half-path is drawn twice. -->
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    class={cls}
    aria-hidden="true"
  >
    <path
      fill="#3776AB"
      d="M14.25.18l.9.2.73.26.59.3.45.32.34.34.25.34.16.33.1.3.04.26.02.2-.01.13V8.5l-.05.63-.13.55-.21.46-.26.38-.3.31-.33.25-.35.19-.35.14-.33.1-.3.07-.26.04-.21.02H8.77l-.69.05-.59.14-.5.22-.41.27-.33.32-.27.35-.2.36-.15.37-.1.35-.07.32-.04.27-.02.21v3.06H3.17l-.21-.03-.28-.07-.32-.12-.35-.18-.36-.26-.36-.36-.35-.46-.32-.59-.28-.73-.21-.88-.14-1.05-.05-1.23.06-1.22.16-1.04.24-.87.32-.71.36-.57.4-.44.42-.33.42-.24.4-.16.36-.1.32-.05.24-.01h.16l.06.01h8.16v-.83H6.18l-.01-2.75-.02-.37.05-.34.11-.31.17-.28.25-.26.31-.23.38-.2.44-.18.51-.15.58-.12.64-.1.71-.06.77-.04.84-.02 1.27.05zm-6.3 1.98l-.23.33-.08.41.08.41.23.34.33.22.41.09.41-.09.33-.22.23-.34.08-.41-.08-.41-.23-.33-.33-.22-.41-.09-.41.09z"
    />
    <path
      fill="#FFD43B"
      transform="rotate(180 12 12)"
      d="M14.25.18l.9.2.73.26.59.3.45.32.34.34.25.34.16.33.1.3.04.26.02.2-.01.13V8.5l-.05.63-.13.55-.21.46-.26.38-.3.31-.33.25-.35.19-.35.14-.33.1-.3.07-.26.04-.21.02H8.77l-.69.05-.59.14-.5.22-.41.27-.33.32-.27.35-.2.36-.15.37-.1.35-.07.32-.04.27-.02.21v3.06H3.17l-.21-.03-.28-.07-.32-.12-.35-.18-.36-.26-.36-.36-.35-.46-.32-.59-.28-.73-.21-.88-.14-1.05-.05-1.23.06-1.22.16-1.04.24-.87.32-.71.36-.57.4-.44.42-.33.42-.24.4-.16.36-.1.32-.05.24-.01h.16l.06.01h8.16v-.83H6.18l-.01-2.75-.02-.37.05-.34.11-.31.17-.28.25-.26.31-.23.38-.2.44-.18.51-.15.58-.12.64-.1.71-.06.77-.04.84-.02 1.27.05zm-6.3 1.98l-.23.33-.08.41.08.41.23.34.33.22.41.09.41-.09.33-.22.23-.34.08-.41-.08-.41-.23-.33-.33-.22-.41-.09-.41.09z"
    />
  </svg>
{:else}
  <!-- custom / fallback -->
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    class={cls}
    aria-hidden="true"
  >
    <rect x="2.5" y="2.5" width="19" height="19" rx="4" fill="#374151" />
    <path
      d="M7 9 L12 6 L17 9 V15 L12 18 L7 15 Z"
      stroke="#9CA3AF"
      stroke-width="1.3"
      stroke-linejoin="round"
      fill="none"
    />
  </svg>
{/if}
