<!--
  SFTP file-manager modal — the dialog chrome around FileBrowserPane. Opened per
  SSH connection from the fileBrowser store (SSH host split menu, tunnel detail).
  The SSH host workspace embeds FileBrowserPane directly instead of going through
  this.

  A modal rather than a second pane: it keeps the uncommitted SSH-page WIP
  untouched and works from any tunnel that references the connection. The pane
  owns the Escape cascade (it closes its own sub-modals first), so the wrapper
  only adds the backdrop-click close.
-->
<script lang="ts">
  import FileBrowserPane from "$lib/components/connections/FileBrowserPane.svelte";

  let {
    connectionId,
    label,
    onClose,
  }: { connectionId: string; label: string; onClose: () => void } = $props();
</script>

<!-- Backdrop -->
<div
  class="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4 backdrop-blur-sm"
  role="presentation"
  onclick={(e) => {
    if (e.target === e.currentTarget) onClose();
  }}
>
  <div
    class="flex h-[80vh] w-full max-w-4xl flex-col overflow-hidden rounded-xl border border-border bg-surface shadow-2xl"
    role="dialog"
    aria-modal="true"
    aria-label="Remote files"
  >
    <FileBrowserPane {connectionId} {label} {onClose} />
  </div>
</div>
