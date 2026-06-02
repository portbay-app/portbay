<!--
  SshProcesses — a point-in-time process list for the host. Runs `ps aux` over
  the exec layer (so it shares the credential-prompt flow), parses it, and shows
  the top processes by CPU with a refresh and a kill action (confirmed, TERM or
  forced -9). Honest about being a snapshot, not a live `top` — refresh re-runs.
-->
<script lang="ts">
  import Icon from "$lib/components/atoms/Icon.svelte";
  import { invokeQuiet } from "$lib/ipc";
  import { connectWithPrompt } from "$lib/ssh/connectWithPrompt";
  import { confirmDialog } from "$lib/stores/confirm.svelte";
  import { relativeTime } from "$lib/ssh/hostFormat";
  import type { ExecResult } from "$lib/types/sshTunnels";

  let {
    connectionId,
    label,
    active = false,
  }: { connectionId: string; label: string; active?: boolean } = $props();

  interface Proc {
    pid: string;
    user: string;
    cpu: number;
    mem: number;
    command: string;
  }

  let procs = $state<Proc[]>([]);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let stampedAt = $state<number | null>(null);
  let killing = $state<string | null>(null);

  // `ps aux` is the portable form (Linux + macOS/BSD). Columns:
  //   USER PID %CPU %MEM VSZ RSS TTY STAT START TIME COMMAND…
  // Command is everything from the 11th field on, so split off the first 10.
  function parse(stdout: string): Proc[] {
    const lines = stdout.split("\n").filter((l) => l.trim());
    if (lines.length <= 1) return [];
    const out: Proc[] = [];
    for (const line of lines.slice(1)) {
      const cols = line.trim().split(/\s+/);
      if (cols.length < 11) continue;
      out.push({
        user: cols[0],
        pid: cols[1],
        cpu: Number(cols[2]) || 0,
        mem: Number(cols[3]) || 0,
        command: cols.slice(10).join(" "),
      });
    }
    out.sort((a, b) => b.cpu - a.cpu);
    return out.slice(0, 60);
  }

  async function refresh() {
    if (loading) return;
    loading = true;
    error = null;
    try {
      const result = await connectWithPrompt(connectionId, label, (cred) =>
        invokeQuiet<ExecResult>("ssh_exec_run", {
          input: {
            connectionId,
            command: "ps aux",
            password: cred?.kind === "password" ? cred.secret : undefined,
            passphrase: cred?.kind === "passphrase" ? cred.secret : undefined,
          },
        }),
      );
      procs = parse(result.stdout ?? "");
      stampedAt = Math.floor(Date.now() / 1000);
      if (procs.length === 0) error = "Couldn't read the process list on this host.";
    } catch {
      /* connectWithPrompt already toasted */
    } finally {
      loading = false;
    }
  }

  // Auto-load the list the first time the Processes tab is opened, so it shows
  // immediately instead of waiting for a click. Latched so toggling back to the
  // tab keeps the existing snapshot (the user re-runs it with Refresh); the
  // session is already warm from the workspace, so this adds no extra prompt.
  let autoLoaded = false;
  $effect(() => {
    if (active && !autoLoaded) {
      autoLoaded = true;
      void refresh();
    }
  });

  async function kill(p: Proc) {
    const choice = await confirmDialog.open({
      title: `End process ${p.pid}?`,
      message: `${p.command}\n\nSends a signal to PID ${p.pid} on ${label}.`,
      destructive: true,
      icon: "circle-stop",
      actions: [
        { label: "End (SIGTERM)", value: "term", icon: "circle-stop" },
        { label: "Force kill (-9)", value: "kill", tone: "destructive", icon: "trash-2" },
      ],
    });
    if (choice !== "term" && choice !== "kill") return;
    killing = p.pid;
    try {
      await connectWithPrompt(connectionId, label, (cred) =>
        invokeQuiet<ExecResult>("ssh_exec_run", {
          input: {
            connectionId,
            command: `kill ${choice === "kill" ? "-9 " : ""}${p.pid}`,
            password: cred?.kind === "password" ? cred.secret : undefined,
            passphrase: cred?.kind === "passphrase" ? cred.secret : undefined,
          },
        }),
      );
      await refresh();
    } catch {
      /* toasted */
    } finally {
      killing = null;
    }
  }
