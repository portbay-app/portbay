<!--
  MlDashboards — one-click quick-forward presets for the dashboards an ML
  researcher reaches for: Jupyter, TensorBoard, the Ray dashboard, W&B local,
  code-server. Each preset creates *and starts* a local forward via the same
  tunnel machinery the Ports tab uses, then opens the forwarded
  http://127.0.0.1:<localPort> in the browser — so "I want TensorBoard" is one
  click instead of hand-building a tunnel and remembering the local port.

  Presets de-dupe against existing local forwards (keyed by remote port), so a
  dashboard that's already wired shows "Open" instead of "Forward". The host's
  secret is cached for the session, so neither action re-prompts.
-->
<script lang="ts">
  import Icon from "$lib/components/atoms/Icon.svelte";
  import type { IconName } from "$lib/components/atoms/Icon.svelte";
  import { openUrl } from "$lib/security/openUrl";
  import { sshConnections } from "$lib/stores/sshConnections.svelte";
  import { sshTunnels } from "$lib/stores/sshTunnels.svelte";

  let { connectionId }: { connectionId: string; label: string } = $props();

  interface Preset {
    key: string;
    name: string;
    /** Default remote port the tool listens on. */
    port: number;
    /** One-line "what this is", so the list reads without prior knowledge. */
    blurb: string;
    icon: IconName;
  }

  // Canonical default ports. code-server and W&B local both default to 8080 —
  // that's genuinely how they ship, so forwarding one lights up the other (same
  // remote port, same forwarded URL); we keep both rows for discoverability.
  const PRESETS: Preset[] = [
    { key: "jupyter", name: "Jupyter", port: 8888, blurb: "Notebooks", icon: "file-code" },
    { key: "tensorboard", name: "TensorBoard", port: 6006, blurb: "Training curves", icon: "activity" },
    { key: "ray", name: "Ray dashboard", port: 8265, blurb: "Cluster & jobs", icon: "layers" },
    { key: "wandb", name: "W&B local", port: 8080, blurb: "Experiment tracking", icon: "gauge" },
    { key: "code-server", name: "code-server", port: 8080, blurb: "VS Code in the browser", icon: "terminal" },
  ];

  const host = $derived(sshConnections.find(connectionId));

  // Saved local forwards on this host, keyed by remote port, so a preset can
  // show its live local port + Open instead of a Forward button. Reactive off
  // the store so it flips the instant a forward is created.
  const forwardedByRemotePort = $derived.by(() => {
    const map = new Map<number, { id: string; localPort: number; running: boolean }>();
    for (const t of sshTunnels.value) {
      if (t.connectionId !== connectionId || t.forwardKind !== "local") continue;
      map.set(t.remotePort, { id: t.id, localPort: t.localPort, running: t.running });
    }
    return map;
  });

  let working = $state<Record<string, boolean>>({});

  function localUrl(port: number): string {
    return `http://127.0.0.1:${port}`;
  }

  // Forward (create + start) then open. One click from "I want Jupyter" to the
  // tab. If the port's already forwarded we skip straight to opening it.
  async function launch(p: Preset) {
    if (working[p.key]) return;
    const existing = forwardedByRemotePort.get(p.port);
    if (existing) {
      if (!existing.running) await sshTunnels.start(existing.id);
      await openUrl(localUrl(existing.localPort));
      return;
    }
    const h = host;
    if (!h) return;
    working = { ...working, [p.key]: true };
    try {
      const saved = await sshTunnels.save({
        name: `${p.name} :${p.port}`,
        sshHost: h.sshHost,
        sshPort: h.sshPort,
        sshUser: h.sshUser,
        authKind: h.authKind,
        keyPath: h.keyPath,
        localHost: "127.0.0.1",
        // Let the backend pick a free local port near the remote one.
        localPort: null,
        remoteHost: "localhost",
        remotePort: p.port,
        forwardKind: "local",
        keepAlive: false,
        autoReconnect: false,
      });
      if (saved) {
        await sshTunnels.start(saved.id);
        await openUrl(localUrl(saved.localPort));
      }
    } finally {
      working = { ...working, [p.key]: false };
    }
  }

  async function open(localPort: number, id: string, running: boolean) {
    if (!running) await sshTunnels.start(id);
    await openUrl(localUrl(localPort));
  }
</script>

<section class="mb-4">
  <div class="mb-2 flex items-center gap-2">
    <h3 class="text-[12px] font-semibold text-fg">ML dashboards</h3>
    <span class="text-[11px] text-fg-subtle">quick-forward</span>
  </div>
  <div class="space-y-1.5">
    {#each PRESETS as p (p.key)}
      {@const fwd = forwardedByRemotePort.get(p.port)}
      <div class="flex items-center gap-2.5 rounded-lg border border-border/70 bg-surface px-3 py-2">
        <span class="grid h-7 w-7 shrink-0 place-items-center rounded-md bg-surface-2 text-fg-muted">
          <Icon name={p.icon} size={14} />
        </span>
        <div class="min-w-0 flex-1">
          <div class="flex items-baseline gap-1.5">
            <span class="truncate text-[12px] font-medium text-fg">{p.name}</span>
            <span class="font-mono text-[10.5px] text-fg-subtle">:{p.port}</span>
          </div>
          <p class="truncate text-[10.5px] text-fg-subtle">
            {#if fwd}
              <span class="inline-flex items-center gap-1 {fwd.running ? 'text-status-running' : 'text-fg-subtle'}">
                <span class="h-1.5 w-1.5 rounded-full {fwd.running ? 'bg-status-running' : 'bg-status-stopped'}"></span>
                127.0.0.1:{fwd.localPort}
              </span>
            {:else}
              {p.blurb}
            {/if}
          </p>
        </div>
        {#if fwd}
          <button
            type="button"
            onclick={() => open(fwd.localPort, fwd.id, fwd.running)}
            class="shrink-0 inline-flex items-center gap-1 h-7 px-2.5 rounded-md text-[11.5px] font-medium border border-border text-fg-muted hover:bg-surface-2 hover:text-fg"
            title={`Open http://127.0.0.1:${fwd.localPort}`}
          >
            <Icon name="external-link" size={12} /> Open
          </button>
        {:else}
          <button
            type="button"
            onclick={() => launch(p)}
            disabled={working[p.key] || !host}
            class="shrink-0 inline-flex items-center gap-1 h-7 px-2.5 rounded-md text-[11.5px] font-medium bg-surface-2 text-fg hover:bg-surface-2/70 disabled:opacity-50"
            title={`Forward ${p.port} and open ${p.name}`}
          >
            <Icon name={working[p.key] ? "refresh-cw" : "arrow-right"} size={12} class={working[p.key] ? "animate-spin" : ""} />
            Forward
          </button>
        {/if}
      </div>
    {/each}
  </div>
</section>
