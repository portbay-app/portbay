<!--
  ProjectDeploySection — configure + run a project's one-click deploy: pick a
  saved SSH host, a remote path, an optional local sub-directory to sync, and an
  ordered list of build/release steps. Save persists the config on the project;
  Run syncs the files then runs the steps, showing an upload summary and each
  step's output.

  Reused in two places: the project detail panel (full host picker) and the host
  workspace's Deploy view (`lockedConnectionId` fixes the host to the one you're
  inside, hiding the picker).
-->
<script lang="ts">
  import Icon from "$lib/components/atoms/Icon.svelte";
  import { formatSize } from "$lib/sftp";
  import {
    projectGetDeploy,
    projectSetDeploy,
    projectDeployRun,
  } from "$lib/deploy";
  import { sshConnections } from "$lib/stores/sshConnections.svelte";
  import { defaultProjectDeploy, type DeployRunResult, type ProjectDeploy } from "$lib/types/projects";

  interface Props {
    projectId: string;
    /** Fixes the deploy host (workspace context); hides the host picker. */
    lockedConnectionId?: string | null;
    onClose?: () => void;
    /** Detail-panel mode: drop the internal header + fill-height/scroll so the
        section flows inside its DashboardCard instead of owning a pane. */
    embedded?: boolean;
  }
  let { projectId, lockedConnectionId = null, onClose, embedded = false }: Props = $props();

  // Seeded empty; the load $effect below always re-seeds from the saved config
  // (or a fresh default) and applies `lockedConnectionId` there, so the
  // initializer doesn't need to read the prop reactively.
  let cfg = $state<ProjectDeploy>(defaultProjectDeploy(""));
  let loaded = $state(false);
  let dirty = $state(false);
  let saving = $state(false);
  let running = $state(false);
  let result = $state<DeployRunResult | null>(null);
  let excludeText = $state("node_modules, .git");

  const hosts = $derived(sshConnections.value);
  const selectedHost = $derived(hosts.find((h) => h.id === cfg.connectionId) ?? null);
  const hostLabel = $derived(selectedHost ? `${selectedHost.sshHost}:${selectedHost.sshPort}` : "");

  const canRun = $derived(
    !!cfg.connectionId && cfg.remotePath.trim() !== "" && !running && !saving,
  );
  const syncPreview = $derived(
    `Sync ./${(cfg.localSubdir ?? "").trim() || "."} → ${cfg.remotePath.trim() || "<remote path>"}`,
  );

  function markDirty() {
    dirty = true;
  }

  $effect(() => {
    if (!sshConnections.loaded) void sshConnections.refresh();
  });

  // Load the saved config once.
  $effect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const saved = await projectGetDeploy(projectId);
        if (cancelled) return;
        if (saved) {
          cfg = {
            ...saved,
            // Inside a workspace the host is fixed to the one you're in.
            connectionId: lockedConnectionId ?? saved.connectionId,
          };
          excludeText = saved.exclude.join(", ");
        } else {
          cfg = defaultProjectDeploy(lockedConnectionId ?? "");
          excludeText = cfg.exclude.join(", ");
        }
      } catch {
        /* toasted */
      } finally {
        if (!cancelled) loaded = true;
      }
    })();
    return () => {
      cancelled = true;
    };
  });

  function addStep() {
    cfg.steps = [...cfg.steps, ""];
    markDirty();
  }
  function removeStep(i: number) {
    cfg.steps = cfg.steps.filter((_, idx) => idx !== i);
    markDirty();
  }

  function normalised(): ProjectDeploy {
    return {
      connectionId: lockedConnectionId ?? cfg.connectionId,
      remotePath: cfg.remotePath.trim(),
      localSubdir: (cfg.localSubdir ?? "").trim() || null,
      steps: cfg.steps.map((s) => s.trim()).filter((s) => s !== ""),
      exclude: excludeText
        .split(/[,\n]/)
        .map((s) => s.trim())
        .filter((s) => s !== ""),
    };
  }

  async function save(): Promise<boolean> {
    saving = true;
    try {
      await projectSetDeploy(projectId, normalised());
      dirty = false;
      return true;
    } catch {
      return false; // toasted
    } finally {
      saving = false;
    }
  }

  async function run() {
    if (!canRun) return;
    // Persist any pending edits first so the backend deploys what's on screen.
    if (dirty || !loaded) {
      const ok = await save();
      if (!ok) return;
    }
    running = true;
    result = null;
    try {
      result = await projectDeployRun(projectId, cfg.connectionId, hostLabel);
    } catch {
      /* connectWithPrompt already surfaced any real failure */
    } finally {
      running = false;
    }
  }

  const failedAt = $derived(result ? result.steps.findIndex((r) => r.exitCode !== 0) : -1);
  const stepsOk = $derived(!!result && result.steps.length > 0 && failedAt === -1);
