<!--
  Remote deploy / run modal — the dialog chrome around DeployPane. Opened per SSH
  connection from the deployPanel store (SSH host split menu, tunnel detail). The
  SSH host workspace embeds DeployPane directly instead of going through this.

  The pane owns the run state and mirrors it onto `busy` so we refuse to close
  (Escape / backdrop) while a deploy is mid-run.
-->
<script lang="ts">
  import DeployPane from "$lib/components/connections/DeployPane.svelte";

  let {
    connectionId,
    label,
    onClose,
  }: { connectionId: string; label: string; onClose: () => void } = $props();

  let busy = $state(false);

  function onKeydown(ev: KeyboardEvent) {
    if (ev.key === "Escape" && !busy) onClose();
  }
</script>

<svelte:window onkeydown={onKeydown} />

<div
  class="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4 backdrop-blur-sm"
  role="presentation"
  onclick={(e) => {
    if (e.target === e.currentTarget && !busy) onClose();
  }}
>
  <div
    class="flex h-[80vh] w-full max-w-3xl flex-col overflow-hidden rounded-xl border border-border bg-surface shadow-2xl"
    role="dialog"
    aria-modal="true"
    aria-label="Remote deploy"
  >
    <DeployPane {connectionId} {label} {onClose} bind:busy />
  </div>
</div>
