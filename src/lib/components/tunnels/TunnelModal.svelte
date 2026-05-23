<!--
  TunnelModal — full-screen modal showing one project's live
  Cloudflare Tunnel. Mounts globally at the layout root and renders
  whenever `tunnelModal.id` is non-null.

  Lifecycle:
    - On open: poll `tunnel_status(id)` every 1 s until publicUrl
      lands (typically 2-6 s after the user clicked Share publicly).
    - "Copy URL" copies to clipboard.
    - "Stop sharing" calls `stop_tunnel` and closes.
    - Closing the modal does NOT stop the tunnel — running tunnels
      persist; the TopBar pill keeps showing the count.
-->
<script lang="ts">
  import { onDestroy, untrack } from "svelte";

  import { Icon } from "$lib/components/atoms";
  import { safeInvoke } from "$lib/ipc";
  import { errorBus } from "$lib/stores/errors.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import { tunnelModal } from "$lib/stores/tunnelModal.svelte";
  import { tunnels } from "$lib/stores/tunnels.svelte";
  import type { TunnelStatus } from "$lib/types/tunnel";

  const POLL_INTERVAL_MS = 1_000;

  let status = $state<TunnelStatus | null>(null);
  let stopping = $state<boolean>(false);
  let pollTimer: ReturnType<typeof setInterval> | null = null;

  const project = $derived(
    tunnelModal.id === null
      ? null
      : (projects.value.find((p) => p.id === tunnelModal.id) ?? null),
  );

  async function refresh() {
    if (tunnelModal.id === null) return;
    try {
      status = await safeInvoke<TunnelStatus | null>("tunnel_status", {
        id: tunnelModal.id,
      });
    } catch {
      status = null;
    }
  }

  function startPolling() {
    stopPolling();
    pollTimer = setInterval(() => void refresh(), POLL_INTERVAL_MS);
  }
  function stopPolling() {
    if (pollTimer !== null) {
      clearInterval(pollTimer);
      pollTimer = null;
    }
  }

  // Open / close + projectId switch handler.
  $effect(() => {
    const id = tunnelModal.id;
    if (id === null) {
      stopPolling();
      status = null;
      return;
    }
    untrack(() => {
      status = null;
      void refresh();
      startPolling();
    });
  });

  async function copyUrl() {
    if (!status?.publicUrl) return;
    try {
      await navigator.clipboard.writeText(status.publicUrl);
      errorBus.push({
        code: "COPIED",
        whatHappened: "Public URL copied.",
        whyItMatters: "Paste anywhere.",
        whoCausedIt: "system",
        actions: [],
      });
    } catch {
      // clipboard permission missing — fail quietly
    }
  }

  async function stopSharing() {
    if (tunnelModal.id === null) return;
    stopping = true;
    try {
      await safeInvoke("stop_tunnel", { id: tunnelModal.id });
      await tunnels.refresh();
      tunnelModal.hide();
    } catch {
      /* toast pushed */
    } finally {
      stopping = false;
    }
  }

  function close() {
    tunnelModal.hide();
  }

  function onKeydown(e: KeyboardEvent) {
    if (tunnelModal.id === null) return;
    if (e.key === "Escape") close();
  }

  onDestroy(() => stopPolling());
</script>

<svelte:window onkeydown={onKeydown} />

{#if tunnelModal.id !== null && project}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="fixed inset-0 z-50 bg-bg/70 backdrop-blur-sm flex items-center justify-center p-6"
    onclick={close}
  >
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      onclick={(e) => e.stopPropagation()}
      class="w-[520px] max-w-[95vw] bg-surface border border-border rounded-xl shadow-2xl flex flex-col overflow-hidden"
      role="dialog"
      aria-label="Cloudflare tunnel"
      aria-modal="true"
      tabindex="-1"
    >
      <!-- Header -->
      <header class="shrink-0 flex items-center gap-3 px-5 py-4 border-b border-border">
        <Icon name="globe" size={16} class="text-fg-muted" />
        <div class="flex-1 min-w-0">
          <h2 class="text-sm font-semibold text-fg truncate">{project.name}</h2>
          <p class="text-[11px] text-fg-muted font-mono truncate">
            shared from {project.url}
          </p>
        </div>
        <button
          type="button"
          onclick={close}
          title="Close (tunnel keeps running)"
          aria-label="Close"
          class="p-1.5 rounded-md text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
        >
          <Icon name="x" size={16} />
        </button>
      </header>

      <!-- Body -->
      <div class="p-5 space-y-3">
        {#if status === null || !status.publicUrl}
          <div class="flex items-center gap-2 text-sm text-fg-muted">
            <Icon name="refresh-cw" size={14} class="animate-spin" />
            <span>Establishing tunnel…</span>
          </div>
          <p class="text-[11px] text-fg-subtle">
            Cloudflare assigns a public URL within a few seconds. The tunnel
            is ephemeral — it disappears when you stop sharing or close the app.
          </p>
        {:else}
          <div>
            <div class="text-[11px] text-fg-muted">Public URL</div>
            <div
              class="mt-1 flex items-center gap-2 px-3 py-2 bg-bg/60 border border-border rounded-md"
            >
              <code class="flex-1 text-xs font-mono text-fg truncate">
                {status.publicUrl}
              </code>
              <button
                type="button"
                onclick={copyUrl}
                title="Copy"
                class="p-1 rounded text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
              >
                <Icon name="link" size={13} />
              </button>
            </div>
          </div>

          <p class="text-[11px] text-fg-subtle">
            Send this URL to a colleague, open it on a phone, or paste it into
            a webhook target. The tunnel routes traffic through Cloudflare's
            edge to <span class="font-mono">{status.upstreamUrl}</span> on
            this machine.
          </p>
        {/if}
      </div>

      <!-- Footer -->
      <footer
        class="shrink-0 px-5 py-3 border-t border-border flex items-center justify-between gap-3"
      >
        <span class="text-[11px] text-fg-subtle">
          Ephemeral · ESC to close · tunnel keeps running
        </span>
        <button
          type="button"
          onclick={stopSharing}
          disabled={stopping}
          class="text-xs px-3 py-1.5 rounded-md text-status-crashed border border-status-crashed/40 hover:bg-status-crashed/10 transition-colors disabled:opacity-50"
        >
          {stopping ? "Stopping…" : "Stop sharing"}
        </button>
      </footer>
    </div>
  </div>
{/if}
