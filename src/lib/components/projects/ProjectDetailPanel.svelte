<!--
  ProjectDetailPanel — slide-over right pane with full project controls,
  recent log tail, connection info, and an inline "edit raw" L3 escape.

  Reads the selected project from the projects store (no separate fetch
  for the panel). Refreshes the log tail on open and on demand.
-->
<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { Channel } from "@tauri-apps/api/core";
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import { openUrl } from "$lib/security/openUrl";

  import { DashboardCard, Icon, StackIcon, StatusPill } from "$lib/components/atoms";
  import Popover from "$lib/components/atoms/Popover.svelte";
  import EnvEditor from "./EnvEditor.svelte";
  import AdvancedFields from "./AdvancedFields.svelte";
  import ProjectDbConnections from "./ProjectDbConnections.svelte";
  import ArtifactsSection from "./ArtifactsSection.svelte";
  import ProjectDeploySection from "./ProjectDeploySection.svelte";
  import { ErrorEnvelope } from "$lib/components/errors";
  import { safeInvoke, invokeQuiet } from "$lib/ipc";
  import { startProject, startProjectSandboxed } from "$lib/actions/startProject";
  import { errorBus } from "$lib/stores/errors.svelte";
  import { projectDetailPanel } from "$lib/stores/detailPanel.svelte";
  import { logViewer } from "$lib/stores/logViewer.svelte";
  import { parseLogLine, levelClass } from "$lib/components/logs/ansi";
  import { projects } from "$lib/stores/projects.svelte";
  import { dns } from "$lib/stores/dns.svelte";
  import { confirmDialog } from "$lib/stores/confirm.svelte";
  import HostnameField from "$lib/components/domains/HostnameField.svelte";
  import { entitlements } from "$lib/stores/entitlements.svelte";
  import { createCertInfo } from "$lib/stores/certInfo.svelte";
  import type { CommandError } from "$lib/types/error";
  import type {
    ProjectType,
    ProjectView,
    ProvisionEvent,
    ReadinessProbeResult,
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

  // The run controls are a single Start↔Stop toggle keyed on whether the
  // project is up: a stopped/crashed project only offers Start; an up one
  // (running, starting, unhealthy, or optimistically stopping) offers Stop +
  // Restart. Showing both Start and Stop at rest made no sense — there's
  // nothing to stop until it's running.
  const isUp = $derived(
    displayStatus === "running" ||
      displayStatus === "starting" ||
      displayStatus === "unhealthy" ||
      displayStatus === "stopping",
  );

  // Editable form state — initialised on open / when target project changes.
  let nameDraft = $state<string>("");
  let hostnameDraft = $state<string>("");
  let hostnameValid = $state<boolean>(true);
  // Active DNS suffix for the split hostname editor; matches the /domains page.
  const systemSuffix = $derived(dns.status?.suffix ?? "portbay.test");
  let portDraft = $state<number | null>(null);
  let startCommandDraft = $state<string>("");
  // Editable project kind, so a board-only `custom` project can be promoted
  // into a runnable web/app project (and back). Gates the PHP-only fields and
  // is sent to `update_project`, which recomputes services on a kind change.
  let kindDraft = $state<ProjectType>("custom");
  let webServerDraft = $state<WebServer>("caddy");
  let httpsDraft = $state<boolean>(true);
  let autoStartDraft = $state<boolean>(false);

  // Pre/post-start hook commands (one shell command per row) and the readiness
  // probe the project is gated on. Initialised from the project on open.
  let preStartDraft = $state<string[]>([]);
  let postStartDraft = $state<string[]>([]);
  let readinessTypeDraft = $state<"http" | "tcp" | "process">("http");
  let readinessPathDraft = $state<string>("/");
  let readinessTimeoutDraft = $state<number>(75);
  // "Probe now" one-shot test state.
  let probing = $state<boolean>(false);
  let probeResult = $state<ReadinessProbeResult | null>(null);

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

  type SandboxInstallReport = {
    command: string;
    ok: boolean;
    exitCode: number | null;
    output: string;
    violations: string[];
    blockedHosts: string[];
  };
  let sandboxInstalling = $state<boolean>(false);
  let sandboxInstallReport = $state<SandboxInstallReport | null>(null);

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
      kindDraft = p.type;
      webServerDraft = p.webServer ?? "caddy";
      httpsDraft = p.https;
      autoStartDraft = p.autoStart;
      preStartDraft = [...p.preStart];
      postStartDraft = [...p.postStart];
      readinessTypeDraft = p.readiness?.type ?? (p.port != null ? "http" : "process");
      readinessPathDraft = p.readiness?.path ?? "/";
      readinessTimeoutDraft = p.readiness?.timeout_seconds ?? 75;
      probeResult = null;
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
      type: kindDraft,
      hostname: hostnameDraft,
      port: portDraft ?? undefined,
      startCommand: startCommandDraft || undefined,
      webServer: kindDraft === "php" ? webServerDraft : undefined,
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
      if (typeof parsed.type === "string") kindDraft = parsed.type as ProjectType;
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
        category: "project-error",
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
    if (!project || !dirty || !hostnameValid) return;
    saving = true;
    formError = null;
    try {
      await safeInvoke<ProjectView>("update_project", {
        id: project.id,
        patch: {
          name: nameDraft,
          kind: kindDraft !== project.type ? kindDraft : undefined,
          hostname: hostnameDraft,
          port: portDraft ?? undefined,
          startCommand: startCommandDraft.trim() ? startCommandDraft : null,
          webServer: kindDraft === "php" ? webServerDraft : undefined,
          https: httpsDraft,
          autoStart: autoStartDraft,
          readiness: buildReadiness(),
          preStart: preStartDraft.map((c) => c.trim()).filter(Boolean),
          postStart: postStartDraft.map((c) => c.trim()).filter(Boolean),
        },
      });
      await projects.refresh();
      dirty = false;
      errorBus.push({
        code: "UPDATE_OK",
        category: "lifecycle",
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
    kindDraft = project.type;
    webServerDraft = project.webServer ?? "caddy";
    httpsDraft = project.https;
    autoStartDraft = project.autoStart;
    preStartDraft = [...project.preStart];
    postStartDraft = [...project.postStart];
    readinessTypeDraft = project.readiness?.type ?? (project.port != null ? "http" : "process");
    readinessPathDraft = project.readiness?.path ?? "/";
    readinessTimeoutDraft = project.readiness?.timeout_seconds ?? 75;
    probeResult = null;
    dirty = false;
    formError = null;
    syncRawFromFields();
  }

  /** Assemble the Readiness payload the backend expects from the form state. */
  function buildReadiness() {
    if (readinessTypeDraft === "http") {
      return {
        type: "http" as const,
        path: readinessPathDraft.trim() || "/",
        timeout_seconds: readinessTimeoutDraft,
      };
    }
    if (readinessTypeDraft === "tcp") {
      return { type: "tcp" as const, timeout_seconds: readinessTimeoutDraft };
    }
    return { type: "process" as const };
  }

  function addHook(which: "pre" | "post") {
    if (which === "pre") preStartDraft = [...preStartDraft, ""];
    else postStartDraft = [...postStartDraft, ""];
    dirty = true;
  }

  function removeHook(which: "pre" | "post", index: number) {
    if (which === "pre")
      preStartDraft = preStartDraft.filter((_, i) => i !== index);
    else postStartDraft = postStartDraft.filter((_, i) => i !== index);
    dirty = true;
  }

  /** Run the configured readiness check once against the local dev port. */
  async function probeNow() {
    probing = true;
    probeResult = null;
    try {
      probeResult = await safeInvoke<ReadinessProbeResult>("probe_readiness", {
        kind: readinessTypeDraft,
        port: portDraft ?? undefined,
        path:
          readinessTypeDraft === "http"
            ? readinessPathDraft.trim() || "/"
            : undefined,
      });
    } catch (e) {
      formError = e as CommandError;
    } finally {
      probing = false;
    }
  }

  async function loadLogs() {
    if (!project) return;
    logLoading = true;
    try {
      logTail = await invokeQuiet<string[]>("tail_logs", {
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
        category: "infrastructure",
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
    try {
      // Reveal the cert file selected in the OS file manager. `revealItemInDir`
      // is the opener API for this — `openUrl("file://…")` silently no-ops for
      // directories on macOS/Windows, which is why "Reveal" did nothing.
      await revealItemInDir(certInfo.certificatePath);
    } catch {
      /* opener pushes its own toast */
    }
  }

  function certStatusLabel(): string {
    if (!certInfo) return "Not issued";
    switch (certInfo.status) {
      case "ready":
        return certInfo.trustStoreVerified === false ? "Unverified" : "Ready";
      case "missingCa":
        return "Missing CA";
      case "expired":
        return "Expired";
      case "untrusted":
        return "Untrusted";
      case "regenerateNeeded":
        return "Regenerate needed";
      case "error":
        return "Error";
    }
  }

  async function exportPortfile() {
    if (!project) return;
    try {
      const written = await safeInvoke<string>("export_portfile", { id: project.id });
      errorBus.push({
        code: "EXPORT_OK",
        category: "lifecycle",
        whatHappened: `Wrote ${written}`,
        whyItMatters: "Commit this file to your repo so teammates get the same local setup.",
        whoCausedIt: "system",
        actions: [],
      });
    } catch {
      /* safeInvoke toast already pushed */
    }
  }


  // --- Python virtualenv provisioning ---------------------------------
  let provisioning = $state(false);
  let provisionLog = $state<string[]>([]);
  let provisionLogEl = $state<HTMLElement | null>(null);

  async function provisionPythonEnv() {
    if (!project || provisioning) return;
    const id = project.id;
    provisioning = true;
    provisionLog = [];

    const ch = new Channel<ProvisionEvent>();
    ch.onmessage = (event) => {
      if (event.kind === "log") {
        provisionLog = provisionLog.concat(event.line);
        requestAnimationFrame(() => {
          if (provisionLogEl) provisionLogEl.scrollTop = provisionLogEl.scrollHeight;
        });
      }
    };

    try {
      await safeInvoke("provision_python_env", { id, onEvent: ch });
      provisionLog = provisionLog.concat("✓ Environment ready");
    } catch {
      // safeInvoke already pushed the error toast; the inline log shows what ran.
    } finally {
      provisioning = false;
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
    const name = project.name;
    // Enabling sandbox rewrites this project's launch command, so Process
    // Compose reloads its config and briefly restarts every *other* running
    // project. (Re-running an already-sandboxed project doesn't change the
    // config, so this only fires on the first enable.) Warn before disrupting
    // running work.
    if (!project.sandboxed) {
      const others = projects.value.filter(
        (p) => p.id !== id && p.status === "running",
      ).length;
      if (others > 0) {
        const ok = await confirmDialog.open({
          title: "Start in sandbox?",
          message: `Sandboxing ${name} reloads Process Compose, which briefly restarts your ${others} other running project${others === 1 ? "" : "s"}. They'll come back on their own.`,
          actions: [
            { label: "Start in sandbox", value: "go", tone: "primary", icon: "shield" },
          ],
        });
        if (ok !== "go") return;
      }
    }
    projects.beginTransition(id, "start");
    try {
      await dns.ensureReady();
      // Resolves a port conflict via the shared confirm + force-quit prompt,
      // identical to the normal Play path.
      const r = await startProjectSandboxed(id, name, {
        network: sandboxNetwork,
        ephemeral: sandboxEphemeral,
      });
      if (r.kind === "declined") {
        projects.failTransition(id); // nothing started — roll back
        return;
      }
      if (r.kind === "error") {
        projects.failTransition(id);
        errorBus.push(r.error);
        return;
      }
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
        category: "lifecycle",
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

  async function installSandboxed() {
    if (!project || sandboxInstalling) return;
    sandboxInstalling = true;
    sandboxInstallReport = null;
    try {
      sandboxInstallReport = await safeInvoke<SandboxInstallReport>(
        "install_project_sandboxed",
        { id: project.id },
      );
    } catch {
      /* toast already pushed */
    } finally {
      sandboxInstalling = false;
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
      // Reveal the project folder in the default file manager. Same fix as the
      // cert reveal: `openUrl("file://…")` silently no-ops for directories.
      await revealItemInDir(project.path);
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
        category: "lifecycle",
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

  async function copyToClipboard(text: string, _label: string) {
    // Copying is a trivial, self-evident action — no notification (the user
    // just clicked Copy). Failures fall through silently (no clipboard perm).
    try {
      await navigator.clipboard.writeText(text);
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
    class="h-full w-full min-h-0 overflow-hidden bg-surface border-l border-border flex flex-col"
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
        {#if isUp}
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
        {:else}
          <button
            type="button"
            onclick={() => run("start")}
            class="inline-flex items-center gap-1.5 px-2.5 py-1.5 text-xs rounded-md text-status-running border border-status-running/40 hover:bg-status-running/10 transition-colors"
          >
            <Icon name="play" size={12} /> Start
          </button>
        {/if}
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
          <p class="text-[11px] text-fg-subtle leading-relaxed">
            Runs this project's command under macOS Seatbelt: blocks reads of your
            credentials, keychains, browser data, and other projects'
            <span class="font-mono">.env</span>; confines writes to the project.
            Not a VM — shares the host kernel and sets no CPU/memory limits, so
            use it for careless or untrusted code, not genuinely hostile code.
          </p>
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
            <button
              type="button"
              onclick={installSandboxed}
              disabled={sandboxInstalling}
              title="Run this project's dependency install (npm/pnpm/yarn/bun/composer) sandboxed: network pinned to package registries only, your secrets still blocked"
              class="inline-flex items-center gap-1.5 px-2.5 py-1.5 rounded-md
                     border border-border text-fg-muted hover:text-fg hover:bg-surface-2
                     disabled:opacity-50"
            >
              <Icon
                name={sandboxInstalling ? "refresh-cw" : "package"}
                size={12}
                class={sandboxInstalling ? "animate-spin" : ""}
              />
              {sandboxInstalling ? "Installing…" : "Install (sandboxed)"}
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

          {#if sandboxInstallReport}
            {@const r = sandboxInstallReport}
            <div class="space-y-1.5">
              <div
                class="flex items-center gap-1.5 text-[11px] {r.ok
                  ? 'text-status-running'
                  : 'text-status-unhealthy'}"
              >
                <Icon name={r.ok ? "check" : "circle-alert"} size={12} />
                <span class="font-mono">{r.command}</span>
                <span class="text-fg-subtle">
                  {r.ok
                    ? "completed"
                    : `failed${r.exitCode != null ? ` (exit ${r.exitCode})` : ""}`}
                </span>
              </div>
              {#if r.blockedHosts.length > 0}
                <p class="text-[11px] text-status-unhealthy">
                  Blocked {r.blockedHosts.length} non-registry host{r.blockedHosts
                    .length === 1
                    ? ""
                    : "s"} — the install tried to reach these, but only package registries are allowed:
                </p>
                <div
                  class="max-h-24 overflow-auto rounded-md border border-border bg-bg
                         p-2 font-mono text-[11px] text-status-unhealthy space-y-1"
                >
                  {#each r.blockedHosts as host}
                    <div>{host}</div>
                  {/each}
                </div>
              {/if}
              {#if r.violations.length > 0}
                <p class="text-[11px] text-status-unhealthy">
                  {r.violations.length} sandbox denial{r.violations.length === 1
                    ? ""
                    : "s"} during install — the profile blocked these:
                </p>
                <div
                  class="max-h-24 overflow-auto rounded-md border border-border bg-bg
                         p-2 font-mono text-[11px] text-status-unhealthy space-y-1"
                >
                  {#each r.violations as line}
                    <div>{line}</div>
                  {/each}
                </div>
              {/if}
              {#if r.output.trim()}
                <pre
                  class="max-h-40 overflow-auto rounded-md border border-border bg-bg
                         p-2 font-mono text-[11px] text-fg-muted whitespace-pre-wrap break-all">{r.output}</pre>
              {/if}
            </div>
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
              <dt class="text-fg-muted">Status</dt>
              <dd
                class={certInfo.status === "ready"
                  ? "text-status-running"
                  : certInfo.status === "regenerateNeeded"
                    ? "text-status-unhealthy"
                    : "text-status-crashed"}
              >
                {certStatusLabel()}
              </dd>

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
              {#if certInfo.errors.length > 0}
                <dt class="text-fg-muted">Error</dt>
                <dd class="text-status-crashed">{certInfo.errors.join("; ")}</dd>
              {/if}
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
          <label for="detail-host" class="text-fg-muted self-start pt-1.5"
            >Hostname</label
          >
          <div>
            <HostnameField
              id="detail-host"
              bind:value={hostnameDraft}
              {systemSuffix}
              onInput={markDirty}
              onValidChange={(v) => (hostnameValid = v)}
            />
          </div>
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
          <label for="detail-type" class="text-fg-muted self-start pt-1.5">Type</label>
          <div class="space-y-1">
            <Popover align="left" width="11rem">
              {#snippet trigger(toggle, open)}
                <button
                  id="detail-type"
                  type="button"
                  onclick={toggle}
                  aria-haspopup="listbox"
                  aria-expanded={open}
                  class="flex items-center gap-2 w-40 px-2.5 py-1.5 rounded-md bg-bg border
                         text-left transition-colors outline-none
                         {open ? 'border-accent/60' : 'border-border hover:border-border-strong'}"
                >
                  <StackIcon type={kindDraft} size={16} class="shrink-0" />
                  <span class="flex-1 truncate text-fg">{typeLabel[kindDraft]}</span>
                  <Icon name="chevron-down" size={14} class="text-fg-subtle shrink-0" />
                </button>
              {/snippet}
              {#snippet children(close)}
                <div role="listbox" aria-label="Project type" class="max-h-72 overflow-y-auto -m-0.5">
                  {#each Object.entries(typeLabel) as [value, label] (value)}
                    <button
                      type="button"
                      role="option"
                      aria-selected={kindDraft === value}
                      onclick={() => {
                        if (kindDraft !== value) {
                          kindDraft = value as ProjectType;
                          markDirty();
                        }
                        close();
                      }}
                      class="w-full flex items-center gap-2.5 px-2 py-1.5 rounded-md text-left
                             transition-colors {kindDraft === value
                        ? 'bg-accent/10'
                        : 'hover:bg-surface-2'}"
                    >
                      <StackIcon type={value as ProjectType} size={16} class="shrink-0" />
                      <span class="flex-1 truncate text-[13px] text-fg">{label}</span>
                      {#if kindDraft === value}
                        <Icon name="check" size={13} class="text-accent shrink-0" />
                      {/if}
                    </button>
                  {/each}
                </div>
              {/snippet}
            </Popover>
            {#if kindDraft !== project.type}
              <p class="text-[11px] text-fg-subtle">
                Changing the type promotes this into a
                <span class="font-mono">{typeLabel[kindDraft]}</span> project.
                Set a port and start command above so it can run; save, then
                start it from the projects table.
              </p>
            {/if}
          </div>
          {#if kindDraft === "php"}
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

        <!-- Hooks — shell commands chained around the dev server on start -->
        <div class="pt-4 mt-4 border-t border-border space-y-4">
          <div>
            <h4 class="text-xs font-medium text-fg flex items-center gap-1.5">
              <Icon name="terminal" size={12} />
              Hooks
            </h4>
            <p class="text-[11px] text-fg-subtle mt-0.5">
              Commands run in the project directory on every start. A pre-start
              command that exits non-zero stops the dev server from launching;
              a failed post-start only warns. Output appears in this project's
              logs.
            </p>
          </div>

          {#each [{ key: "pre", label: "Before start", rows: preStartDraft }, { key: "post", label: "After ready", rows: postStartDraft }] as group (group.key)}
            <div class="space-y-1.5">
              <span class="text-[11px] uppercase tracking-wide text-fg-muted">
                {group.label}
              </span>
              {#if group.rows.length === 0}
                <p class="text-[11px] text-fg-subtle italic">No commands.</p>
              {/if}
              {#each group.rows as _, i (i)}
                <div class="flex items-center gap-2">
                  <span class="text-[11px] text-fg-subtle font-mono w-4 text-right">
                    {i + 1}
                  </span>
                  {#if group.key === "pre"}
                    <input
                      type="text"
                      placeholder="e.g. pnpm install"
                      bind:value={preStartDraft[i]}
                      oninput={markDirty}
                      class="flex-1 px-2.5 py-1.5 rounded-md bg-bg border border-border focus:border-accent/60 outline-none text-fg font-mono text-xs"
                    />
                  {:else}
                    <input
                      type="text"
                      placeholder="e.g. curl -fsS http://127.0.0.1:3000/health"
                      bind:value={postStartDraft[i]}
                      oninput={markDirty}
                      class="flex-1 px-2.5 py-1.5 rounded-md bg-bg border border-border focus:border-accent/60 outline-none text-fg font-mono text-xs"
                    />
                  {/if}
                  <button
                    type="button"
                    onclick={() => removeHook(group.key as "pre" | "post", i)}
                    aria-label="Remove command"
                    class="p-1 rounded-md text-fg-subtle hover:text-status-crashed hover:bg-surface-2 transition-colors"
                  >
                    <Icon name="x" size={13} />
                  </button>
                </div>
              {/each}
              <button
                type="button"
                onclick={() => addHook(group.key as "pre" | "post")}
                class="inline-flex items-center gap-1 text-[11px] text-accent hover:underline"
              >
                <Icon name="plus" size={11} />
                Add command
              </button>
            </div>
          {/each}
        </div>

        <!-- Readiness — how PortBay decides the project is serving -->
        <div class="pt-4 mt-4 border-t border-border space-y-3">
          <div>
            <h4 class="text-xs font-medium text-fg flex items-center gap-1.5">
              <Icon name="activity" size={12} />
              Readiness
            </h4>
            <p class="text-[11px] text-fg-subtle mt-0.5">
              The check that flips this project from “starting” to “running”.
            </p>
          </div>

          <div class="flex flex-wrap items-center gap-3">
            {#each [{ v: "http", l: "HTTP" }, { v: "tcp", l: "TCP" }, { v: "process", l: "Process alive" }] as opt (opt.v)}
              <label class="flex items-center gap-1.5 text-xs cursor-pointer">
                <input
                  type="radio"
                  name="readiness-type"
                  value={opt.v}
                  checked={readinessTypeDraft === opt.v}
                  onchange={() => {
                    readinessTypeDraft = opt.v as "http" | "tcp" | "process";
                    probeResult = null;
                    markDirty();
                  }}
                  class="accent-accent"
                />
                {opt.l}
              </label>
            {/each}
          </div>

          {#if readinessTypeDraft === "http"}
            <div class="flex flex-wrap items-end gap-3">
              <label class="text-[11px] text-fg-muted">
                Path
                <input
                  type="text"
                  bind:value={readinessPathDraft}
                  oninput={markDirty}
                  placeholder="/api/health"
                  class="block mt-0.5 px-2.5 py-1.5 rounded-md bg-bg border border-border focus:border-accent/60 outline-none text-fg font-mono text-xs w-44"
                />
              </label>
              <label class="text-[11px] text-fg-muted">
                Timeout (s)
                <input
                  type="number"
                  min="1"
                  bind:value={readinessTimeoutDraft}
                  oninput={markDirty}
                  class="block mt-0.5 px-2.5 py-1.5 rounded-md bg-bg border border-border focus:border-accent/60 outline-none text-fg font-mono text-xs w-20"
                />
              </label>
            </div>
          {:else if readinessTypeDraft === "tcp"}
            <label class="text-[11px] text-fg-muted">
              Timeout (s)
              <input
                type="number"
                min="1"
                bind:value={readinessTimeoutDraft}
                oninput={markDirty}
                class="block mt-0.5 px-2.5 py-1.5 rounded-md bg-bg border border-border focus:border-accent/60 outline-none text-fg font-mono text-xs w-20"
              />
            </label>
          {:else}
            <p class="text-[11px] text-fg-subtle">
              Ready as soon as the process is running — no probe.
            </p>
          {/if}

          {#if readinessTypeDraft !== "process"}
            <div class="flex items-center gap-2">
              <button
                type="button"
                onclick={probeNow}
                disabled={probing || portDraft == null}
                class="inline-flex items-center gap-1.5 px-2.5 py-1.5 text-[11px] rounded-md text-fg-muted border border-border hover:bg-surface-2 disabled:opacity-50 transition-colors"
              >
                {#if probing}
                  <Icon name="refresh-cw" size={11} class="animate-spin" />
                  Probing…
                {:else}
                  <Icon name="play" size={11} />
                  Probe now
                {/if}
              </button>
              {#if portDraft == null}
                <span class="text-[11px] text-fg-subtle">Set a port to test.</span>
              {:else if probeResult}
                <span
                  class="text-[11px] font-mono {probeResult.ok
                    ? 'text-status-running'
                    : 'text-status-crashed'}"
                >
                  {probeResult.ok ? "✓" : "✗"}
                  {probeResult.detail} ({probeResult.elapsedMs}ms)
                </span>
              {/if}
            </div>
          {/if}
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
              disabled={saving || !hostnameValid}
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

      <!-- Python: create the project's .venv and install its dependencies -->
      {#if project.type === "python"}
        <DashboardCard title="Python environment" flush>
          <div class="px-3 py-2.5 space-y-2.5">
            <p class="text-[12px] text-fg-muted leading-snug">
              Create a <code>.venv</code> in the project and install its
              dependencies (from <code>requirements.txt</code> or
              <code>pyproject.toml</code>). Uses <code>uv</code> when available,
              otherwise the bundled <code>venv</code> module. Play and tasks then
              run inside this environment.
            </p>
            <button
              type="button"
              onclick={provisionPythonEnv}
              disabled={provisioning}
              class="px-2.5 py-1.5 rounded-md bg-accent text-accent-fg text-[12px] font-medium hover:bg-accent-hover disabled:opacity-60 disabled:cursor-not-allowed transition-colors"
            >
              {provisioning ? "Setting up…" : "Set up environment"}
            </button>
            {#if provisionLog.length > 0}
              <pre
                bind:this={provisionLogEl}
                class="max-h-48 overflow-auto rounded-md bg-bg border border-border p-2 text-[11px] leading-relaxed text-fg-muted whitespace-pre-wrap">{provisionLog.join("\n")}</pre>
            {/if}
          </div>
        </DashboardCard>
      {/if}

      <!-- Build artifacts (disk usage + clean), if any are present -->
      <ArtifactsSection {project} />

      <!-- Deploy this project to a saved SSH host -->
      <DashboardCard title="Deploy to a host" flush>
        <ProjectDeploySection projectId={project.id} embedded />
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
