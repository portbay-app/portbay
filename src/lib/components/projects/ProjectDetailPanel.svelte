<!--
  ProjectDetailPanel — slide-over right pane with full project controls,
  recent log tail, connection info, and an inline "edit raw" L3 escape.

  Reads the selected project from the projects store (no separate fetch
  for the panel). Refreshes the log tail on open and on demand.
-->
<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { openUrl } from "$lib/security/openUrl";

  import { DashboardCard, Icon, StatusPill } from "$lib/components/atoms";
  import EnvEditor from "./EnvEditor.svelte";
  import AdvancedFields from "./AdvancedFields.svelte";
  import ProjectDbConnections from "./ProjectDbConnections.svelte";
  import ArtifactsSection from "./ArtifactsSection.svelte";
  import { ErrorEnvelope } from "$lib/components/errors";
  import { safeInvoke } from "$lib/ipc";
  import { startProject } from "$lib/actions/startProject";
  import { errorBus } from "$lib/stores/errors.svelte";
  import { projectDetailPanel } from "$lib/stores/detailPanel.svelte";
  import { logViewer } from "$lib/stores/logViewer.svelte";
  import { parseLogLine, levelClass } from "$lib/components/logs/ansi";
  import { projects } from "$lib/stores/projects.svelte";
  import { dns } from "$lib/stores/dns.svelte";
  import { entitlements } from "$lib/stores/entitlements.svelte";
  import { createCertInfo } from "$lib/stores/certInfo.svelte";
  import type { CommandError } from "$lib/types/error";
  import type {
    ProjectView,
    SandboxConfig,
    SandboxNetworkPolicy,
    WebServer,
  } from "$lib/types/projects";
  import { typeLabel, webServerLabel } from "$lib/types/projects";

  // Currently-displayed project; null while panel is closed.
  const project = $derived<ProjectView | null>(
    projectDetailPanel.id === null
      ? null
      : (projects.value.find((p) => p.id === projectDetailPanel.id) ?? null),
  );

  // Sandboxed-project tally for the community cap. Count the *other* sandboxed
  // projects (this one already being sandboxed never counts against itself),
  // matching the backend's `check_can_sandbox`. `canSandboxThis` is the gate the
  // UI uses: enabling Sandboxed Run is offered when this project is already
  // sandboxed (the button becomes "Promote") or the tier has room.
  const othersSandboxedCount = $derived(
    projects.value.filter((p) => p.sandboxed && p.id !== project?.id).length,
  );
  const canSandboxThis = $derived(
    (project?.sandboxed ?? false) || entitlements.canSandbox(othersSandboxedCount),
  );
  // Cap label for messaging (e.g. "2/2"); null while unlimited (Pro).
  const sandboxCap = $derived(entitlements.maxSandboxProjects());

  // Optimistic display status for the header pill (overlay while a Play/Stop
  // is in flight, else the real status). Falls back to "stopped" when no
  // project is selected — the pill isn't rendered in that case anyway.
  const displayStatus = $derived(
    project ? projects.displayStatusOf(project) : "stopped",
  );

  // Editable form state — initialised on open / when target project changes.
  let nameDraft = $state<string>("");
  let hostnameDraft = $state<string>("");
  let portDraft = $state<number | null>(null);
  let startCommandDraft = $state<string>("");
  let webServerDraft = $state<WebServer>("caddy");
  let httpsDraft = $state<boolean>(true);
  let autoStartDraft = $state<boolean>(false);

  let dirty = $state<boolean>(false);
  let saving = $state<boolean>(false);
  let formError = $state<CommandError | null>(null);

  let logTail = $state<string[]>([]);
  let logLoading = $state<boolean>(false);
  // Unwrap PC's JSON log envelope + tag each line with a level for colouring.
  const logTailParsed = $derived(logTail.map(parseLogLine));

  // Shared cert loader (same semantics as the detail rail).
  const cert = createCertInfo();
  const certInfo = $derived(cert.info);
  const certLoading = $derived(cert.loading);
  const certError = $derived(cert.error);
  let reissuing = $state<boolean>(false);

  let rawConfigOpen = $state<boolean>(false);
  let rawDraft = $state<string>("");
  let sandboxNetwork = $state<SandboxNetworkPolicy>("loopback_only");
  let sandboxEphemeral = $state<boolean>(true);
  let sandboxViolations = $state<string[]>([]);
  let loadingSandboxViolations = $state<boolean>(false);

  let removeArmed = $state<boolean>(false);
  let removeArmTimer: ReturnType<typeof setTimeout> | null = null;

  // Re-initialise the form whenever the open project changes.
  $effect(() => {
    const p = project;
    if (!p) return;
    untrack(() => {
      nameDraft = p.name;
      hostnameDraft = p.hostname;
      portDraft = p.port ?? null;
      startCommandDraft = p.startCommand ?? "";
      webServerDraft = p.webServer ?? "caddy";
      httpsDraft = p.https;
      autoStartDraft = p.autoStart;
      dirty = false;
      formError = null;
      rawConfigOpen = false;
      sandboxNetwork = p.sandbox?.network ?? "loopback_only";
      sandboxEphemeral = p.sandbox?.ephemeral ?? true;
      sandboxViolations = [];
      syncRawFromFields();
      void loadLogs();
      void loadCert();
    });
  });

  function syncRawFromFields() {
    if (!project) return;
    const snapshot = {
      id: project.id,
      name: nameDraft,
      path: project.path,
      type: project.type,
      hostname: hostnameDraft,
      port: portDraft ?? undefined,
      startCommand: startCommandDraft || undefined,
      webServer: project.type === "php" ? webServerDraft : undefined,
      https: httpsDraft,
      autoStart: autoStartDraft,
    };
    rawDraft = JSON.stringify(snapshot, null, 2);
  }

  function syncFieldsFromRaw() {
    if (!rawDraft.trim()) return;
    try {
      const parsed = JSON.parse(rawDraft);
      if (typeof parsed.name === "string") nameDraft = parsed.name;
      if (typeof parsed.hostname === "string") hostnameDraft = parsed.hostname;
      if (typeof parsed.port === "number") portDraft = parsed.port;
      if (typeof parsed.startCommand === "string")
        startCommandDraft = parsed.startCommand;
      if (
        parsed.webServer === "caddy" ||
        parsed.webServer === "nginx" ||
        parsed.webServer === "apache"
      )
        webServerDraft = parsed.webServer;
      if (typeof parsed.https === "boolean") httpsDraft = parsed.https;
      if (typeof parsed.autoStart === "boolean") autoStartDraft = parsed.autoStart;
      dirty = true;
      formError = null;
    } catch (e) {
      formError = {
        code: "BAD_RAW_JSON",
        whatHappened: `Raw config is not valid JSON: ${String(e)}`,
        whyItMatters: "Fix the JSON to apply, or revert via the fields above.",
        whoCausedIt: "user",
        actions: [],
      };
    }
  }

  function markDirty() {
    dirty = true;
  }

  async function save() {
    if (!project || !dirty) return;
    saving = true;
    formError = null;
    try {
      await safeInvoke<ProjectView>("update_project", {
        id: project.id,
        patch: {
          name: nameDraft,
          hostname: hostnameDraft,
          port: portDraft ?? undefined,
          startCommand: startCommandDraft.trim() ? startCommandDraft : null,
          webServer: project.type === "php" ? webServerDraft : undefined,
          https: httpsDraft,
          autoStart: autoStartDraft,
        },
      });
      await projects.refresh();
      dirty = false;
      errorBus.push({
        code: "UPDATE_OK",
        whatHappened: `${nameDraft} updated.`,
        whyItMatters: "Restart the project for changes to take effect.",
        whoCausedIt: "system",
        actions: [],
      });
    } catch (e) {
      formError = e as CommandError;
    } finally {
      saving = false;
    }
  }

  function discard() {
    if (!project) return;
    nameDraft = project.name;
    hostnameDraft = project.hostname;
    portDraft = project.port ?? null;
    startCommandDraft = project.startCommand ?? "";
    webServerDraft = project.webServer ?? "caddy";
    httpsDraft = project.https;
    autoStartDraft = project.autoStart;
    dirty = false;
    formError = null;
    syncRawFromFields();
  }

  async function loadLogs() {
    if (!project) return;
    logLoading = true;
    try {
      logTail = await safeInvoke<string[]>("tail_logs", {
        id: project.id,
        limit: 50,
      });
    } catch {
      logTail = [];
    } finally {
      logLoading = false;
    }
  }

  function loadCert() {
    return cert.load(project);
  }

  async function reissue() {
    if (!project) return;
    reissuing = true;
    try {
      await safeInvoke("reissue_cert", { id: project.id });
      // Reconcile tick issued the cert; give it a beat then refresh.
      await new Promise((r) => setTimeout(r, 400));
      await loadCert();
      errorBus.push({
        code: "REISSUE_OK",
        whatHappened: `Cert reissued for ${project.name}.`,
        whyItMatters: "Caddy reloaded the cert; refresh your browser tab.",
        whoCausedIt: "system",
        actions: [],
      });
    } catch {
      /* toast already pushed */
    } finally {
      reissuing = false;
    }
  }

  async function revealCertFolder() {
    if (!certInfo) return;
    const dir = certInfo.certificatePath.replace(/\/cert\.pem$/, "");
    try {
      await openUrl(`file://${dir}`);
    } catch {
      /* opener pushes its own toast */
    }
  }

  async function exportPortfile() {
    if (!project) return;
    try {
      const written = await safeInvoke<string>("export_portfile", { id: project.id });
      errorBus.push({
        code: "EXPORT_OK",
        whatHappened: `Wrote ${written}`,
        whyItMatters: "Commit this file to your repo so teammates get the same local setup.",
        whoCausedIt: "system",
        actions: [],
      });
    } catch {
      /* safeInvoke toast already pushed */
    }
  }


  async function run(op: "start" | "stop" | "restart") {
    if (!project) return;
    const id = project.id;
    // Optimistic flip before any await — see ProjectRow for the rationale.
    projects.beginTransition(id, op === "stop" ? "stop" : "start");
    try {
      switch (op) {
        case "start": {
          await dns.ensureReady();
          // Resolves a port conflict via confirm + force-quit.
          const r = await startProject(id, project.name);
          if (r.kind === "declined") {
            projects.failTransition(id); // nothing started — roll back
            break;
          }
          if (r.kind === "error") {
            projects.failTransition(id);
            errorBus.push(r.error);
          }
          break;
        }
        case "stop":
          await safeInvoke("stop_project", { id });
          break;
        case "restart":
          await safeInvoke("restart_project", { id });
          break;
      }
    } catch {
      projects.failTransition(id); // roll the optimistic overlay back
      /* toast already pushed */
    }
  }

  async function runSandboxed() {
    if (!project) return;
    const id = project.id;
    projects.beginTransition(id, "start");
    try {
      await dns.ensureReady();
      await safeInvoke("start_project_sandboxed", {
        id,
        options: {
          network: sandboxNetwork,
          ephemeral: sandboxEphemeral,
        } satisfies Partial<SandboxConfig>,
      });
      await projects.refresh();
    } catch {
      projects.failTransition(id);
      /* toast already pushed */
    }
  }

  async function promoteToLocal() {
    if (!project) return;
    try {
      await safeInvoke("promote_project_to_local", { id: project.id });
      await projects.refresh();
      errorBus.push({
        code: "SANDBOX_PROMOTED",
        whatHappened: `${project.name} will run locally on the next start.`,
        whyItMatters: "The sandbox wrapper was removed from this project.",
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });
    } catch {
      /* toast already pushed */
    }
  }

  async function loadSandboxViolations() {
    if (!project) return;
    loadingSandboxViolations = true;
    try {
      sandboxViolations = await safeInvoke<string[]>("sandbox_violations", {
        id: project.id,
        limit: 250,
      });
    } catch {
      sandboxViolations = [];
    } finally {
      loadingSandboxViolations = false;
    }
  }

  async function openProjectUrl() {
    if (!project) return;
    try {
      await safeInvoke("open_project", { id: project.id });
    } catch {
      /* toast already pushed */
    }
  }

  async function revealInFinder() {
    if (!project) return;
    try {
      // Use opener — opens the directory in the default file manager.
      await openUrl(`file://${project.path}`);
    } catch {
      /* opener pushes its own toast on failure */
    }
  }

  function armRemove() {
    if (removeArmed) {
      // Second click commits.
      void confirmRemove();
      return;
    }
    removeArmed = true;
    removeArmTimer = setTimeout(() => {
      removeArmed = false;
    }, 2_000);
  }

  async function confirmRemove() {
    if (!project) return;
    if (removeArmTimer !== null) clearTimeout(removeArmTimer);
    try {
      await safeInvoke("remove_project", { id: project.id });
      await projects.refresh();
      errorBus.push({
        code: "REMOVE_OK",
        whatHappened: `${project.name} removed.`,
        whyItMatters: "Registry entry, cert directory, and hosts entry were cleaned up.",
        whoCausedIt: "system",
        actions: [],
      });
      projectDetailPanel.hide();
    } catch {
      /* toast already pushed */
    }
  }

  async function copyToClipboard(text: string, label: string) {
    try {
      await navigator.clipboard.writeText(text);
      errorBus.push({
        code: "COPIED",
        whatHappened: `${label} copied.`,
        whyItMatters: "Paste anywhere.",
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });
    } catch {
      // No clipboard permission — quietly fail.
    }
  }

  function onKeydown(e: KeyboardEvent) {
    if (projectDetailPanel.id === null) return;
    if (e.key === "Escape" && !dirty) projectDetailPanel.hide();
    if ((e.metaKey || e.ctrlKey) && e.key === "s") {
      e.preventDefault();
      void save();
    }
  }

  const url = $derived(project?.url ?? "");
