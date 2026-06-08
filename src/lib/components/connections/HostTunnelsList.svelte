<!--
  HostTunnelsList — the port-forwards riding on one SSH host: a header with a
  count + Add button, then each tunnel's status dot, endpoints, and start/stop
  control. Shared by the SSH host detail panel (right rail) and the host
  workspace's Tunnels tab so both render the list identically.
-->
<script lang="ts">
  import Icon from "$lib/components/atoms/Icon.svelte";
  import { openUrl } from "$lib/security/openUrl";
  import { sshTunnels } from "$lib/stores/sshTunnels.svelte";
  import type { SshTunnelRuntimeStatus } from "$lib/types/sshTunnels";

  let {
    tunnels,
    onOpenTunnel,
    onAddTunnel,
  }: {
    tunnels: SshTunnelRuntimeStatus[];
    onOpenTunnel: (id: string) => void;
    onAddTunnel: () => void;
  } = $props();

  function statusFor(t: SshTunnelRuntimeStatus): "running" | "stopped" | "starting" {
    if (sshTunnels.isBusy(t.id)) return "starting";
    return t.running ? "running" : "stopped";
  }

  // A running local forward is reachable at its local endpoint — offer to open
  // it in the browser (covers Jupyter/TensorBoard/etc. and any web service).
  function openLocal(t: SshTunnelRuntimeStatus) {
    void openUrl(`http://${t.localHost}:${t.localPort}`);
  }
</script>

<div class="mb-2 flex items-center gap-2">
  <h3 class="text-[12px] font-semibold text-fg">Port forwards</h3>
  <span class="text-[11px] tabular-nums text-fg-subtle">{tunnels.length}</span>
  <button
    type="button"
    onclick={onAddTunnel}
    class="ml-auto inline-flex items-center gap-1 h-7 px-2 rounded-md text-[11.5px] font-medium border border-border text-fg-muted hover:text-fg hover:bg-surface-2"
  >
    <Icon name="plus" size={12} /> Add
  </button>
</div>
{#if tunnels.length === 0}
  <p class="rounded-lg border border-dashed border-border px-3 py-4 text-center text-[11.5px] text-fg-subtle">
    No tunnels on this host yet.
  </p>
{:else}
  <div class="space-y-1.5">
    {#each tunnels as t (t.id)}
      {@const busy = sshTunnels.isBusy(t.id)}
      <div class="flex items-center gap-2 rounded-lg border border-border/70 bg-surface px-3 py-2">
        <span class="w-1.5 h-1.5 rounded-full {statusFor(t) === 'running' ? 'bg-status-running' : statusFor(t) === 'starting' ? 'bg-status-starting animate-pulse' : 'bg-status-stopped'}"></span>
        <button type="button" onclick={() => onOpenTunnel(t.id)} class="min-w-0 flex-1 text-left">
          <span class="block truncate text-[12px] font-medium text-fg">{t.name}</span>
          <span class="block truncate font-mono text-[10.5px] text-fg-subtle">{t.localHost}:{t.localPort} → {t.remoteHost}:{t.remotePort}</span>
        </button>
        {#if t.running}
          {#if t.forwardKind === "local"}
            <button type="button" onclick={() => openLocal(t)} title={`Open http://${t.localHost}:${t.localPort}`} class="shrink-0 inline-flex items-center gap-1 h-7 px-2 rounded-md text-[11px] font-medium text-fg-muted border border-border hover:bg-surface-2 hover:text-fg">
              <Icon name="external-link" size={11} /> Open
            </button>
          {/if}
          <button type="button" onclick={() => sshTunnels.stop(t.id)} disabled={busy} class="shrink-0 inline-flex items-center gap-1 h-7 px-2 rounded-md text-[11px] font-medium text-status-crashed border border-status-crashed/40 hover:bg-status-crashed/10 disabled:opacity-50">
            <Icon name="circle-stop" size={11} /> Stop
          </button>
        {:else}
          <button type="button" onclick={() => sshTunnels.start(t.id)} disabled={busy} class="shrink-0 inline-flex items-center gap-1 h-7 px-2 rounded-md text-[11px] font-medium bg-surface-2 text-fg hover:bg-surface-2/70 disabled:opacity-50">
            <Icon name={busy ? "refresh-cw" : "play"} size={11} class={busy ? "animate-spin" : ""} /> Start
          </button>
        {/if}
      </div>
    {/each}
  </div>
{/if}
