<!--
  StopAllButton — universal kill switch.

  Card #3 ships the visual + accessibility shell. Card #7 wires it to the
  `stop_all` Tauri command, two-step confirm, and the post-action report.

  Stays interactive (logs to console) so the shell isn't visually broken —
  but doesn't actually call any Tauri command yet.
-->
<script lang="ts">
  import Icon from "$lib/components/atoms/Icon.svelte";

  function stopAllStub() {
    // Card #7: replace with safeInvoke('stop_all') + StopAllReport rendering.
    console.info("[stop-all] stub — wired in card #7");
  }
</script>

<button
  type="button"
  onclick={stopAllStub}
  title="Stop all running projects"
  aria-label="Stop all running projects"
  class="inline-flex items-center justify-center w-8 h-8 rounded-md
         text-status-crashed border border-status-crashed/30
         hover:bg-status-crashed/10 hover:border-status-crashed/60
         transition-colors"
>
  <Icon name="circle-stop" size={16} />
</button>
