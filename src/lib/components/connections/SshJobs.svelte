<!--
  SshJobs — the researcher's "detached work" view: the long-running things that
  outlive an SSH session. Two parts over one snapshot exec (shared credential
  prompt, point-in-time, not live):

    • Persistent sessions — `tmux ls` / `screen -ls`, with one-click **Attach**
      (opens the session in a real terminal pane, since attach is interactive)
      and a "new persistent session" that wraps a shell in tmux so a dropped SSH
      doesn't kill the run.
    • SLURM jobs — `squeue` for the user's live jobs (with **Cancel** = scancel)
      plus a little `sacct` history. Gated behind `command -v squeue`: on a box
      without a scheduler the whole section is hidden, the same graceful-degrade
      SshGpu does for a missing nvidia-smi.

  Scope is visibility + attach/cancel — not a scheduler UI.
-->
<script lang="ts">
  import Icon from "$lib/components/atoms/Icon.svelte";
  import { connectWithPrompt } from "$lib/ssh/connectWithPrompt";
  import { relativeTime } from "$lib/ssh/hostFormat";
  import {
    fetchJobsReadout,
    type JobsReadout,
    type PersistentSession,
    type SlurmJob,
  } from "$lib/ssh/jobsSnapshot";
  import { invokeQuiet } from "$lib/ipc";
  import { confirmDialog } from "$lib/stores/confirm.svelte";
  import { terminalLaunch } from "$lib/stores/terminalLaunch.svelte";
  import type { ExecResult } from "$lib/types/sshTunnels";

  let {
    connectionId,
    label,
    active = false,
  }: { connectionId: string; label: string; active?: boolean } = $props();

  let readout = $state<JobsReadout | null>(null);
  let loading = $state(false);
  let stampedAt = $state<number | null>(null);
  let cancelling = $state<string | null>(null);

  async function refresh() {
    if (loading) return;
    loading = true;
    try {
      readout = await fetchJobsReadout(connectionId, label);
      stampedAt = Math.floor(Date.now() / 1000);
    } catch {
      /* connectWithPrompt already toasted the real failure */
    } finally {
      loading = false;
    }
  }

  // Auto-load on first open, like the GPU/Processes siblings — the session is
  // already warm from the workspace, so this adds no extra prompt. Latched so
  // toggling back keeps the snapshot until the user re-runs it.
  let autoLoaded = false;
  $effect(() => {
    if (active && !autoLoaded) {
      autoLoaded = true;
      void refresh();
    }
  });

  // Attach is interactive (a full-screen TUI), so it can't run over the snapshot
  // exec layer — hand it to the Terminal panel as a new pty-backed tab.
  function attach(s: PersistentSession) {
    const command =
      s.kind === "tmux" ? `tmux attach -t ${shq(s.target)}` : `screen -r ${shq(s.target)}`;
    terminalLaunch.launch(connectionId, command, `${s.kind}: ${s.label}`);
  }

  // "New persistent session": wrap a shell in tmux (attach-or-create) so a
  // dropped SSH leaves the run alive to re-attach later.
  function newSession() {
    const name = nextSessionName();
    terminalLaunch.launch(connectionId, `tmux new-session -A -s ${shq(name)}`, `tmux: ${name}`);
  }

  // A fresh name that won't collide with an existing tmux session this snapshot.
  function nextSessionName(): string {
    const taken = new Set(
      (readout?.sessions ?? []).filter((s) => s.kind === "tmux").map((s) => s.label),
    );
    if (!taken.has("work")) return "work";
    let n = 2;
    while (taken.has(`work${n}`)) n++;
    return `work${n}`;
  }

  /** Single-quote a shell argument (session names are tame, but be correct). */
  function shq(s: string): string {
    return `'${s.replace(/'/g, "'\\''")}'`;
  }

  async function killSession(s: PersistentSession) {
    const choice = await confirmDialog.open({
      title: `Kill ${s.kind} session “${s.label}”?`,
      message: `Ends the session and every process inside it on ${label}. This can't be undone.`,
      destructive: true,
      icon: "trash-2",
      actions: [{ label: "Kill session", value: "kill", tone: "destructive", icon: "trash-2" }],
    });
    if (choice !== "kill") return;
    const command =
      s.kind === "tmux"
        ? `tmux kill-session -t ${shq(s.target)}`
        : `screen -S ${shq(s.target)} -X quit`;
    await runAction(`session:${s.target}`, command);
  }

  async function cancelJob(j: SlurmJob) {
    const choice = await confirmDialog.open({
      title: `Cancel job ${j.id}?`,
      message: `${j.name}\n\nSends scancel to SLURM job ${j.id} on ${label}.`,
      destructive: true,
      icon: "circle-stop",
      actions: [{ label: "Cancel job", value: "cancel", tone: "destructive", icon: "circle-stop" }],
    });
    if (choice !== "cancel") return;
    await runAction(`job:${j.id}`, `scancel ${shq(j.id)}`);
  }

  // Run a one-shot mutating command over the warm exec session, then re-snapshot.
  async function runAction(key: string, command: string) {
    cancelling = key;
    try {
      await connectWithPrompt(connectionId, label, (cred) =>
        invokeQuiet<ExecResult>("ssh_exec_run", {
          input: {
            connectionId,
            command,
            password: cred?.kind === "password" ? cred.secret : undefined,
            passphrase: cred?.kind === "passphrase" ? cred.secret : undefined,
          },
        }),
      );
      await refresh();
    } catch {
      /* toasted */
    } finally {
      cancelling = null;
    }
  }

  // Running/active states read calm-green; pending amber; failures red; the rest
  // (completed/cancelled/timeout history) get muted, since they're just records.
  function stateTone(state: string): string {
    const s = state.toUpperCase();
    if (s === "RUNNING" || s === "COMPLETING" || s === "CONFIGURING") return "text-status-running";
    if (s === "PENDING" || s === "REQUEUED" || s === "SUSPENDED") return "text-status-unhealthy";
    if (s === "FAILED" || s === "NODE_FAIL" || s === "OUT_OF_MEMORY") return "text-status-crashed";
    if (s === "COMPLETED") return "text-status-running";
    return "text-fg-subtle";
  }
  function stateDot(state: string): string {
    const s = state.toUpperCase();
    if (s === "RUNNING" || s === "COMPLETING" || s === "CONFIGURING") return "bg-status-running";
    if (s === "PENDING" || s === "REQUEUED" || s === "SUSPENDED") return "bg-status-unhealthy";
    if (s === "FAILED" || s === "NODE_FAIL" || s === "OUT_OF_MEMORY") return "bg-status-crashed";
    return "bg-status-stopped";
  }

  const sessions = $derived(readout?.sessions ?? []);
  const jobs = $derived(readout?.jobs ?? []);
  const history = $derived(readout?.history ?? []);

  const subtitle = $derived(
    !readout
      ? "tmux / screen / SLURM — point-in-time, not live"
      : [
          stampedAt ? `Snapshot · ${relativeTime(stampedAt)}` : null,
          `${sessions.length} ${sessions.length === 1 ? "session" : "sessions"}`,
          readout.hasSlurm ? `${jobs.length} active ${jobs.length === 1 ? "job" : "jobs"}` : null,
        ]
          .filter(Boolean)
          .join(" · "),
  );
