<!--
  /php — superseded by /languages.

  The PHP-specific panel was the v1 surface; the unified Languages
  container (P4 — Languages container card) replaced it. We redirect
  here so any existing bookmark / command-palette entry keeps working
  without a 404. The old detail view is deleted in a follow-up
  cleanup commit once /languages has settled in use.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";

  onMount(() => {
    void goto("/languages", { replaceState: true });
  });
</script>

<div class="p-6 text-xs text-fg-subtle">
  Redirecting to <a href="/languages" class="text-accent hover:underline">Languages</a>…
</div>
