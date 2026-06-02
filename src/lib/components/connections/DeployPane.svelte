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
  import Icon from "$lib/components/atoms/Icon.svelte";
  import ProjectDeploySection from "$lib/components/projects/ProjectDeploySection.svelte";
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
  let results = $state<StepResult[]>([]);

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

  function addStep() {
    steps = [...steps, ""];
  }
  function removeStep(i: number) {
    steps = steps.filter((_, idx) => idx !== i);
    if (steps.length === 0) steps = [""];
  }

  async function run() {
    if (allEmpty || running) return;
    running = true;
    results = [];
    try {
      // Prompt once (VS Code-style) for a one-shot credential if the host needs
      // one; the secret is passed inline for this run and never stored.
      results = await connectWithPrompt(connectionId, label, (cred) =>
        invokeQuiet<StepResult[]>("ssh_deploy_run", {
          input: {
            connectionId,
            steps,
            cwd: cwd.trim() || null,
            password: cred?.kind === "password" ? cred.secret : undefined,
            passphrase: cred?.kind === "passphrase" ? cred.secret : undefined,
          },
        }),
      );
    } catch {
      /* connectWithPrompt already surfaced any real failure */
    } finally {
      running = false;
    }
  }

  const failedAt = $derived(results.findIndex((r) => r.exitCode !== 0));
  const succeeded = $derived(results.length > 0 && failedAt === -1);
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
            onclick={() => (snippetsOpen = !snippetsOpen)}
            class="inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-[11px] text-fg-muted hover:bg-surface-2 hover:text-fg"
          >
            <Icon name="square-kanban" size={11} /> Snippets
            {#if snippets.length}<span class="text-fg-subtle">({snippets.length})</span>{/if}
            <Icon name="chevron-down" size={10} />
          </button>
          {#if snippetsOpen}
            <button type="button" class="fixed inset-0 z-40 cursor-default" aria-label="Close" onclick={() => (snippetsOpen = false)}></button>
            <div class="absolute right-0 z-50 mt-1 w-64 overflow-hidden rounded-lg border border-border bg-surface shadow-2xl">
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

    <!-- Results -->
    {#if results.length > 0}
      <div class="mt-5 space-y-2">
        {#each results as r, i (i)}
          <div class="overflow-hidden rounded-md border border-border/70">
            <div
              class="flex items-center gap-2 px-3 py-1.5 text-[12px]
                     {r.exitCode === 0 ? 'bg-status-running/10' : 'bg-status-crashed/10'}"
            >
              <Icon
                name={r.exitCode === 0 ? "circle-check" : "circle-alert"}
                size={13}
                class={r.exitCode === 0 ? "text-status-running" : "text-status-crashed"}
              />
              <code class="flex-1 truncate font-mono text-fg">{r.command}</code>
              <span class="font-mono text-[11px] text-fg-subtle">exit {r.exitCode}</span>
            </div>
            {#if r.stdout || r.stderr}
              <pre class="max-h-48 overflow-auto bg-surface-2/50 px-3 py-2 font-mono text-[11px] leading-relaxed text-fg">{r.stdout}{#if r.stderr}<span class="text-status-crashed">{r.stderr}</span>{/if}</pre>
            {/if}
          </div>
        {/each}
        {#if succeeded}
          <p class="text-[12px] font-medium text-status-running">All steps succeeded.</p>
        {:else if failedAt !== -1}
          <p class="text-[12px] font-medium text-status-crashed">
            Stopped at step {failedAt + 1} (non-zero exit). Later steps were skipped.
          </p>
        {/if}
      </div>
    {/if}
  </div>

  <footer class="flex items-center justify-end gap-2 border-t border-border px-4 py-2.5">
    <button
      type="button"
      onclick={run}
      disabled={running || allEmpty}
      class="inline-flex h-8 items-center gap-1.5 rounded-md bg-accent px-3 text-[12px] font-medium text-on-accent hover:brightness-110 disabled:opacity-50"
    >
      <Icon name={running ? "refresh-cw" : "play"} size={13} class={running ? "animate-spin" : ""} />
      {running ? "Running…" : "Run"}
    </button>
  </footer>
</div>
{/if}