</script>

<div class={embedded ? "flex flex-col" : "flex h-full min-h-0 flex-col"}>
  {#if !embedded}
    <header class="flex items-center gap-2 border-b border-border px-4 py-3">
      <Icon name="rocket" size={15} class="text-fg-muted" />
      <div class="min-w-0 flex-1">
        <h2 class="truncate text-[13px] font-semibold text-fg">Deploy</h2>
        <p class="truncate text-[11px] text-fg-subtle">Sync files to a host, then run build steps.</p>
      </div>
      {#if onClose}
        <button type="button" onclick={onClose} disabled={running} class="rounded-md p-1.5 text-fg-muted hover:bg-surface-2 hover:text-fg disabled:opacity-40" aria-label="Close">
          <Icon name="x" size={16} />
        </button>
      {/if}
    </header>
  {/if}

  <div class={embedded ? "p-4" : "min-h-0 flex-1 overflow-y-auto p-4"}>
    {#if !loaded}
      <p class="py-6 text-center text-[12px] text-fg-subtle">Loading…</p>
    {:else}
      <!-- Host -->
      {#if !lockedConnectionId}
        <label class="block text-[11px] font-medium uppercase text-fg-subtle" for="deploy-host">Host</label>
        <select
          id="deploy-host"
          bind:value={cfg.connectionId}
          onchange={markDirty}
          class="mt-1 w-full rounded-md border border-border bg-surface-2 px-2 py-1.5 text-[12px] text-fg outline-none focus:border-accent"
        >
          <option value="" disabled>Select a saved host…</option>
          {#each hosts as h (h.id)}
            <option value={h.id}>{h.name} — {h.sshHost}:{h.sshPort}</option>
          {/each}
        </select>
        {#if hosts.length === 0}
          <p class="mt-1 text-[11px] text-fg-subtle">No saved SSH hosts yet — add one on the Connections page.</p>
        {/if}
      {:else if selectedHost}
        <p class="text-[11px] text-fg-subtle">
          Deploying to <span class="font-mono text-fg-muted">{selectedHost.name}</span>
        </p>
      {/if}

      <!-- Remote path -->
      <label class="mt-4 block text-[11px] font-medium uppercase text-fg-subtle" for="deploy-remote">Remote path</label>
      <input
        id="deploy-remote"
        bind:value={cfg.remotePath}
        oninput={markDirty}
        placeholder="/var/www/myapp"
        class="mt-1 w-full rounded-md border border-border bg-surface-2 px-2 py-1.5 font-mono text-[12px] text-fg outline-none focus:border-accent"
      />

      <!-- Local subdir -->
      <label class="mt-4 block text-[11px] font-medium uppercase text-fg-subtle" for="deploy-subdir">
        Local sub-directory (optional)
      </label>
      <input
        id="deploy-subdir"
        value={cfg.localSubdir ?? ""}
        oninput={(e) => {
          cfg.localSubdir = (e.currentTarget as HTMLInputElement).value;
          markDirty();
        }}
        placeholder="dist (blank = whole project)"
        class="mt-1 w-full rounded-md border border-border bg-surface-2 px-2 py-1.5 font-mono text-[12px] text-fg outline-none focus:border-accent"
      />

      <!-- Steps -->
      <div class="mt-4 flex items-center justify-between">
        <span class="text-[11px] font-medium uppercase text-fg-subtle">Steps (run after sync)</span>
        <button type="button" onclick={addStep} class="inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-[11px] text-fg-muted hover:bg-surface-2 hover:text-fg">
          <Icon name="plus" size={11} /> Add step
        </button>
      </div>
      {#if cfg.steps.length === 0}
        <p class="mt-1 text-[11px] text-fg-subtle">No steps — files are synced only.</p>
      {/if}
      <div class="mt-1.5 space-y-1.5">
        {#each cfg.steps as _, i (i)}
          <div class="flex items-center gap-1.5">
            <span class="w-5 shrink-0 text-right font-mono text-[11px] text-fg-subtle">{i + 1}</span>
            <input
              bind:value={cfg.steps[i]}
              oninput={markDirty}
              placeholder="e.g. npm ci && npm run build"
              class="flex-1 rounded-md border border-border bg-surface-2 px-2 py-1.5 font-mono text-[12px] text-fg outline-none focus:border-accent"
            />
            <button type="button" onclick={() => removeStep(i)} class="rounded p-1 text-fg-muted hover:bg-status-crashed/10 hover:text-status-crashed" aria-label="Remove step">
              <Icon name="trash-2" size={13} />
            </button>
          </div>
        {/each}
      </div>

      <!-- Exclude -->
      <label class="mt-4 block text-[11px] font-medium uppercase text-fg-subtle" for="deploy-exclude">
        Exclude (comma-separated)
      </label>
      <input
        id="deploy-exclude"
        bind:value={excludeText}
        oninput={markDirty}
        placeholder="node_modules, .git"
        class="mt-1 w-full rounded-md border border-border bg-surface-2 px-2 py-1.5 font-mono text-[12px] text-fg outline-none focus:border-accent"
      />

      <p class="mt-4 truncate rounded-md bg-surface-2/50 px-2.5 py-1.5 font-mono text-[11.5px] text-fg-muted" title={syncPreview}>
        {syncPreview}
      </p>

      <!-- Results -->
      {#if result}
        <div class="mt-4 space-y-2">
          <p class="text-[12px] text-fg-muted">
            Uploaded <span class="font-medium text-fg">{result.uploaded}</span>
            file{result.uploaded === 1 ? "" : "s"} ({formatSize(result.bytes)}) to
            <span class="font-mono">{result.remotePath}</span>.
          </p>
          {#if result.skipped.length > 0}
            <p class="text-[11px] text-status-unhealthy">
              Skipped {result.skipped.length} file{result.skipped.length === 1 ? "" : "s"} over the 1 GiB limit.
            </p>
          {/if}
          {#each result.steps as r, i (i)}
            <div class="overflow-hidden rounded-md border border-border/70">
              <div class="flex items-center gap-2 px-3 py-1.5 text-[12px] {r.exitCode === 0 ? 'bg-status-running/10' : 'bg-status-crashed/10'}">
                <Icon name={r.exitCode === 0 ? "circle-check" : "circle-alert"} size={13} class={r.exitCode === 0 ? "text-status-running" : "text-status-crashed"} />
                <code class="flex-1 truncate font-mono text-fg">{r.command}</code>
                <span class="font-mono text-[11px] text-fg-subtle">exit {r.exitCode}</span>
              </div>
              {#if r.stdout || r.stderr}
                <pre class="max-h-48 overflow-auto bg-surface-2/50 px-3 py-2 font-mono text-[11px] leading-relaxed text-fg">{r.stdout}{#if r.stderr}<span class="text-status-crashed">{r.stderr}</span>{/if}</pre>
              {/if}
            </div>
          {/each}
          {#if stepsOk}
            <p class="text-[12px] font-medium text-status-running">All steps succeeded.</p>
          {:else if failedAt !== -1}
            <p class="text-[12px] font-medium text-status-crashed">
              Stopped at step {failedAt + 1} (non-zero exit). Later steps were skipped.
            </p>
          {/if}
        </div>
      {/if}
    {/if}
  </div>

  <footer class="flex items-center justify-end gap-2 border-t border-border px-4 py-2.5">
    <button
      type="button"
      onclick={save}
      disabled={saving || running || !loaded}
      class="inline-flex h-8 items-center gap-1.5 rounded-md border border-border px-3 text-[12px] font-medium text-fg-muted hover:bg-surface-2 hover:text-fg disabled:opacity-50"
    >
      <Icon name={saving ? "refresh-cw" : "save"} size={13} class={saving ? "animate-spin" : ""} />
      {saving ? "Saving…" : "Save"}
    </button>
    <button
      type="button"
      onclick={run}
      disabled={!canRun}
      title={canRun ? "" : "Pick a host and remote path first"}
      class="inline-flex h-8 items-center gap-1.5 rounded-md bg-accent px-3 text-[12px] font-medium text-on-accent hover:brightness-110 disabled:opacity-50"
    >
      <Icon name={running ? "refresh-cw" : "rocket"} size={13} class={running ? "animate-spin" : ""} />
      {running ? "Deploying…" : "Deploy"}
    </button>
  </footer>
</div>
