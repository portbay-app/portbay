<!--
  (connections) — thin shell shared by the two remote-access surfaces, SSH
  (/ssh) and Cloudflare Tunnel (/tunnels). Each now has its own top-level
  sidebar entry, so this group no longer draws an inner SSH/Cloudflare rail —
  both child pages render standalone, full-width.

  This is a SvelteKit route *group*: the `(connections)` segment never appears
  in the URL, so the child routes stay `/ssh` and `/tunnels` — every existing
  deep link, command-palette entry, and TopBar `goto` keeps working untouched.

  The group still owns the SSH SFTP file browser + deploy panel overlays, which
  open over whichever surface triggered them (driven by their own stores). The
  Cloudflare page never sets those targets, so they never render there.
-->
<script lang="ts">
  import type { Snippet } from "svelte";

  import { fileBrowser } from "$lib/stores/fileBrowser.svelte";
  import FileBrowser from "$lib/components/connections/FileBrowser.svelte";
  import { deployPanel } from "$lib/stores/deployPanel.svelte";
  import DeployPanel from "$lib/components/connections/DeployPanel.svelte";

  let { children }: { children: Snippet } = $props();
</script>

<div class="h-full min-w-0">
  {@render children()}
</div>

{#if fileBrowser.target}
  {@const t = fileBrowser.target}
  <FileBrowser connectionId={t.connectionId} label={t.label} onClose={() => fileBrowser.close()} />
{/if}

{#if deployPanel.target}
  {@const t = deployPanel.target}
  <DeployPanel connectionId={t.connectionId} label={t.label} onClose={() => deployPanel.close()} />
{/if}
