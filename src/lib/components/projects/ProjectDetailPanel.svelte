<!--
  ProjectDetailPanel — slide-over right pane with full project controls,
  recent log tail, connection info, and an inline "edit raw" L3 escape.

  Reads the selected project from the projects store (no separate fetch
  for the panel). Refreshes the log tail on open and on demand.
-->
<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { trapFocus } from "$lib/actions/trapFocus";
  import { openUrl } from "@tauri-apps/plugin-opener";

  import { DashboardCard, Icon, StatusPill } from "$lib/components/atoms";
  import EnvEditor from "./EnvEditor.svelte";
  import AdvancedFields from "./AdvancedFields.svelte";
  import { ErrorEnvelope } from "$lib/components/errors";
  import { safeInvoke } from "$lib/ipc";
  import { errorBus } from "$lib/stores/errors.svelte";
  import { projectDetailPanel } from "$lib/stores/detailPanel.svelte";
  import { logViewer } from "$lib/stores/logViewer.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import { dns } from "$lib/stores/dns.svelte";
  import type { CertInfo } from "$lib/types/certs";
  import type { CommandError } from "$lib/types/error";
  import type { ProjectView, ProjectType } from "$lib/types/projects";
  import { typeLabel } from "$lib/types/projects";

  // Currently-displayed project; null while panel is closed.
  const project = $derived<ProjectView | null>(
    projectDetailPanel.id === null
      ? null
      : (projects.value.find((p) => p.id === projectDetailPanel.id) ?? null),
  );

  // Editable form state — initialised on open / when target project changes.
  let nameDraft = $state<string>("");
  let hostnameDraft = $state<string>("");
  let portDraft = $state<number | null>(null);
  let startCommandDraft = $state<string>("");
  let httpsDraft = $state<boolean>(true);
  let autoStartDraft = $state<boolean>(false);

  let dirty = $state<boolean>(false);
  let saving = $state<boolean>(false);
  let formError = $state<CommandError | null>(null);

  let logTail = $state<string[]>([]);
  let logLoading = $state<boolean>(false);

  let certInfo = $state<CertInfo | null>(null);
  let certLoading = $state<boolean>(false);
  let certError = $state<string | null>(null);
  let reissuing = $state<boolean>(false);

  let rawConfigOpen = $state<boolean>(false);
  let rawDraft = $state<string>("");

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
      httpsDraft = p.https;
      autoStartDraft = p.autoStart;
      dirty = false;
      formError = null;
      rawConfigOpen = false;
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
          startCommand: startCommandDraft || undefined,
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

  async function loadCert() {
    if (!project || !project.https) {
      certInfo = null;
      certError = null;
      return;
    }
    certLoading = true;
    certError = null;
    try {
      certInfo = await safeInvoke<CertInfo>("cert_info", { id: project.id });
    } catch (e) {
      certInfo = null;
      const err = e as CommandError | undefined;
      // PROJECT_NOT_FOUND from cert_info means "no cert issued yet" —
      // that's the empty state, not a hard error.
      certError = err && err.code !== "PROJECT_NOT_FOUND"
        ? err.whatHappened
        : null;
    } finally {
      certLoading = false;
    }
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
    try {
      switch (op) {
        case "start":
          await dns.ensureReady();
          await safeInvoke("start_project", { id: project.id });
          break;
        case "stop":
          await safeInvoke("stop_project", { id: project.id });
          break;
        case "restart":
          await safeInvoke("restart_project", { id: project.id });
          break;
      }
    } catch {
      /* toast already pushed */
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
  <!-- Backdrop is a mouse convenience; Escape (window handler) and the
       header close button cover keyboard users. -->
  <div
    class="fixed inset-0 z-40 bg-bg/60 backdrop-blur-sm"
    onclick={() => !dirty && projectDetailPanel.hide()}
    role="presentation"
  ></div>

  <aside
    use:trapFocus
    class="fixed inset-y-0 right-0 z-50 w-[640px] max-w-[90vw] bg-surface border-l border-border shadow-2xl flex flex-col"
    aria-label="Project detail"
  >
    <!-- Header -->
    <header
      class="shrink-0 px-5 py-4 border-b border-border space-y-3"
    >
      <div class="flex items-start justify-between gap-3">
        <div class="flex items-center gap-2.5 min-w-0">
          <StatusPill status={project.status} />
          <h2 class="text-base font-semibold truncate">{project.name}</h2>
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

      <div class="flex items-center gap-1.5">
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
        {#if logTail.length === 0}
          <p class="text-xs text-fg-subtle">
            {logLoading ? "Loading…" : "No log output yet."}
          </p>
        {:else}
          <pre
            class="text-[11px] font-mono leading-relaxed text-fg-muted bg-bg/60 border border-border rounded-md p-2 overflow-x-auto max-h-48"
          >{logTail.join("\n")}</pre>
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
