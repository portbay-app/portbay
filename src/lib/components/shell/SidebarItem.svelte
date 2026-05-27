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
    /** Icon-only rendering for the collapsed (compact-density) sidebar. */
    collapsed?: boolean;
  }
  let { href, icon, label, matchPrefix = false, collapsed = false }: Props =
    $props();

  const active = $derived.by(() => {
    const path = page.url.pathname;
    if (matchPrefix) return path === href || path.startsWith(`${href}/`);
    return path === href;
  });
</script>

<a
  {href}
  data-active={active}
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
  {/if}
</a>
