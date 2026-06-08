<!--
  SettingsPanel — the content shell for one settings tab.

  In the left-tab layout the nav supplies the category identity, so a panel
  doesn't repeat an icon+title column the way the old stacked cards did. It just
  leads with a title + optional one-line description, then renders its rows.
-->
<script lang="ts">
  import type { Snippet } from "svelte";

  interface Props {
    title: string;
    description?: string;
    children: Snippet;
  }
  let { title, description, children }: Props = $props();
</script>

<section>
  <header class="mb-5">
    <h2 class="text-[17px] font-semibold tracking-tight text-fg">{title}</h2>
    {#if description}
      <p class="text-[13px] text-fg-muted mt-1 leading-relaxed">{description}</p>
    {/if}
  </header>
  {@render children()}
</section>
