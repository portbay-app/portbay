<!--
  DatabaseMark — small brand glyph for a database engine.

  Inline SVG so the mark stays in step with the theme and never depends
  on a network asset. Each engine carries its iconic colour so the row
  reads at a glance against either light or dark surfaces.
-->
<script lang="ts">
  interface Props {
    id: string;
    size?: number;
    class?: string;
  }
  let { id, size = 36, class: cls = "" }: Props = $props();
</script>

{#if id === "mysql"}
  <!-- MySQL — stylised dolphin silhouette on a navy slab -->
  <svg
    width={size}
    height={size}
    viewBox="0 0 40 40"
    fill="none"
    class={cls}
    aria-hidden="true"
  >
    <rect x="2" y="2" width="36" height="36" rx="9" fill="#1B4E72" />
    <path
      d="M8 22c3-1 6-1 8 1 1 1 2 3 5 3 4 0 6-3 9-3 1 0 2 0 3 1-1-3-4-4-6-4-3 0-5 2-6 2-2 0-3-2-5-3-3-1-6-1-8 3z"
      fill="#00758F"
    />
    <path
      d="M27 21c1 0 2 1 2 2"
      stroke="#7BD8F0"
      stroke-width="1.2"
      stroke-linecap="round"
    />
  </svg>
{:else if id === "postgres"}
  <!-- PostgreSQL — elephant silhouette on a steel-blue slab -->
  <svg
    width={size}
    height={size}
    viewBox="0 0 40 40"
    fill="none"
    class={cls}
    aria-hidden="true"
  >
    <rect x="2" y="2" width="36" height="36" rx="9" fill="#1F3E5C" />
    <path
      d="M11 25c-1-3 0-6 2-8 1-2 3-3 6-3 4 0 7 3 7 7 0 3-2 5-1 7 1 1 2 1 2 2-1 1-3 1-4 0-2-1-2-3-3-4-1-2-3-2-4-1-2 1-4 1-5 0z"
      fill="#336791"
    />
    <circle cx="22" cy="17" r="1" fill="#fff" />
  </svg>
{:else if id === "mongo"}
  <!-- MongoDB — leaf-shape on dark-green slab -->
  <svg
    width={size}
    height={size}
    viewBox="0 0 40 40"
    fill="none"
    class={cls}
    aria-hidden="true"
  >
    <rect x="2" y="2" width="36" height="36" rx="9" fill="#0E3327" />
    <path
      d="M20 7c2 4 6 8 6 14 0 5-3 9-6 11-3-2-6-6-6-11 0-6 4-10 6-14z"
      fill="#00ED64"
    />
    <path
      d="M20 12c1 3 3 7 3 11 0 4-2 7-3 8"
      stroke="#0E3327"
      stroke-width="1.2"
      stroke-linecap="round"
    />
  </svg>
{:else if id === "redis"}
  <!-- Redis — stacked cubes on red slab -->
  <svg
    width={size}
    height={size}
    viewBox="0 0 40 40"
    fill="none"
    class={cls}
    aria-hidden="true"
  >
    <rect x="2" y="2" width="36" height="36" rx="9" fill="#3C0F0C" />
    <path
      d="M20 9 L29 13 L20 17 L11 13 Z"
      fill="#DC382C"
      stroke="#fff"
      stroke-width="0.8"
    />
    <path
      d="M11 17 L20 21 L29 17"
      stroke="#fff"
      stroke-width="1"
      fill="none"
    />
    <path
      d="M11 21 L20 25 L29 21"
      stroke="#fff"
      stroke-width="1"
      fill="none"
    />
    <path
      d="M11 25 L20 29 L29 25"
      stroke="#fff"
      stroke-width="1"
      fill="none"
    />
  </svg>
{:else if id === "mariadb"}
  <!-- MariaDB — seal silhouette on dark-blue slab -->
  <svg
    width={size}
    height={size}
    viewBox="0 0 40 40"
    fill="none"
    class={cls}
    aria-hidden="true"
  >
    <rect x="2" y="2" width="36" height="36" rx="9" fill="#1E2A35" />
    <path
      d="M9 25c2-2 4-3 7-3 4 0 6 2 9 2 3 0 5-1 6-2-1 4-4 7-9 7-5 0-9-1-13-4z"
      fill="#C0765A"
    />
    <circle cx="27" cy="20" r="0.9" fill="#fff" />
  </svg>
{:else}
  <svg
    width={size}
    height={size}
    viewBox="0 0 40 40"
    fill="none"
    class={cls}
    aria-hidden="true"
  >
    <rect x="2" y="2" width="36" height="36" rx="9" fill="#374151" />
    <ellipse cx="20" cy="14" rx="9" ry="3" fill="#9CA3AF" />
    <path d="M11 14 V26 C11 28 15 29 20 29 C25 29 29 28 29 26 V14"
      stroke="#9CA3AF"
      stroke-width="1.4"
      fill="none"
    />
    <ellipse cx="20" cy="20" rx="9" ry="3" stroke="#9CA3AF" stroke-width="1" fill="none" />
  </svg>
{/if}