</script>

<svelte:window onkeydown={onKeydown} />

{#if project}
  <!-- In-layout right-side panel (not a modal overlay): the root layout renders
       this into the grid's rail column so it matches the dashboard rail and
       every other page's side-panel language. Escape + the × button close it. -->
  <aside
    class="h-full w-full bg-surface border-l border-border flex flex-col"
    aria-label="Project detail"
  >
    <!-- Header -->
    <header
      class="shrink-0 px-5 py-4 border-b border-border space-y-3"
    >
      <div class="flex items-start justify-between gap-3">
        <div class="flex items-center gap-2.5 min-w-0">
          <StatusPill status={displayStatus} />
          <h2 class="text-base font-semibold truncate">{project.name}</h2>
          {#if project.sandboxed}
            <span
              class="inline-flex items-center gap-1 px-1.5 py-0.5 rounded border
                     border-accent/40 bg-accent/10 text-[10px] text-accent"
              title="This project command is wrapped by PortBay's sandbox profile"
            >
              <Icon name="shield" size={10} /> Sandbox
            </span>
          {/if}
        </div>
        <button
          type="button"
          onclick={() => projectDetailPanel.hide()}
          title="Close"
          aria-label="Close panel"
          class="p-1.5 rounded-md text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
        >
          <Icon name="x" size={16} />
        </button>
      </div>

      <div class="flex flex-wrap items-center gap-1.5">
        <button
          type="button"
          onclick={openProjectUrl}
          class="inline-flex items-center gap-1.5 px-2.5 py-1.5 text-xs rounded-md border border-border text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
        >
          <Icon name="globe" size={12} /> Open URL
        </button>
        <button
          type="button"
          onclick={revealInFinder}
          class="inline-flex items-center gap-1.5 px-2.5 py-1.5 text-xs rounded-md border border-border text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
        >
          <Icon name="folder" size={12} /> Folder
        </button>
        <button
          type="button"
          onclick={() => run("start")}
          class="inline-flex items-center gap-1.5 px-2.5 py-1.5 text-xs rounded-md text-status-running border border-status-running/40 hover:bg-status-running/10 transition-colors"
        >
          <Icon name="play" size={12} /> Start
        </button>
        <button
          type="button"
          onclick={project.sandboxed ? promoteToLocal : runSandboxed}
          disabled={!canSandboxThis}
          title={project.sandboxed
            ? "Remove the sandbox wrapper"
            : canSandboxThis
              ? "Run this project with PortBay's macOS sandbox profile"
              : `You're using all ${sandboxCap} sandboxed projects — upgrade to Pro for unlimited`}
          class="inline-flex items-center gap-1.5 px-2.5 py-1.5 text-xs rounded-md
                 border transition-colors disabled:opacity-45 disabled:cursor-not-allowed
                 {project.sandboxed
            ? 'text-accent border-accent/40 hover:bg-accent/10'
            : 'text-fg-muted border-border hover:text-fg hover:bg-surface-2'}"
        >
          <Icon name="shield" size={12} />
          {project.sandboxed ? "Promote" : "Sandbox"}
        </button>
        <button
          type="button"
          onclick={() => run("stop")}
          class="inline-flex items-center gap-1.5 px-2.5 py-1.5 text-xs rounded-md text-status-crashed border border-status-crashed/40 hover:bg-status-crashed/10 transition-colors"
        >
          <Icon name="square" size={12} /> Stop
        </button>
        <button
          type="button"
          onclick={() => run("restart")}
          class="inline-flex items-center gap-1.5 px-2.5 py-1.5 text-xs rounded-md text-fg-muted border border-border hover:text-fg hover:bg-surface-2 transition-colors"
        >
          <Icon name="rotate-cw" size={12} /> Restart
        </button>
        <button
          type="button"
          onclick={exportPortfile}
          title="Write .portbay.json to the project folder so this setup is reproducible"
          class="inline-flex items-center gap-1.5 px-2.5 py-1.5 text-xs rounded-md text-fg-muted border border-border hover:text-fg hover:bg-surface-2 transition-colors"
        >
          <Icon name="external-link" size={12} /> Export
        </button>
        <button
          type="button"
          onclick={armRemove}
          class="ml-auto inline-flex items-center gap-1.5 px-2.5 py-1.5 text-xs rounded-md transition-colors
                 {removeArmed
            ? 'text-bg bg-status-crashed border border-status-crashed'
            : 'text-status-crashed border border-status-crashed/30 hover:bg-status-crashed/10'}"
        >
          <Icon name="x" size={12} /> {removeArmed ? "Confirm" : "Remove"}
        </button>
      </div>
    </header>

    <!-- Body -->
    <div class="flex-1 min-h-0 overflow-y-auto p-5 space-y-4">
      {#if formError}
        <ErrorEnvelope envelope={formError} tone="inline" />
      {/if}

      <!-- Sandbox -->
      <DashboardCard title="Sandbox" flush>
        {#snippet badge()}
          {#if project.sandboxed}
            <span class="text-[11px] text-accent">Active</span>
          {:else if canSandboxThis}
            <span class="text-[11px] text-fg-subtle">Ready</span>
          {:else}
            <span class="text-[11px] text-fg-subtle" title="Upgrade to Pro for unlimited sandboxed projects">
              {othersSandboxedCount}/{sandboxCap} used
            </span>
          {/if}
        {/snippet}
        <div class="space-y-3 text-xs">
          <div class="grid grid-cols-[120px,1fr] gap-x-4 gap-y-3 items-center">
            <label for="sandbox-network" class="text-fg-muted">Network</label>
            <select
              id="sandbox-network"
              bind:value={sandboxNetwork}
              disabled={!canSandboxThis || project.sandboxed}
              class="px-3 py-2 rounded-md bg-bg border border-border text-fg
                     disabled:opacity-55 focus:border-accent/60 outline-none"
            >
              <option value="loopback_only">Loopback only</option>
              <option value="outbound">Outbound</option>
              <option value="full">Full</option>
              <option value="blocked">Blocked</option>
            </select>

            <span class="text-fg-muted">Ephemeral</span>
            <label class="inline-flex items-center gap-2 text-fg">
              <input
                type="checkbox"
                bind:checked={sandboxEphemeral}
                disabled={!canSandboxThis || project.sandboxed}
                class="accent-accent"
              />
              Reset sandbox temp/cache before start
            </label>
          </div>
          <div class="flex items-center gap-2">
            <button
              type="button"
              onclick={project.sandboxed ? promoteToLocal : runSandboxed}
              disabled={!canSandboxThis}
              title={canSandboxThis
                ? undefined
                : `You're using all ${sandboxCap} sandboxed projects — upgrade to Pro for unlimited`}
              class="inline-flex items-center gap-1.5 px-2.5 py-1.5 rounded-md
                     border text-xs transition-colors disabled:opacity-45
                     {project.sandboxed
                ? 'text-accent border-accent/40 hover:bg-accent/10'
                : 'text-status-running border-status-running/40 hover:bg-status-running/10'}"
            >
              <Icon name={project.sandboxed ? "check" : "shield"} size={12} />
              {project.sandboxed ? "Promote to local" : "Run in Sandbox"}
            </button>
            {#if project.sandboxed}
              <button
                type="button"
                onclick={loadSandboxViolations}
                disabled={loadingSandboxViolations}
                class="inline-flex items-center gap-1.5 px-2.5 py-1.5 rounded-md
                       border border-border text-fg-muted hover:text-fg hover:bg-surface-2
                       disabled:opacity-50"
              >
                <Icon
                  name={loadingSandboxViolations ? "refresh-cw" : "circle-alert"}
                  size={12}
                  class={loadingSandboxViolations ? "animate-spin" : ""}
                />
                Violations
              </button>
            {/if}
          </div>
          {#if sandboxViolations.length > 0}
            <div
              class="max-h-28 overflow-auto rounded-md border border-border bg-bg
                     p-2 font-mono text-[11px] text-status-unhealthy space-y-1"
            >
              {#each sandboxViolations as line}
                <div>{line}</div>
              {/each}
            </div>
          {:else if project.sandboxed}
            <p class="text-fg-subtle">
              No sandbox violations loaded for this run.
            </p>
          {/if}
        </div>
      </DashboardCard>

      <!-- Connection -->
      <DashboardCard title="Connection" flush>
        <dl class="grid grid-cols-[100px,1fr] gap-x-4 gap-y-2 text-xs">
          <dt class="text-fg-muted">URL</dt>
          <dd class="flex items-center gap-2 min-w-0">
            <span class="text-fg font-mono truncate">{url}</span>
            <button
              type="button"
              onclick={() => copyToClipboard(url, "URL")}
              title="Copy"
              class="p-0.5 rounded text-fg-subtle hover:text-fg"
            >
              <Icon name="link" size={11} />
            </button>
          </dd>

          <dt class="text-fg-muted">Hostname</dt>
          <dd class="text-fg font-mono">{project.hostname}</dd>

          <dt class="text-fg-muted">Port</dt>
          <dd class="text-fg font-mono">{project.port ?? "—"}</dd>

          <dt class="text-fg-muted">Type</dt>
          <dd class="text-fg">{typeLabel[project.type]}</dd>

          <dt class="text-fg-muted">Path</dt>
          <dd class="flex items-center gap-2 min-w-0">
            <span class="text-fg font-mono text-[11px] truncate">{project.path}</span>
            <button
              type="button"
              onclick={() => copyToClipboard(project.path, "Path")}
              title="Copy path"
              class="p-0.5 rounded text-fg-subtle hover:text-fg"
            >
              <Icon name="link" size={11} />
            </button>
          </dd>
        </dl>
      </DashboardCard>

      <!-- Status detail -->
      {#if project.runtime}
        <DashboardCard title="Runtime" flush>
          <dl class="grid grid-cols-[100px,1fr] gap-x-4 gap-y-1.5 text-xs font-mono">
            <dt class="text-fg-muted font-sans">PID</dt>
            <dd class="text-fg">{project.runtime.pid}</dd>
            <dt class="text-fg-muted font-sans">Restarts</dt>
            <dd class="text-fg">{project.runtime.restarts}</dd>
            <dt class="text-fg-muted font-sans">Ready</dt>
            <dd class="text-fg">{project.runtime.isReady}</dd>
            <dt class="text-fg-muted font-sans">Exit code</dt>
            <dd class="text-fg">{project.runtime.exitCode}</dd>
          </dl>
        </DashboardCard>
      {/if}

      <!-- Certificates (HTTPS projects only) -->
      {#if project.https}
        <DashboardCard title="Certificates" flush>
          {#snippet badge()}
            <div class="flex items-center gap-1">
              {#if certInfo}
                <button
                  type="button"
                  onclick={revealCertFolder}
                  title="Reveal cert folder in Finder"
                  class="text-[11px] text-fg-muted hover:text-fg px-1.5 py-0.5"
                >
                  Reveal
                </button>
              {/if}
              <button
                type="button"
                onclick={reissue}
                disabled={reissuing}
                title="Reissue cert"
                class="inline-flex items-center gap-1 text-[11px] text-accent hover:text-accent-hover px-1.5 py-0.5 disabled:opacity-50"
              >
                {#if reissuing}
                  <Icon name="refresh-cw" size={10} class="animate-spin" />
                  Reissuing…
                {:else}
                  Reissue
                {/if}
              </button>
            </div>
          {/snippet}

          {#if certError}
            <p class="text-xs text-status-crashed">{certError}</p>
          {:else if certLoading && !certInfo}
            <p class="text-xs text-fg-subtle">Loading…</p>
          {:else if !certInfo}
            <p class="text-xs text-fg-subtle">
              No certificate yet. The reconciler issues one within ~30 s on
              first reconcile, or click <span class="text-accent">Reissue</span>
              to force it now.
            </p>
          {:else}
            <dl class="grid grid-cols-[100px,1fr] gap-x-4 gap-y-2 text-xs">
              <dt class="text-fg-muted">Issued</dt>
              <dd class="text-fg font-mono">{certInfo.issuedAt ?? "—"}</dd>

              <dt class="text-fg-muted">Expires</dt>
              <dd class="text-fg font-mono">
                {certInfo.expiresAt ?? "—"}
                {#if certInfo.daysUntilExpiry !== null}
                  <span
                    class={certInfo.daysUntilExpiry < 30
                      ? "ml-2 text-status-unhealthy"
                      : "ml-2 text-fg-subtle"}
                  >
                    ({certInfo.daysUntilExpiry} day{certInfo.daysUntilExpiry === 1 ? "" : "s"})
                  </span>
                {/if}
              </dd>

              <dt class="text-fg-muted">SANs</dt>
              <dd class="text-fg font-mono">
                {#if certInfo.sans.length === 0}
                  <span class="text-fg-subtle">—</span>
                {:else}
                  {certInfo.sans.join(", ")}
                {/if}
              </dd>

              <dt class="text-fg-muted">Path</dt>
              <dd class="flex items-center gap-2 min-w-0">
                <span class="text-fg font-mono text-[11px] truncate">{certInfo.certificatePath}</span>
                <button
                  type="button"
                  onclick={() => copyToClipboard(certInfo!.certificatePath, "Cert path")}
                  title="Copy path"
                  class="p-0.5 rounded text-fg-subtle hover:text-fg"
                >
                  <Icon name="link" size={11} />
                </button>
              </dd>
            </dl>
          {/if}
        </DashboardCard>
      {/if}

      <!-- Configuration -->
      <DashboardCard title="Configuration" flush>
        {#snippet badge()}
          {#if dirty}
            <span class="text-[11px] text-status-unhealthy">Unsaved</span>
          {/if}
        {/snippet}
        <div class="grid grid-cols-[120px,1fr] gap-x-4 gap-y-3 items-center text-sm">
          <label for="detail-name" class="text-fg-muted">Name</label>
          <input
            id="detail-name"
            type="text"
            bind:value={nameDraft}
            oninput={markDirty}
            class="px-2.5 py-1.5 rounded-md bg-bg border border-border focus:border-accent/60 outline-none text-fg"
          />
          <label for="detail-host" class="text-fg-muted">Hostname</label>
          <input
            id="detail-host"
            type="text"
            bind:value={hostnameDraft}
            oninput={markDirty}
            class="px-2.5 py-1.5 rounded-md bg-bg border border-border focus:border-accent/60 outline-none text-fg font-mono"
          />
          <label for="detail-port" class="text-fg-muted">Port</label>
          <input
            id="detail-port"
            type="number"
            min="1"
            max="65535"
            value={portDraft ?? ""}
            oninput={(e) => {
              const v = (e.currentTarget as HTMLInputElement).value;
              portDraft = v ? Number(v) : null;
              dirty = true;
            }}
            class="px-2.5 py-1.5 rounded-md bg-bg border border-border focus:border-accent/60 outline-none text-fg font-mono w-32"
          />
          <label for="detail-cmd" class="text-fg-muted self-start pt-1.5">
            Start command
          </label>
          <input
            id="detail-cmd"
            type="text"
            bind:value={startCommandDraft}
            oninput={markDirty}
            class="px-2.5 py-1.5 rounded-md bg-bg border border-border focus:border-accent/60 outline-none text-fg font-mono"
          />
          {#if project.type === "php"}
            <label for="detail-web-server" class="text-fg-muted">Web server</label>
            <div class="space-y-1">
              <select
                id="detail-web-server"
                bind:value={webServerDraft}
                onchange={markDirty}
                class="px-2.5 py-1.5 rounded-md bg-bg border border-border focus:border-accent/60 outline-none text-fg w-40"
              >
                {#each Object.entries(webServerLabel) as [value, label] (value)}
                  <option value={value}>{label}</option>
                {/each}
              </select>
              {#if startCommandDraft.trim()}
                <p class="text-[11px] text-fg-subtle">
                  Custom PHP commands are reverse-proxied by Caddy. Clear the
                  start command to run this project through the selected
                  generated backend.
                </p>
              {/if}
            </div>
          {/if}
          <span class="text-fg-muted">Options</span>
          <div class="flex items-center gap-4">
            <label class="flex items-center gap-1.5 text-xs cursor-pointer">
              <input
                type="checkbox"
                bind:checked={httpsDraft}
                onchange={markDirty}
                class="accent-accent"
              />
              HTTPS
            </label>
            <label class="flex items-center gap-1.5 text-xs cursor-pointer">
              <input
                type="checkbox"
                bind:checked={autoStartDraft}
                onchange={markDirty}
                class="accent-accent"
              />
              Auto-start
            </label>
          </div>
        </div>
        {#if dirty}
          <div class="flex items-center justify-end gap-2 pt-3 mt-3 border-t border-border">
            <button
              type="button"
              onclick={discard}
              class="px-3 py-1.5 text-xs rounded-md text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
            >
              Discard
            </button>
            <button
              type="button"
              onclick={save}
              disabled={saving}
              class="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-md text-accent border border-accent/40 hover:bg-accent/10 disabled:opacity-50 transition-colors"
            >
              {#if saving}
                <Icon name="refresh-cw" size={11} class="animate-spin" />
                Saving…
              {:else}
                <Icon name="check" size={11} />
                Save (⌘S)
              {/if}
            </button>
          </div>
        {/if}
      </DashboardCard>

      <!-- Environment -->
      <DashboardCard title="Environment" flush>
        <EnvEditor {project} />
      </DashboardCard>

      <!-- Database connection(s) parsed from the project's .env (if any) -->
      <ProjectDbConnections {project} />

      <!-- Build artifacts (disk usage + clean), if any are present -->
      <ArtifactsSection {project} />

      <!-- Advanced — tags / extra ports / services / PHP -->
      <DashboardCard title="Advanced" flush>
        <AdvancedFields {project} />
      </DashboardCard>

      <!-- Logs preview -->
      <DashboardCard title="Recent logs" flush>
        {#snippet badge()}
          <div class="flex items-center gap-1">
            <button
              type="button"
              onclick={() => project && logViewer.show(project.id)}
              title="Open full log viewer"
              class="text-[11px] text-accent hover:text-accent-hover px-1.5 py-0.5"
            >
              Open viewer
            </button>
            <button
              type="button"
              onclick={loadLogs}
              title="Refresh logs"
              class="p-1 rounded-md text-fg-subtle hover:text-fg hover:bg-surface-2 transition-colors"
              class:animate-spin={logLoading}
            >
              <Icon name="refresh-cw" size={11} />
            </button>
          </div>
        {/snippet}
        {#if logTailParsed.length === 0}
          <p class="text-xs text-fg-subtle">
            {logLoading ? "Loading…" : "No log output yet."}
          </p>
        {:else}
          <div
            class="text-[11px] font-mono leading-relaxed text-fg-muted bg-bg/60 border border-border rounded-md p-2 overflow-x-auto max-h-48"
          >
            {#each logTailParsed as pl, i (i)}
              <div class="whitespace-pre-wrap break-words {levelClass(pl.level)}">
                {@html pl.html}
              </div>
            {/each}
          </div>
        {/if}
      </DashboardCard>

      <!-- Raw config -->
      <DashboardCard title="Advanced" flush>
        <button
          type="button"
          onclick={() => (rawConfigOpen = !rawConfigOpen)}
          class="text-xs text-fg-muted hover:text-fg inline-flex items-center gap-1"
        >
          <Icon
            name={rawConfigOpen ? "chevron-down" : "chevron-right"}
            size={11}
          />
          {rawConfigOpen ? "Hide raw config" : "Show raw config"}
        </button>
        {#if rawConfigOpen}
          <p class="text-[11px] text-fg-subtle mt-2">
            Edits here apply to the fields above on blur. Click Save below.
          </p>
          <textarea
            bind:value={rawDraft}
            onblur={syncFieldsFromRaw}
            rows="14"
            spellcheck="false"
            class="mt-2 w-full px-3 py-2 rounded-md bg-bg border border-border focus:border-accent/60 outline-none text-xs font-mono text-fg leading-relaxed"
          ></textarea>
        {/if}
      </DashboardCard>
    </div>
  </aside>
{/if}
