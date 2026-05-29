<!--
  DatabaseMark — brand glyph for a database engine.

  Inline SVG so the mark stays theme-independent and needs no network asset.
  Each engine sits on its *canonical brand background colour* with a simplified,
  recognisable glyph in the brand's foreground colour — stylised marks, not
  official logo assets. `id` is the engine id (`mysql`, `postgres`, …); unknown
  ids fall back to a neutral database cylinder.
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
  <!-- MySQL — Sakila dolphin on the brand teal-navy -->
  <svg width={size} height={size} viewBox="0 0 40 40" fill="none" class={cls} aria-hidden="true">
    <rect x="2" y="2" width="36" height="36" rx="9" fill="#00618A" />
    <path
      d="M9 24c4-1 7 1 9 3 0-2 0-4 1-5 2-3 5-4 8-4-1 1-2 2-2 3 0 2 1 3 1 5 1-1 2-1 3-1-1 2-3 3-5 3-2 0-3-1-4-2-1 2-3 3-6 3-3 0-5-3-6-8z"
      fill="#F8FAFC"
    />
    <circle cx="25.5" cy="19.6" r="0.9" fill="#00618A" />
    <path d="M30 22c1 0 2 .6 2 1.4" stroke="#F29111" stroke-width="1.3" stroke-linecap="round" />
  </svg>
{:else if id === "mariadb"}
  <!-- MariaDB — sea-lion silhouette on the brand deep teal -->
  <svg width={size} height={size} viewBox="0 0 40 40" fill="none" class={cls} aria-hidden="true">
    <rect x="2" y="2" width="36" height="36" rx="9" fill="#003545" />
    <path
      d="M8 26c2-1 3-3 4-5 1-3 4-6 8-6 2 0 3-1 4-3 1 2 0 4-1 5 2 0 4 1 5 3-2 0-3 0-4 1-2 2-3 5-6 6-4 1-8 0-10-1z"
      fill="#C0765A"
    />
    <circle cx="22.5" cy="14.6" r="0.85" fill="#fff" />
  </svg>
{:else if id === "postgres"}
  <!-- PostgreSQL — Slonik the elephant on the brand blue -->
  <svg width={size} height={size} viewBox="0 0 40 40" fill="none" class={cls} aria-hidden="true">
    <rect x="2" y="2" width="36" height="36" rx="9" fill="#336791" />
    <path
      d="M13 27c-2-3-2-7-1-10 1-4 5-6 9-5 4 0 7 3 6 7-1 3-3 4-3 6 0 1 1 2 2 2-1 1-2 1-3 1l-1-3c-1 0-1 2-2 3-1-1-1-3-2-4-1 2-2 3-2 4-1-1 0-3 0-4-1 1-2 1-3 2 1 1 1 1 0 1-1-1-2-1-3 0z"
      fill="#EAF2F8"
    />
    <circle cx="24" cy="18.5" r="1" fill="#336791" />
  </svg>
{:else if id === "sqlite"}
  <!-- SQLite — feather on the brand deep blue -->
  <svg width={size} height={size} viewBox="0 0 40 40" fill="none" class={cls} aria-hidden="true">
    <rect x="2" y="2" width="36" height="36" rx="9" fill="#003B57" />
    <path
      d="M28 8c-8 2-13 9-14 16-.6 3 0 5 .8 7 1-1 2-2 2.7-3.4 1.6.2 3.2.1 4.5-.6-1.4 0-2.6-.5-3.6-1.2C20 19 23.8 13 28.5 9.6 28.4 9 28.2 8.4 28 8z"
      fill="#0F80CC"
    />
    <path d="M15.5 30c1.4-6 5-11 10.5-14" stroke="#7DD3F7" stroke-width="1.2" stroke-linecap="round" />
  </svg>
{:else if id === "redis"}
  <!-- Redis — stacked-cube mark on the brand red -->
  <svg width={size} height={size} viewBox="0 0 40 40" fill="none" class={cls} aria-hidden="true">
    <rect x="2" y="2" width="36" height="36" rx="9" fill="#A41E11" />
    <path d="M20 9 L30 13 L20 17 L10 13 Z" fill="#fff" />
    <path d="M10 13 V17 L20 21 L30 17 V13" fill="#C9332A" />
    <path d="M10 17 V21 L20 25 L30 21 V17" fill="#fff" opacity="0.85" />
    <path d="M10 21 V25 L20 29 L30 25 V21" fill="#C9332A" />
  </svg>
{:else if id === "mongo"}
  <!-- MongoDB — leaf on the brand slate -->
  <svg width={size} height={size} viewBox="0 0 40 40" fill="none" class={cls} aria-hidden="true">
    <rect x="2" y="2" width="36" height="36" rx="9" fill="#001E2B" />
    <path
      d="M20 7c2.5 4 6 8 6 14 0 5-3 8.5-5 10.5l-1 1.5-1-1.5C17 29.5 14 26 14 21c0-6 3.5-10 6-14z"
      fill="#00ED64"
    />
    <path d="M20 11c.6 4 1 9 1 13 0 3-.5 5.5-1 7.5" stroke="#001E2B" stroke-width="1.1" stroke-linecap="round" />
  </svg>
{:else if id === "memcached"}
  <!-- Memcached — no mascot; slab "m" monogram on the brand slate-blue -->
  <svg width={size} height={size} viewBox="0 0 40 40" fill="none" class={cls} aria-hidden="true">
    <rect x="2" y="2" width="36" height="36" rx="9" fill="#2D5E73" />
    <path
      d="M11 27 V15 h3 l3 7 3-7 h3 v12 h-3 v-7 l-2.5 5.5h-1L17 20 v7 z"
      fill="#fff"
    />
  </svg>
{:else}
  <!-- neutral fallback — generic database cylinder -->
  <svg width={size} height={size} viewBox="0 0 40 40" fill="none" class={cls} aria-hidden="true">
    <rect x="2" y="2" width="36" height="36" rx="9" fill="#374151" />
    <ellipse cx="20" cy="14" rx="9" ry="3" fill="#9CA3AF" />
    <path
      d="M11 14 V26 C11 28 15 29 20 29 C25 29 29 28 29 26 V14"
      stroke="#9CA3AF"
      stroke-width="1.4"
      fill="none"
    />
    <ellipse cx="20" cy="20" rx="9" ry="3" stroke="#9CA3AF" stroke-width="1" fill="none" />
  </svg>
{/if}
