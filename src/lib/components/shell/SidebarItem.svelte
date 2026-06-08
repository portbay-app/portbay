<!--
  SidebarItem — one navigation row.

  Renders as an <a> for native browser behavior + SvelteKit's client-side
  routing. Active state derived from $page.url.pathname, with a 4px
  cyan-blue stripe on the left when active.

  In `collapsed` mode (compact density) the label is hidden and the icon is
  centered; the label moves to the native `title` tooltip so the rail still
  identifies each destination.
-->
<script lang="ts">
  import { page } from "$app/state";
  import Icon from "$lib/components/atoms/Icon.svelte";
  import type { IconName } from "$lib/components/atoms/Icon.svelte";

  interface Props {
    href: string;
    icon: IconName;
    label: string;
    /** When true, the route matches if the current path *starts with* href —
        useful for sections with sub-routes. */
    matchPrefix?: boolean;
    /** Extra paths that should also light this item active (prefix-matched).
        Used when one entry fronts several routes — e.g. "SSH & Tunnels"
        covering both /ssh and /tunnels. */
    matchPaths?: string[];
    /** Icon-only rendering for the collapsed (compact-density) sidebar. */
    collapsed?: boolean;
    /** Optional count shown as a small pill (or a dot when collapsed). Hidden
        when null/0 so an idle section stays quiet. */
    badge?: number | null;
  }
  let {
    href,
    icon,
    label,
    matchPrefix = false,
    matchPaths,
    collapsed = false,
    badge = null,
  }: Props = $props();

  const showBadge = $derived(typeof badge === "number" && badge > 0);

  const active = $derived.by(() => {
    const path = page.url.pathname;
    const hits = (p: string) => path === p || path.startsWith(`${p}/`);
    if (matchPaths) return matchPaths.some(hits);
    if (matchPrefix) return hits(href);
    return path === href;
  });
</script>

<a
  {href}
  data-active={active}
  draggable="false"
  title={collapsed ? label : undefined}
  aria-label={collapsed ? label : undefined}
  class="group relative flex items-center rounded-md text-sm transition-colors
         text-fg-muted hover:text-fg hover:bg-surface-2
         data-[active=true]:text-fg data-[active=true]:bg-accent/8
         {collapsed ? 'justify-center px-0 py-2' : 'gap-2.5 px-3 py-2'}"
>
  <!-- Active stripe -->
  <span
    aria-hidden="true"
    class="absolute left-0 top-1.5 bottom-1.5 w-[3px] rounded-r-full transition-opacity
           bg-accent
           opacity-0 group-data-[active=true]:opacity-100"
  ></span>
  <Icon name={icon} size={16} />
  {#if !collapsed}
    <span class="truncate">{label}</span>
    {#if showBadge}
      <span
        class="ml-auto shrink-0 inline-flex items-center justify-center min-w-[18px] h-[18px]
               px-1 rounded-full bg-status-running/15 text-status-running
               text-[10px] font-semibold tabular-nums"
        aria-label="{badge} running"
      >
        {badge}
      </span>
    {/if}
  {:else if showBadge}
    <!-- Collapsed: a small running dot in the corner. -->
    <span
      aria-hidden="true"
      class="absolute top-1 right-1 w-1.5 h-1.5 rounded-full bg-status-running"
    ></span>
  {/if}
</a>
