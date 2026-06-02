<!--
  SshPorts — a point-in-time list of the TCP ports the host is listening on,
  scanned over the exec layer (`ss`/`netstat`, same credential flow as the rest
  of the workspace). Each row offers a one-click "Forward" that creates *and
  starts* a local tunnel via the existing tunnel machinery — no hand-built form.

  Ports already covered by a saved local forward are shown as "Forwarded" with
  their local port instead of a button, so the list de-dupes against what's
  already wired. Well-known ports get a friendly service label. Like Processes,
  this is an explicit-refresh snapshot, not a live `watch`.
-->
<script lang="ts">
  import Icon from "$lib/components/atoms/Icon.svelte";
  import { invokeQuiet } from "$lib/ipc";
  import { connectWithPrompt } from "$lib/ssh/connectWithPrompt";
  import { sshTunnels } from "$lib/stores/sshTunnels.svelte";
  import { relativeTime } from "$lib/ssh/hostFormat";
  import {
    parseListeningPorts,
    SCAN_COMMAND,
    wellKnownLabel,
    type DetectedPort,
  } from "$lib/ssh/portScan";
  import type { SshConnectionView } from "$lib/types/sshConnections";
  import type { ExecResult } from "$lib/types/sshTunnels";

  let {
    connectionId,
    label,
    host,
    active = false,
  }: {
    connectionId: string;
    label: string;
    host: SshConnectionView;
    active?: boolean;
  } = $props();

  let ports = $state<DetectedPort[]>([]);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let stampedAt = $state<number | null>(null);
  let forwarding = $state<Record<number, boolean>>({});

  // Saved local forwards on this host, keyed by the remote port they target, so
  // a detected port can show its existing tunnel instead of a Forward button.
  // Reactive off the store so it updates the moment a new forward is created.
  const forwardedByRemotePort = $derived.by(() => {
    const map = new Map<number, { id: string; localPort: number; running: boolean }>();
    for (const t of sshTunnels.value) {
      if (t.connectionId !== host.id) continue;
      if (t.forwardKind !== "local") continue;
      map.set(t.remotePort, { id: t.id, localPort: t.localPort, running: t.running });
    }
    return map;
  });

  async function refresh() {
    if (loading) return;
    loading = true;
    error = null;
    try {
      const result = await connectWithPrompt(connectionId, label, (cred) =>
        invokeQuiet<ExecResult>("ssh_exec_run", {
          input: {
            connectionId,
            command: SCAN_COMMAND,
            password: cred?.kind === "password" ? cred.secret : undefined,
            passphrase: cred?.kind === "passphrase" ? cred.secret : undefined,
          },
        }),
      );
      ports = parseListeningPorts(result.stdout ?? "");
      stampedAt = Math.floor(Date.now() / 1000);
      if (ports.length === 0)
        error = "No listening TCP ports found — or this host has neither `ss` nor `netstat`.";
    } catch {
      /* connectWithPrompt already surfaced any real failure */
    } finally {
      loading = false;
    }
  }

  // Auto-scan the first time the Ports tab is opened (the session is already
  // warm from the workspace, so this adds no extra prompt). Latched so toggling
  // back keeps the snapshot — the user re-runs it with Refresh.
  let autoLoaded = false;
  $effect(() => {
    if (active && !autoLoaded) {
      autoLoaded = true;
      void refresh();
    }
  });

  async function forward(p: DetectedPort) {
    if (forwarding[p.port]) return;
    forwarding = { ...forwarding, [p.port]: true };
    try {
      const name = p.processName ? `${p.processName} :${p.port}` : `Port ${p.port}`;
      const saved = await sshTunnels.save({
        name,
        sshHost: host.sshHost,
        sshPort: host.sshPort,
        sshUser: host.sshUser,
        authKind: host.authKind,
        keyPath: host.keyPath,
        localHost: "127.0.0.1",
        // Let the backend pick a free local port near the remote one.
        localPort: null,
        remoteHost: "localhost",
        remotePort: p.port,
        forwardKind: "local",
        keepAlive: false,
        autoReconnect: false,
      });
      // One-click means usable: bring the forward up right away. The host's
      // secret is already cached this session, so this won't re-prompt.
      if (saved) await sshTunnels.start(saved.id);
    } finally {
      forwarding = { ...forwarding, [p.port]: false };
    }
  }

  function serviceLabel(p: DetectedPort): string | null {
    return wellKnownLabel(p.port) ?? p.processName;
  }
