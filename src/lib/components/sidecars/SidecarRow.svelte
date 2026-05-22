<!--
  SidecarRow — 4-up grid of SidecarCards. Starts the sidecars store's
  polling on mount and tears it down on unmount.

  Responsive: 4 columns at full width, 2×2 below ~900px, vertical stack
  below ~640px. The shell forces a minimum of 900×560 (tauri.conf.json),
  so the 2×2 fallback is rare in practice.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { sidecars } from "$lib/stores/sidecars.svelte";
  import { SIDECAR_ORDER } from "$lib/types/sidecars";
  import SidecarCard from "./SidecarCard.svelte";

  onMount(() => {
    sidecars.start();
    return () => sidecars.stop();
  });
</script>

<div class="grid grid-cols-1 sm:grid-cols-2 md:grid-cols-4 gap-3">
  {#each SIDECAR_ORDER as key (key)}
    <SidecarCard sidecarKey={key} info={sidecars.value[key]} />
  {/each}
</div>
