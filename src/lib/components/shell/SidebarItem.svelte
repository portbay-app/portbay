<!--
  SidebarItem — one navigation row.

  Renders as an <a> for native browser behavior + SvelteKit's client-side
  routing. Active state derived from $page.url.pathname, with a 4px
  cyan-blue stripe on the left when active.
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
  }
  let { href, icon, label, matchPrefix = false }: Props = $props();

  const active = $derived.by(() => {
    const path = page.url.pathname;
    if (matchPrefix) return path === href || path.startsWith(`${href}/`);
    return path === href;
  });
</script>

<a
  {href}
  data-active={active}
  class="group relative flex items-center gap-2.5 px-3 py-2 rounded-md text-sm transition-colors
         text-fg-muted hover:text-fg hover:bg-surface-2
         data-[active=true]:text-fg data-[active=true]:bg-accent/8"
>
  <!-- Active stripe -->
  <span
    aria-hidden="true"
    class="absolute left-0 top-1.5 bottom-1.5 w-[3px] rounded-r-full transition-opacity
           bg-accent
           opacity-0 group-data-[active=true]:opacity-100"
  ></span>
  <Icon name={icon} size={16} />
  <span class="truncate">{label}</span>
</a>
