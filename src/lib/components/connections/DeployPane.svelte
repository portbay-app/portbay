<!--
  DeployPane — the run/deploy body: an ordered list of commands + a working
  directory, a Run button, and each step's captured output + exit code. The
  backend stops at the first failing step (so a failed `npm ci` won't reach the
  build).

  This is the presentational core shared by two hosts: the modal wrapper
  (DeployPanel.svelte, opened from the deployPanel store) and the SSH host
  workspace's Deploy tab. It fills its container (`h-full`) and renders its own
  header; the optional `onClose` adds a close button (the modal passes it, the
  embedded tab does not). `busy` is bindable so a modal wrapper can refuse to
  close mid-run.

  Showing the exact commands before the explicit Run click IS the approval step
  for executing on a remote host.
-->
<script lang="ts">
  import { clampToViewport } from "$lib/actions/clampToViewport";
  import Icon from "$lib/components/atoms/Icon.svelte";
  import ProjectDeploySection from "$lib/components/projects/ProjectDeploySection.svelte";
  import DeployStepsView from "$lib/components/deploy/DeployStepsView.svelte";
  import { cancelDeploy, listenDeploy } from "$lib/deploy";
  import {
    applyDeployEvent,
    finalizeError,
    formatDuration,
    initLiveSteps,
    reconcileResults,
    summarize,
    type LiveStep,
  } from "$lib/deployLive";
  import { invokeQuiet } from "$lib/ipc";
  import { connectWithPrompt } from "$lib/ssh/connectWithPrompt";
  import { deploySnippets, type DeploySnippet } from "$lib/stores/deploySnippets.svelte";
  import type { StepResult } from "$lib/types/sshTunnels";

  let {
    connectionId,
    label,
    onClose,
    busy = $bindable(false),
    projectId = null,
  }: {
    connectionId: string;
    label: string;
    onClose?: () => void;
    busy?: boolean;
    /** When set, deploy this project to the current host (config + sync + steps)
        instead of running an ad-hoc command list. */
    projectId?: string | null;
  } = $props();

  let cwd = $state("");
  let steps = $state<string[]>(["npm ci", "npm run build"]);
  let running = $state(false);
  // Live model of the current/last run: seeded on Run, fed by streamed
  // events, settled against the returned results. Survives until Clear or
  // the next Run so the user can read back the output.
  let live = $state<LiveStep[]>([]);
  let runId = $state<string | null>(null);
  let cancelling = $state(false);

  // Mirror the in-flight flag onto the bindable prop so a modal wrapper can gate
  // its Escape / backdrop close while a deploy is mid-run.
  $effect(() => {
    busy = running;
  });

  const allEmpty = $derived(steps.every((s) => s.trim() === ""));

  // Saved snippets (deploy macros) for this connection. Derived off the store,
  // so add/remove reflect immediately without a manual refresh.
  const snippets = $derived(deploySnippets.list(connectionId));
  let snippetsOpen = $state(false);
  // Viewport anchor for the snippets dropdown: it renders `fixed` (escaping
  // the pane's scroll-container clipping) right-aligned under the trigger,
  // and clampToViewport nudges it fully on-screen in narrow layouts.
  let snippetsAt = $state<{ x: number; y: number } | null>(null);
  function toggleSnippets(ev: MouseEvent) {
    const r = (ev.currentTarget as HTMLElement).getBoundingClientRect();
    snippetsAt = { x: r.right, y: r.bottom + 4 };
    snippetsOpen = !snippetsOpen;
  }
  let saving = $state(false);
  let saveName = $state("");

  function applySnippet(s: DeploySnippet) {
    cwd = s.cwd;
    steps = s.steps.length ? [...s.steps] : [""];
    snippetsOpen = false;
  }
  function saveSnippet() {
    const name = saveName.trim();
    const kept = steps.map((s) => s.trim()).filter(Boolean);
    if (!name || kept.length === 0) return;
    deploySnippets.add(connectionId, name, cwd.trim(), kept);
    saveName = "";
    saving = false;
  }
  function removeSnippet(id: string) {
    deploySnippets.remove(connectionId, id);
  }

  /** Cmd/Ctrl+Enter from the cwd or any command input starts the run. */
  function runShortcut(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
      e.preventDefault();
      void run();
    }
  }

  function addStep() {
    steps = [...steps, ""];
  }
  function removeStep(i: number) {
    steps = steps.filter((_, idx) => idx !== i);
    if (steps.length === 0) steps = [""];
  }

  async function run() {
    if (allEmpty || running) return;
    const kept = steps.map((s) => s.trim()).filter(Boolean);
    running = true;
    cancelling = false;
    const id = crypto.randomUUID();
    runId = id;
    live = initLiveSteps(kept);
    // Mutate through the $state proxy (reading `live` back), not the raw
    // array — otherwise the streamed updates wouldn't be reactive.
    const liveSteps = live;
    const unlisten = await listenDeploy(id, (ev) => applyDeployEvent(liveSteps, ev));
    try {
      // Prompt once (VS Code-style) for a one-shot credential if the host needs
      // one; the secret is passed inline for this run and never stored.
      const results = await connectWithPrompt(connectionId, label, (cred) =>
        invokeQuiet<StepResult[]>("ssh_deploy_run", {
          input: {
            connectionId,
            steps: kept,
            cwd: cwd.trim() || null,
            runId: id,
            password: cred?.kind === "password" ? cred.secret : undefined,
            passphrase: cred?.kind === "passphrase" ? cred.secret : undefined,
          },
        }),
      );
      reconcileResults(liveSteps, results, cancelling);
    } catch {
      // connectWithPrompt already surfaced the real failure; if the run died
      // before any step started there's nothing worth keeping on screen.
      if (!finalizeError(liveSteps)) live = [];
    } finally {
      unlisten();
      running = false;
      cancelling = false;
      runId = null;
    }
  }

  async function cancel() {
    if (!runId || cancelling) return;
    cancelling = true;
    try {
      await cancelDeploy(runId);
    } catch {
      cancelling = false; // toasted; the run keeps going
    }
  }

  const summary = $derived(summarize(live));
  const runningAt = $derived(live.findIndex((s) => s.status === "running"));