</script>

<div class="flex h-full min-h-0 flex-col">
  <header class="flex items-center gap-2 border-b border-border/60 px-6 py-3">
    <Icon name="layers" size={15} class="text-fg-muted" />
    <div class="min-w-0 flex-1">
      <h2 class="text-[13px] font-semibold text-fg">Jobs</h2>
      <p class="truncate text-[11px] text-fg-subtle">{subtitle}</p>
    </div>
    <button
      type="button"
      onclick={refresh}
      disabled={loading}
      class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md text-[12px] font-medium border border-border text-fg-muted hover:text-fg hover:bg-surface-2 disabled:opacity-50"
    >
      <Icon name="refresh-cw" size={12} class={loading ? "animate-spin" : ""} />
      {readout ? "Refresh" : "Load"}
    </button>
  </header>

  <div class="min-h-0 flex-1 overflow-y-auto">
    {#if !readout}
      <div class="flex h-full items-center justify-center">
        <button
          type="button"
          onclick={refresh}
          disabled={loading}
          class="inline-flex items-center gap-2 rounded-lg border border-border px-3.5 py-2 text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg disabled:opacity-50"
        >
          <Icon name={loading ? "refresh-cw" : "layers"} size={14} class={loading ? "animate-spin" : ""} />
          {loading ? "Reading sessions & jobs…" : "Load sessions & jobs"}
        </button>
      </div>
    {:else}
      <div class="space-y-5 p-4">
        <!-- ── Persistent sessions (tmux / screen) ────────────────────────── -->
        <section>
          <div class="mb-2 flex items-center gap-2">
            <Icon name="terminal" size={13} class="text-fg-subtle" />
            <h3 class="text-[11.5px] font-semibold uppercase tracking-wide text-fg-muted">
              Persistent sessions
            </h3>
            <span class="text-[11px] tabular-nums text-fg-subtle">{sessions.length}</span>
            <button
              type="button"
              onclick={newSession}
              class="ml-auto inline-flex items-center gap-1.5 rounded-md border border-border px-2 py-1 text-[11.5px] font-medium text-fg-muted hover:bg-surface-2 hover:text-fg"
              title="Start a new tmux session (survives a dropped SSH)"
            >
              <Icon name="plus" size={12} /> New session
            </button>
          </div>

          {#if sessions.length === 0}
            <div class="rounded-xl border border-dashed border-border/70 bg-surface-2/30 px-4 py-5 text-center">
              <p class="text-[12.5px] font-medium text-fg-muted">No tmux or screen sessions</p>
              <p class="mx-auto mt-1 max-w-sm text-[11.5px] leading-relaxed text-fg-subtle">
                Start a persistent session to keep a training run alive after the SSH connection
                drops — then re-attach from here next time.
              </p>
            </div>
          {:else}
            <ul class="space-y-1.5">
              {#each sessions as s (s.kind + ":" + s.target)}
                <li class="group flex items-center gap-3 rounded-lg border border-border/70 bg-surface px-3 py-2">
                  <span
                    class="h-1.5 w-1.5 shrink-0 rounded-full {s.attached ? 'bg-status-running' : 'bg-status-stopped'}"
                    title={s.attached ? "A client is attached" : "Detached — running in the background"}
                  ></span>
                  <div class="min-w-0 flex-1">
                    <div class="flex items-center gap-2">
                      <span class="truncate font-mono text-[12.5px] text-fg" title={s.target}>{s.label}</span>
                      <span class="shrink-0 rounded bg-surface-2 px-1.5 py-px text-[10px] font-medium text-fg-subtle">
                        {s.kind}
                      </span>
                    </div>
                    <div class="mt-0.5 flex items-center gap-2 text-[11px] text-fg-subtle">
                      <span>{s.attached ? "attached" : "detached"}</span>
                      {#if s.windows != null}
                        <span class="tabular-nums">· {s.windows} {s.windows === 1 ? "window" : "windows"}</span>
                      {/if}
                    </div>
                  </div>
                  <button
                    type="button"
                    onclick={() => killSession(s)}
                    disabled={cancelling === `session:${s.target}`}
                    class="inline-flex items-center rounded-md p-1.5 text-fg-subtle opacity-0 hover:bg-status-crashed/10 hover:text-status-crashed focus:opacity-100 group-hover:opacity-100 disabled:opacity-50"
                    aria-label="Kill session"
                    title="Kill this session"
                  >
                    <Icon name={cancelling === `session:${s.target}` ? "refresh-cw" : "trash-2"} size={13} class={cancelling === `session:${s.target}` ? "animate-spin" : ""} />
                  </button>
                  <button
                    type="button"
                    onclick={() => attach(s)}
                    class="inline-flex items-center gap-1.5 rounded-md border border-border px-2.5 py-1 text-[11.5px] font-medium text-fg-muted hover:bg-surface-2 hover:text-fg"
                    title="Attach in a new terminal tab"
                  >
                    <Icon name="external-link" size={12} /> Attach
                  </button>
                </li>
              {/each}
            </ul>
          {/if}
        </section>

        <!-- ── SLURM (only when a scheduler is present) ───────────────────── -->
        {#if readout.hasSlurm}
          <section>
            <div class="mb-2 flex items-center gap-2">
              <Icon name="list" size={13} class="text-fg-subtle" />
              <h3 class="text-[11.5px] font-semibold uppercase tracking-wide text-fg-muted">
                SLURM jobs
              </h3>
              <span class="text-[11px] tabular-nums text-fg-subtle">{jobs.length} active</span>
            </div>

            {#if jobs.length === 0}
              <div class="rounded-xl border border-dashed border-border/70 bg-surface-2/30 px-4 py-4 text-center text-[12px] text-fg-subtle">
                No active jobs in the queue.
              </div>
            {:else}
              <ul class="space-y-1.5">
                {#each jobs as j (j.id)}
                  <li class="group flex items-center gap-3 rounded-lg border border-border/70 bg-surface px-3 py-2">
                    <span class="h-1.5 w-1.5 shrink-0 rounded-full {stateDot(j.state)}"></span>
                    <span class="shrink-0 font-mono text-[11.5px] tabular-nums text-fg-muted">{j.id}</span>
                    <div class="min-w-0 flex-1">
                      <div class="flex items-center gap-2">
                        <span class="truncate text-[12.5px] text-fg" title={j.name}>{j.name}</span>
                        <span class="shrink-0 text-[10.5px] font-semibold uppercase tracking-wide {stateTone(j.state)}">{j.state}</span>
                      </div>
                      <div class="mt-0.5 flex flex-wrap items-center gap-x-2 gap-y-0.5 text-[11px] text-fg-subtle">
                        <span class="font-mono tabular-nums">{j.elapsed}</span>
                        {#if j.where}<span class="truncate font-mono" title={j.where}>· {j.where}</span>{/if}
                        {#if j.partition}<span>· {j.partition}</span>{/if}
                        {#if j.nodes}<span class="tabular-nums">· {j.nodes} {j.nodes === "1" ? "node" : "nodes"}</span>{/if}
                      </div>
                    </div>
                    <button
                      type="button"
                      onclick={() => cancelJob(j)}
                      disabled={cancelling === `job:${j.id}`}
                      class="inline-flex items-center gap-1.5 rounded-md border border-border px-2.5 py-1 text-[11.5px] font-medium text-fg-subtle hover:border-status-crashed/40 hover:bg-status-crashed/10 hover:text-status-crashed disabled:opacity-50"
                      title="Cancel this job (scancel)"
                    >
                      <Icon name={cancelling === `job:${j.id}` ? "refresh-cw" : "circle-stop"} size={12} class={cancelling === `job:${j.id}` ? "animate-spin" : ""} />
                      Cancel
                    </button>
                  </li>
                {/each}
              </ul>
            {/if}

            {#if history.length}
              <div class="mt-3">
                <p class="mb-1.5 text-[10.5px] font-semibold uppercase tracking-wide text-fg-subtle">
                  Recent history
                </p>
                <ul class="space-y-0.5">
                  {#each history as h (h.id)}
                    <li class="flex items-center gap-2.5 rounded-md px-2 py-1 text-[11.5px] hover:bg-surface-2/50">
                      <span class="h-1 w-1 shrink-0 rounded-full {stateDot(h.state)}"></span>
                      <span class="shrink-0 font-mono tabular-nums text-fg-subtle">{h.id}</span>
                      <span class="min-w-0 flex-1 truncate text-fg-muted" title={h.name}>{h.name}</span>
                      <span class="shrink-0 font-semibold uppercase tracking-wide {stateTone(h.state)}">{h.state}</span>
                      <span class="shrink-0 font-mono tabular-nums text-fg-subtle">{h.elapsed}</span>
                    </li>
                  {/each}
                </ul>
              </div>
            {/if}
          </section>
        {/if}
      </div>
    {/if}
  </div>
</div>