</script>

<div class="flex h-full min-h-0 flex-col">
  <header class="flex items-center gap-2 border-b border-border/60 px-6 py-3">
    <Icon name="circle-dot" size={15} class="text-fg-muted" />
    <div class="min-w-0 flex-1">
      <h2 class="text-[13px] font-semibold text-fg">Ports</h2>
      <p class="text-[11px] text-fg-subtle">
        {stampedAt
          ? `Snapshot · ${relativeTime(stampedAt)} · listening TCP`
          : "ss / netstat — point-in-time, not live"}
      </p>
    </div>
    <button
      type="button"
      onclick={refresh}
      disabled={loading}
      class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md text-[12px] font-medium border border-border text-fg-muted hover:text-fg hover:bg-surface-2 disabled:opacity-50"
    >
      <Icon name="refresh-cw" size={12} class={loading ? "animate-spin" : ""} />
      {ports.length ? "Refresh" : "Scan"}
    </button>
  </header>

  <div class="min-h-0 flex-1 overflow-y-auto">
    {#if error && ports.length === 0}
      <div class="m-4 rounded-md border border-status-crashed/40 bg-status-crashed/10 p-3 text-[12px] text-status-crashed">
        {error}
      </div>
    {:else if ports.length === 0}
      <div class="flex h-full items-center justify-center">
        <button
          type="button"
          onclick={refresh}
          disabled={loading}
          class="inline-flex items-center gap-2 rounded-lg border border-border px-3.5 py-2 text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg disabled:opacity-50"
        >
          <Icon name={loading ? "refresh-cw" : "circle-dot"} size={14} class={loading ? "animate-spin" : ""} />
          {loading ? "Scanning ports…" : "Scan listening ports"}
        </button>
      </div>
    {:else}
      <table class="w-full text-[12px]">
        <thead class="sticky top-0 bg-surface text-left text-[11px] uppercase text-fg-subtle">
          <tr class="border-b border-border">
            <th class="px-4 py-1.5 font-medium">Port</th>
            <th class="px-2 py-1.5 font-medium">Service</th>
            <th class="px-2 py-1.5 font-medium">Address</th>
            <th class="px-4 py-1.5 text-right font-medium">Forward</th>
          </tr>
        </thead>
        <tbody>
          {#each ports as p (p.port)}
            {@const forwarded = forwardedByRemotePort.get(p.port)}
            {@const service = serviceLabel(p)}
            <tr class="group border-b border-border/40 hover:bg-surface-2/50">
              <td class="px-4 py-1.5 font-mono tabular-nums text-fg">{p.port}</td>
              <td class="px-2 py-1.5 text-fg-muted">
                {#if service}
                  <span class="truncate" title={p.processName ?? undefined}>{service}</span>
                  {#if p.pid}<span class="ml-1 text-fg-subtle">#{p.pid}</span>{/if}
                {:else}
                  <span class="text-fg-subtle">—</span>
                {/if}
              </td>
              <td class="px-2 py-1.5 font-mono text-fg-subtle">{p.address}</td>
              <td class="px-4 py-1.5 text-right">
                {#if forwarded}
                  <span
                    class="inline-flex items-center gap-1.5 text-[11.5px] {forwarded.running ? 'text-status-running' : 'text-fg-subtle'}"
                    title={forwarded.running ? "Tunnel is live" : "Forward saved — start it from Tunnels"}
                  >
                    <Icon name="circle-check" size={12} />
                    localhost:{forwarded.localPort}
                  </span>
                {:else}
                  <button
                    type="button"
                    onclick={() => forward(p)}
                    disabled={forwarding[p.port]}
                    class="inline-flex items-center gap-1 rounded-md h-7 px-2 text-[11.5px] font-medium text-fg-muted border border-border opacity-0 hover:bg-surface-2 hover:text-fg group-hover:opacity-100 disabled:opacity-50"
                    title="Create and start a local forward for this port"
                  >
                    <Icon name={forwarding[p.port] ? "refresh-cw" : "arrow-right"} size={12} class={forwarding[p.port] ? "animate-spin" : ""} />
                    Forward
                  </button>
                {/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </div>
</div>