</script>

{#if projectId}
  <!-- Project deploy: sync the project's files to this host + run its steps. -->
  <ProjectDeploySection {projectId} lockedConnectionId={connectionId} {onClose} />
{:else}
<div class="flex h-full min-h-0 flex-col">
  <header class="flex items-center gap-2 border-b border-border px-4 py-3">
    <Icon name="terminal" size={15} class="text-fg-muted" />
    <div class="min-w-0 flex-1">
      <h2 class="truncate text-[13px] font-semibold text-fg">Run / Deploy · {label}</h2>
      <p class="truncate text-[11px] text-fg-subtle">Commands run in order; stops on the first failure.</p>
    </div>
    {#if onClose}
      <button
        type="button"
        onclick={onClose}
        disabled={running}
        class="rounded-md p-1.5 text-fg-muted hover:bg-surface-2 hover:text-fg disabled:opacity-40"
        aria-label="Close"
      >
        <Icon name="x" size={16} />
      </button>
    {/if}
  </header>

  <div class="min-h-0 flex-1 overflow-y-auto p-4">
    <!-- Working directory -->
    <label class="block text-[11px] font-medium uppercase text-fg-subtle" for="deploy-cwd">
      Working directory (optional)
    </label>
    <input
      id="deploy-cwd"
      bind:value={cwd}
      onkeydown={runShortcut}
      placeholder="/var/www/myapp"
      class="mt-1 w-full rounded-md border border-border bg-surface-2 px-2 py-1.5 font-mono text-[12px] text-fg outline-none focus:border-accent"
    />

    <!-- Steps -->
    <div class="mt-4 flex items-center justify-between gap-2">
      <span class="text-[11px] font-medium uppercase text-fg-subtle">Commands</span>
      <div class="flex items-center gap-1">
        <!-- Saved snippets picker -->
        <div class="relative">
          <button
            type="button"
            onclick={toggleSnippets}
            class="inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-[11px] text-fg-muted hover:bg-surface-2 hover:text-fg"
          >
            <Icon name="square-kanban" size={11} /> Snippets
            {#if snippets.length}<span class="text-fg-subtle">({snippets.length})</span>{/if}
            <Icon name="chevron-down" size={10} />
          </button>
          {#if snippetsOpen && snippetsAt}
            <button type="button" class="fixed inset-0 z-40 cursor-default" aria-label="Close" onclick={() => (snippetsOpen = false)}></button>
            <div
              use:clampToViewport
              class="fixed z-50 w-64 overflow-hidden rounded-lg border border-border bg-surface shadow-2xl backdrop-blur-xl"
              style="left: {snippetsAt.x - 256}px; top: {snippetsAt.y}px"
            >
              {#if snippets.length === 0}
                <p class="px-3 py-2.5 text-[11.5px] text-fg-subtle">No saved snippets yet. Build a command list, then “Save”.</p>
              {:else}
                <ul class="max-h-56 overflow-y-auto p-1">
                  {#each snippets as s (s.id)}
                    <li class="group flex items-center gap-1 rounded-md px-1.5 py-1 hover:bg-surface-2">
                      <button type="button" onclick={() => applySnippet(s)} class="min-w-0 flex-1 text-left">
                        <span class="block truncate text-[12px] text-fg">{s.name}</span>
                        <span class="block truncate font-mono text-[10.5px] text-fg-subtle">{s.steps.join(" · ")}</span>
                      </button>
                      <button type="button" onclick={() => removeSnippet(s.id)} class="shrink-0 rounded p-1 text-fg-subtle opacity-0 hover:text-status-crashed group-hover:opacity-100" aria-label="Delete snippet">
                        <Icon name="trash-2" size={12} />
                      </button>
                    </li>
                  {/each}
                </ul>
              {/if}
            </div>
          {/if}
        </div>

        <button
          type="button"
          onclick={() => { saving = true; }}
          disabled={allEmpty}
          class="inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-[11px] text-fg-muted hover:bg-surface-2 hover:text-fg disabled:opacity-40"
        >
          <Icon name="file-plus" size={11} /> Save
        </button>
        <button
          type="button"
          onclick={addStep}
          class="inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-[11px] text-fg-muted hover:bg-surface-2 hover:text-fg"
        >
          <Icon name="plus" size={11} /> Add step
        </button>
      </div>
    </div>

    {#if saving}
      <div class="mt-2 flex items-center gap-1.5 rounded-md border border-border bg-surface-2/50 px-2 py-1.5">
        <!-- svelte-ignore a11y_autofocus -->
        <input
          bind:value={saveName}
          autofocus
          placeholder="Snippet name (e.g. Build & restart)"
          onkeydown={(e) => { if (e.key === "Enter") saveSnippet(); if (e.key === "Escape") saving = false; }}
          class="h-7 flex-1 rounded-md border border-border bg-surface px-2 text-[12px] text-fg outline-none focus:border-accent"
        />
        <button type="button" onclick={saveSnippet} disabled={!saveName.trim()} class="h-7 rounded-md bg-accent px-2.5 text-[12px] font-medium text-on-accent hover:brightness-110 disabled:opacity-50">Save</button>
        <button type="button" onclick={() => { saving = false; saveName = ""; }} class="h-7 rounded-md px-2 text-[12px] text-fg-muted hover:bg-surface-2">Cancel</button>
      </div>
    {/if}
    <div class="mt-1.5 space-y-1.5">
      {#each steps as _, i (i)}
        <div class="flex items-center gap-1.5">
          <span class="w-5 shrink-0 text-right font-mono text-[11px] text-fg-subtle">{i + 1}</span>
          <input
            bind:value={steps[i]}
            onkeydown={runShortcut}
            placeholder="e.g. npm run build"
            class="flex-1 rounded-md border border-border bg-surface-2 px-2 py-1.5 font-mono text-[12px] text-fg outline-none focus:border-accent"
          />
          <button
            type="button"
            onclick={() => removeStep(i)}
            class="rounded p-1 text-fg-muted hover:bg-status-crashed/10 hover:text-status-crashed"
            aria-label="Remove step"
          >
            <Icon name="trash-2" size={13} />
          </button>
        </div>
      {/each}
    </div>

    <!-- Run output: live while running, readable until cleared or re-run. -->
    {#if live.length > 0}
      <div class="mt-5">
        <div class="mb-1.5 flex items-center justify-between">
          <span class="text-[11px] font-medium uppercase text-fg-subtle">Run output</span>
          {#if !running}
            <button
              type="button"
              onclick={() => (live = [])}
              class="inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-[11px] text-fg-muted hover:bg-surface-2 hover:text-fg"
            >
              <Icon name="eraser" size={11} /> Clear
            </button>
          {/if}
        </div>
        <DeployStepsView steps={live} />
      </div>
    {/if}
  </div>

  <footer class="flex items-center justify-between gap-2 border-t border-border px-4 py-2.5">
    <span class="truncate text-[11px] text-fg-subtle">
      {#if running}
        {#if runningAt !== -1}
          Running step {runningAt + 1} of {live.length}…
        {:else}
          Connecting…
        {/if}
      {:else if summary.allOk && summary.totalMs > 0}
        Finished in {formatDuration(summary.totalMs)}.
      {/if}
    </span>
    <div class="flex items-center gap-2">
      {#if running}
        <button
          type="button"
          onclick={cancel}
          disabled={cancelling}
          class="inline-flex h-8 items-center gap-1.5 rounded-md border border-status-crashed/40 px-3 text-[12px] font-medium text-status-crashed hover:bg-status-crashed/10 disabled:opacity-50"
        >
          <Icon name="circle-stop" size={13} />
          {cancelling ? "Cancelling…" : "Cancel"}
        </button>
      {/if}
      <button
        type="button"
        onclick={run}
        disabled={running || allEmpty}
        class="inline-flex h-8 items-center gap-1.5 rounded-md bg-accent px-3 text-[12px] font-medium text-on-accent hover:brightness-110 disabled:opacity-50"
      >
        <Icon name={running ? "refresh-cw" : "play"} size={13} class={running ? "animate-spin" : ""} />
        {running ? "Running…" : "Run"}
      </button>
    </div>
  </footer>
</div>
{/if}
