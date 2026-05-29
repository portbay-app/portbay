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
  <!-- Flutter — overlapping blue chevrons -->
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    class={cls}
    aria-hidden="true"
  >
    <rect x="2.5" y="2.5" width="19" height="19" rx="4" fill="#0B1B2B" />
    <path d="M15.5 5 L8 12.5 L10.6 15.1 L20.7 5 Z" fill="#47C5FB" />
    <path d="M15.5 11.4 L11.2 15.7 L13.8 18.3 L15.5 16.6 L20.7 11.4 Z" fill="#00569E" />
    <path d="M10.6 15.1 L13.8 18.3 L11.2 20.9 L8 17.7 Z" fill="#47C5FB" />
  </svg>
{:else if type === "xcode"}
  <!-- Xcode — blue slab with a white hammer -->
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    class={cls}
    aria-hidden="true"
  >
    <rect x="2.5" y="2.5" width="19" height="19" rx="4" fill="#1C7EF2" />
    <path
      d="M13.7 8.1 L15.9 10.3 L11 15.2 L8.8 13 Z"
      fill="#fff"
    />
    <path
      d="M14.1 6.4 L17.6 9.9 L16.2 11.3 L12.7 7.8 Z"
      fill="#fff"
    />
    <path d="M8.4 13.4 L7 17.8 L10.6 15.6 Z" fill="#fff" />
  </svg>
{:else if type === "android"}
  <!-- Android — green robot head -->
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    class={cls}
    aria-hidden="true"
  >
    <rect x="2.5" y="2.5" width="19" height="19" rx="4" fill="#0E2A1B" />
    <path
      d="M7 14.5 a5 5 0 0 1 10 0 Z"
      fill="#3DDC84"
    />
    <path d="M8.3 8.2 L7.1 6.4 M15.7 8.2 L16.9 6.4" stroke="#3DDC84" stroke-width="1.1" stroke-linecap="round" />
    <circle cx="9.8" cy="11.6" r="0.8" fill="#0E2A1B" />
    <circle cx="14.2" cy="11.6" r="0.8" fill="#0E2A1B" />
  </svg>
{:else if type === "expo"}
  <!-- Expo — dark slab with a white aperture mark -->
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    class={cls}
    aria-hidden="true"
  >
    <rect x="2.5" y="2.5" width="19" height="19" rx="4" fill="#000020" />
    <path
      d="M12 6.5 C13 8.7 16.2 13 17.8 15.4 C16.6 16.4 15.4 16.9 14.4 15.4 C13.6 14.2 12.5 12.3 12 11.4 C11.5 12.3 10.4 14.2 9.6 15.4 C8.6 16.9 7.4 16.4 6.2 15.4 C7.8 13 11 8.7 12 6.5 Z"
      fill="#fff"
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
