<!--
  PlatformIcon — brand mark for a desktop OS (Apple / Windows / Linux), used by
  the web-demo download button + /download page. Single-colour (currentColor)
  so it inherits text colour. Marketing surface only.
-->
<script lang="ts">
  import type { DesktopOS } from "$lib/platform";

  interface Props {
    os: DesktopOS;
    size?: number;
    class?: string;
  }
  let { os, size = 16, class: cls = "" }: Props = $props();
</script>

{#if os === "macos"}
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="currentColor"
    aria-hidden="true"
    class={cls}
  >
    <path
      d="M16.365 1.43c0 1.14-.493 2.27-1.177 3.08-.744.9-1.99 1.57-2.987 1.57-.12 0-.23-.02-.3-.03-.01-.06-.04-.22-.04-.39 0-1.15.572-2.27 1.206-2.98.804-.94 2.142-1.64 3.248-1.68.03.13.05.28.05.43zm4.565 15.71c-.03.09-.473 1.64-1.518 3.24-.945 1.34-1.94 2.71-3.43 2.71-1.517 0-1.9-.88-3.63-.88-1.698 0-2.302.91-3.67.91-1.377 0-2.332-1.26-3.428-2.8-1.287-1.82-2.323-4.63-2.323-7.28 0-4.28 2.797-6.55 5.552-6.55 1.448 0 2.675.95 3.6.95.865 0 2.222-1.01 3.902-1.01.613 0 2.886.06 4.374 2.19-.13.09-2.383 1.37-2.383 4.19 0 3.26 2.854 4.42 2.955 4.45z"
    />
  </svg>
{:else if os === "windows"}
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="currentColor"
    aria-hidden="true"
    class={cls}
  >
    <path
      d="M0 3.449L9.75 2.1v9.451H0m10.949-9.602L24 0v11.4H10.949M0 12.6h9.75v9.451L0 20.699M10.949 12.6H24V24l-12.9-1.801"
    />
  </svg>
{:else}
  <!-- Linux — simple penguin silhouette -->
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="currentColor"
    aria-hidden="true"
    class={cls}
  >
    <path
      d="M12 2a3.2 3.2 0 0 0-3.2 3.2v3.05c-1.1.78-1.86 2.2-2.45 3.66-.42 1.04-1 1.92-1.64 2.66-.5.58-.27 1.45.46 1.7.48.16.95.28 1.2.62.22.3.13.82.55 1.16.62.5 1.64.42 2.48.42h3.28c.84 0 1.86.08 2.48-.42.42-.34.33-.86.55-1.16.25-.34.72-.46 1.2-.62.73-.25.96-1.12.46-1.7-.64-.74-1.22-1.62-1.64-2.66-.59-1.46-1.35-2.88-2.45-3.66V5.2A3.2 3.2 0 0 0 12 2zm-1.5 4.1a.85.85 0 1 1 0 1.7.85.85 0 0 1 0-1.7zm3 0a.85.85 0 1 1 0 1.7.85.85 0 0 1 0-1.7zM12 8.4c.6 0 1.15.28 1.5.68-.45.32-.96.5-1.5.5s-1.05-.18-1.5-.5c.35-.4.9-.68 1.5-.68z"
    />
  </svg>
{/if}
