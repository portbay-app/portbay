<!--
  DatabaseMark — official brand logo for a database engine.

  Each engine's real logo is served from `static/databases/` (see the LOGOS
  map). The assets are transparent foreground glyphs/wordmarks in the brand's
  own colours — several are dark (SQLite navy, MariaDB navy, MySQL navy) — so
  they sit centred on a white rounded tile to stay legible on PortBay's dark
  shell and keep a consistent footprint across engines. `id` is the engine id
  (`mysql`, `postgres`, …); unknown ids fall back to a neutral cylinder.
-->
<script lang="ts">
  interface Props {
    id: string;
    size?: number;
    class?: string;
  }
  let { id, size = 36, class: cls = "" }: Props = $props();

  /** Engine id → official logo path under static/databases/. */
  const LOGOS: Record<string, string> = {
    mysql: "/databases/mysql.svg",
    mariadb: "/databases/mariadb.svg",
    postgres: "/databases/postgres.png",
    sqlite: "/databases/sqlite.jpg",
    redis: "/databases/redis.svg",
    mongo: "/databases/mongo.svg",
    memcached: "/databases/memcached.svg",
  };

  const src = $derived(LOGOS[id]);
</script>

{#if src}
  <span class="db-mark {cls}" style="width:{size}px;height:{size}px" aria-hidden="true">
    <img {src} alt="" />
  </span>
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

<style>
  .db-mark {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: #fff;
    border-radius: 22%;
    overflow: hidden;
    /* hairline ring so the white tile reads as a tile, not a glare, on dark */
    box-shadow: inset 0 0 0 1px rgba(0, 0, 0, 0.08);
  }
  .db-mark img {
    width: 78%;
    height: 78%;
    object-fit: contain;
  }
</style>