</script>

<div class="flex h-full min-h-0 flex-col">
  <header class="flex items-center gap-2 border-b border-border/60 px-6 py-3">
    <Icon name="activity" size={15} class="text-fg-muted" />
    <div class="min-w-0 flex-1">
      <h2 class="text-[13px] font-semibold text-fg">Processes</h2>
      <p class="text-[11px] text-fg-subtle">
        {stampedAt ? `Snapshot · ${relativeTime(stampedAt)} · top by CPU` : "ps aux — point-in-time, not live"}
      </p>
    </div>
    <button
      type="button"
      onclick={refresh}
      disabled={loading}
      class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md text-[12px] font-medium border border-border text-fg-muted hover:text-fg hover:bg-surface-2 disabled:opacity-50"
    >
      <Icon name="refresh-cw" size={12} class={loading ? "animate-spin" : ""} />
      {procs.length ? "Refresh" : "Load"}
    </button>
  </header>

  <div class="min-h-0 flex-1 overflow-y-auto">
    {#if error && procs.length === 0}
      <div class="m-4 rounded-md border border-status-crashed/40 bg-status-crashed/10 p-3 text-[12px] text-status-crashed">
        {error}
      </div>
    {:else if procs.length === 0}
      <div class="flex h-full items-center justify-center">
        <button
          type="button"
          onclick={refresh}
          disabled={loading}
          class="inline-flex items-center gap-2 rounded-lg border border-border px-3.5 py-2 text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg disabled:opacity-50"
        >
          <Icon name={loading ? "refresh-cw" : "activity"} size={14} class={loading ? "animate-spin" : ""} />
          {loading ? "Reading processes…" : "Load process list"}
        </button>
      </div>
    {:else}
      <table class="w-full text-[12px]">
        <thead class="sticky top-0 bg-surface text-left text-[11px] uppercase text-fg-subtle">
          <tr class="border-b border-border">
            <th class="px-4 py-1.5 font-medium">PID</th>
            <th class="px-2 py-1.5 text-right font-medium">CPU%</th>
            <th class="px-2 py-1.5 text-right font-medium">MEM%</th>
            <th class="px-2 py-1.5 font-medium">User</th>
            <th class="px-4 py-1.5 font-medium">Command</th>
            <th class="px-4 py-1.5"></th>
          </tr>
        </thead>
        <tbody>
          {#each procs as p (p.pid)}
            <tr class="group border-b border-border/40 hover:bg-surface-2/50">
              <td class="px-4 py-1.5 font-mono text-fg-muted">{p.pid}</td>
              <td class="px-2 py-1.5 text-right font-mono tabular-nums {p.cpu >= 50 ? 'text-status-unhealthy' : 'text-fg'}">{p.cpu.toFixed(1)}</td>
              <td class="px-2 py-1.5 text-right font-mono tabular-nums text-fg-muted">{p.mem.toFixed(1)}</td>
              <td class="px-2 py-1.5 truncate text-fg-subtle">{p.user}</td>
              <td class="px-4 py-1.5">
                <span class="block max-w-[420px] truncate font-mono text-fg" title={p.command}>{p.command}</span>
              </td>
              <td class="px-4 py-1.5 text-right">
                <button
                  type="button"
                  onclick={() => kill(p)}
                  disabled={killing === p.pid}
                  class="inline-flex items-center gap-1 rounded p-1 text-fg-subtle opacity-0 hover:bg-status-crashed/10 hover:text-status-crashed group-hover:opacity-100 disabled:opacity-50"
                  title="End process"
                >
                  <Icon name={killing === p.pid ? "refresh-cw" : "circle-stop"} size={13} class={killing === p.pid ? "animate-spin" : ""} />
                </button>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </div>
</div>
