<!--
  AddProjectWizard — slide-over panel from the right.

  Three depth levels in one screen (docs/UX_DESIGN.md §5.2):
    L1 — Drop / browse for folder → auto-detect.
    L2 — Standard fields (name, hostname, port, start cmd, https, autostart).
    L3 — "Show raw config" toggle reveals a monospace JSON editor; on
         blur, JSON edits override the L2 fields.

  ESC + backdrop click + × button all close. Unsaved input is preserved
  while the panel stays open; closing discards.
-->
<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { getCurrentWebview, type DragDropEvent } from "@tauri-apps/api/webview";
  import type { UnlistenFn } from "@tauri-apps/api/event";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";

  import { CodeEditor, DashboardCard, Icon } from "$lib/components/atoms";
  import { ErrorEnvelope } from "$lib/components/errors";
  import { safeInvoke } from "$lib/ipc";
  import { errorBus } from "$lib/stores/errors.svelte";
  import { preferences } from "$lib/stores/preferences.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import { entitlements } from "$lib/stores/entitlements.svelte";
  import { addProjectWizard } from "$lib/stores/wizard.svelte";
  import { onboarding } from "$lib/stores/onboarding.svelte";
  import type { CommandError } from "$lib/types/error";
  import type { PortbayFile, PortfilePreview } from "$lib/types/portfile";
  import type {
    MobileRunConfig,
    ProjectType,
    ProjectView,
    WebServer,
    Workspace,
    WorkspaceApp,
    WorkspaceScan,
    SandboxNetworkPolicy,
  } from "$lib/types/projects";
  import { typeLabel } from "$lib/types/projects";
  import type { DetectedProject } from "$lib/types/wizard";

  // ----- Form state -----
  let path = $state<string>("");
  let id = $state<string>("");
  let name = $state<string>("");
  let hostname = $state<string>("");
  let port = $state<number | null>(null);
  let startCommand = $state<string>("");
  let documentRoot = $state<string>("");
  let phpVersion = $state<string>("");
  let webServer = $state<WebServer>(
    preferences.value.defaultWebServer ?? "caddy",
  );
  let mobileRun = $state<MobileRunConfig | null>(null);
  let kind = $state<ProjectType>("custom");
  let https = $state<boolean>(true);
  let autoStart = $state<boolean>(false);

  let detecting = $state<boolean>(false);
  let submitting = $state<boolean>(false);
  let formError = $state<CommandError | null>(null);
  let rawConfigOpen = $state<boolean>(false);
  let rawDraft = $state<string>("");
  let dropActive = $state<boolean>(false);
  let dropHint = $state<string>("");
  let gitUrl = $state<string>("");
  let gitParentDir = $state<string>("");
  let gitCloneRunning = $state<boolean>(false);
  let gitNetwork = $state<SandboxNetworkPolicy>("outbound");

  // Clone-in-Sandbox makes a *new* sandboxed project, so it's allowed whenever
  // the tier has room under the community cap (Pro is unlimited), not Pro-only.
  const canSandboxNew = $derived(
    entitlements.canSandbox(projects.value.filter((p) => p.sandboxed).length),
  );

  /**
   * Inline port-conflict warning. Backed by a debounced lsof probe so
   * the user is told *while typing* if the port they're claiming is
   * already bound (typically by ServBay, MAMP, or a stray dev server).
   * Empty string when the port is free or unknown.
   */
  let portConflict = $state<string>("");
  let portCheckTimer: ReturnType<typeof setTimeout> | null = null;

  function schedulePortCheck(value: number | null) {
    if (portCheckTimer) clearTimeout(portCheckTimer);
    portConflict = "";
    if (!value || value < 1 || value > 65535) return;
    portCheckTimer = setTimeout(async () => {
      try {
        const holder = await safeInvoke<string | null>("preview_port_conflict", {
          port: value,
        });
        // Only set if the port the user *currently* has typed is still
        // the one we probed (they may have typed something else since).
        if (holder && port === value) {
          portConflict = holder;
        }
      } catch {
        /* benign — leave inline state empty */
      }
    }, 350);
  }
  let dragUnlisten: UnlistenFn | null = null;

  /**
   * Populated when the picked folder contains a `.portbay.json`. The
   * wizard switches into "importing" mode: L2 is locked to the file's
   * values, the secrets list is rendered as required inputs above
   * Commit, and submission goes through `import_portfile_commit`
   * instead of `add_project`.
   */
  let portfile = $state<PortbayFile | null>(null);
  let portfileSecrets = $state<Record<string, string>>({});
  /** True when the file's derived id already exists — surfaced before commit. */
  let portfileIdCollision = $state<boolean>(false);

  const commandPlaceholder = $derived(
    kind === "php"
      ? "leave empty for PortBay-managed PHP-FPM"
      : kind === "flutter"
        ? "flutter run"
        : kind === "xcode"
          ? "xed ."
          : kind === "android"
            ? "./gradlew installDebug"
            : "pnpm dev",
  );

  function isMobileKind(value: ProjectType): boolean {
    return value === "flutter" || value === "xcode" || value === "android";
  }

  /**
   * Set when the picked folder is a JS monorepo root. The wizard shows a
   * one-app picker; choosing an app fills the standard fields with that app's
   * sub-directory so it's added as its own standalone project (rather than a
   * root `pnpm dev` that fans out to every app). `null` = not a monorepo.
   */
  let workspaceScan = $state<WorkspaceScan | null>(null);
  /** Absolute path of the app the user picked from `workspaceScan`, if any. */
  let workspaceChosenPath = $state<string>("");
  /** The monorepo root path — preserved so "use whole repo" still works after
   *  picking an app (which overwrites `path` with the app's sub-directory). */
  let workspaceRootPath = $state<string>("");
  /**
   * Tier-2 toggle: run the chosen app from the repo ROOT via a workspace
   * filter (for apps whose dev script needs the monorepo build pipeline)
   * instead of directly from its own folder. Off by default (Tier 1).
   */
  let workspaceFromRoot = $state<boolean>(false);
  /** The app currently selected in the picker — re-applied when the toggle flips. */
  let workspaceChosenApp = $state<WorkspaceApp | null>(null);
  /** The workspace binding sent to add_project (Tier 2 only); null = standalone. */
  let workspaceBinding = $state<Workspace | null>(null);

  function resetForm() {
    path = "";
    id = "";
    name = "";
    hostname = "";
    port = null;
    startCommand = "";
    documentRoot = "";
    phpVersion = "";
    webServer = preferences.value.defaultWebServer ?? "caddy";
    kind = "custom";
    https = true;
    autoStart = false;
    rawConfigOpen = false;
    rawDraft = "";
    dropActive = false;
    dropHint = "";
    gitUrl = "";
    gitParentDir = "";
    gitCloneRunning = false;
    gitNetwork = "outbound";
    portfile = null;
    portfileSecrets = {};
    portfileIdCollision = false;
    workspaceScan = null;
    workspaceChosenPath = "";
    workspaceRootPath = "";
    workspaceFromRoot = false;
    workspaceChosenApp = null;
    workspaceBinding = null;
    formError = null;
  }

  function close() {
    addProjectWizard.hide();
    // Defer reset to next tick so the slide-out animation doesn't flash empty.
    setTimeout(resetForm, 250);
  }

  // ----- L1: pick a folder, detect framework -----
  async function browse() {
    const picked = await openDialog({
      directory: true,
      multiple: false,
      title: "Select project folder",
    });
    if (!picked || Array.isArray(picked)) return;
    await detect(picked as string);
  }

  async function browseSandboxInstallFolder() {
    const picked = await openDialog({
      directory: true,
      multiple: false,
      title: "Select sandbox install folder",
    });
    if (!picked || Array.isArray(picked)) return;
    gitParentDir = picked as string;
  }

  async function detect(folderPath: string) {
    path = folderPath;
    detecting = true;
    formError = null;
    portfile = null;
    portfileSecrets = {};
    portfileIdCollision = false;
    workspaceScan = null;
    workspaceChosenPath = "";
    workspaceRootPath = "";
    workspaceFromRoot = false;
    workspaceChosenApp = null;
    workspaceBinding = null;
    try {
      // Probe for a committed `.portbay.json` first. If present, the
      // file's values win over framework auto-detection.
      const file = await safeInvoke<PortbayFile | null>("detect_portfile", {
        path: folderPath,
      });
      if (file) {
        portfile = file;
        portfileSecrets = Object.fromEntries(
          (file.secrets ?? []).map((k) => [k, ""]),
        );
        name = file.name;
        hostname = file.hostname;
        port = file.port ?? null;
        startCommand = file.startCommand ?? "";
        documentRoot = file.documentRoot ?? "";
        phpVersion = file.phpVersion ?? "";
        webServer =
          file.webServer ?? preferences.value.defaultWebServer ?? "caddy";
        mobileRun = file.mobileRun ?? null;
        kind = file.type;
        https = file.https;
        autoStart = file.autoStart;
        // id is derived on the backend from the folder's last component.
        const seg = folderPath.split("/").filter(Boolean).pop() ?? "imported";
        id = seg
          .toLowerCase()
          .replace(/[^a-z0-9]+/g, "-")
          .replace(/^-+|-+$/g, "");
        // Preview/confirm step: ask the backend whether this import would
        // collide with an existing project id, and surface it before commit
        // instead of letting the commit fail.
        try {
          const preview = await safeInvoke<PortfilePreview>(
            "import_portfile_preview",
            { path: folderPath },
          );
          portfileIdCollision = preview.idCollision;
        } catch {
          // Non-fatal — detection already succeeded; just skip the hint.
          portfileIdCollision = false;
        }
        syncRawFromFields();
        return;
      }

      // Monorepo probe: if the folder is a workspace root with runnable
      // apps, show the app picker instead of auto-filling. The standard
      // fields fill in once the user chooses an app (chooseWorkspaceApp).
      try {
        const scan = await safeInvoke<WorkspaceScan | null>(
          "detect_workspace_apps",
          { path: folderPath },
        );
        if (scan && scan.apps.length > 0) {
          workspaceScan = scan;
          workspaceRootPath = folderPath;
          return;
        }
      } catch {
        // Non-fatal — fall through to single-folder detection.
      }

      await detectSingleFolder(folderPath);
    } catch (e) {
      // safeInvoke already toasted; surface inline too so the user knows
      // the form didn't autofill.
      formError = e as CommandError;
    } finally {
      detecting = false;
    }
  }

  /** Single-folder framework detection — fills the standard fields. */
  async function detectSingleFolder(folderPath: string) {
    const det = await safeInvoke<DetectedProject>("detect_project", {
      path: folderPath,
    });
    id = det.suggestedId;
    name = det.suggestedName;
    hostname = det.suggestedHostname;
    port = det.suggestedPort ?? null;
    startCommand = det.suggestedStartCommand ?? "";
    documentRoot = det.suggestedDocumentRoot ?? "";
    phpVersion = det.suggestedPhpVersion ?? "";
    webServer =
      det.suggestedWebServer ?? preferences.value.defaultWebServer ?? "caddy";
    mobileRun = det.suggestedMobileRun ?? null;
    kind = det.kind;
    syncRawFromFields();
    schedulePortCheck(port);
  }

  /**
   * The user picked one app from the monorepo. Configure it as a standalone
   * project rooted at its sub-directory — so only this app runs, not the whole
   * `turbo --parallel` fan-out. (Running from the repo root via a workspace
   * filter is a Tier-2 option set later in the project detail panel.)
   */
  function chooseWorkspaceApp(app: WorkspaceApp) {
    workspaceChosenApp = app;
    workspaceChosenPath = app.path;
    id = app.suggestedId;
    name = app.suggestedName;
    hostname = app.suggestedHostname;
    port = app.suggestedPort ?? null;
    kind = app.kind;
    documentRoot = "";
    phpVersion = "";
    webServer = preferences.value.defaultWebServer ?? "caddy";
    mobileRun = null;
    if (workspaceFromRoot && workspaceScan) {
      // Tier 2: run from the repo root with a workspace filter. Leave the
      // start command empty so the backend derives `<tool> --filter … dev`.
      path = workspaceRootPath;
      startCommand = "";
      workspaceBinding = {
        package: app.package,
        relDir: app.relDir,
        tool: workspaceScan.tool,
      };
    } else {
      // Tier 1: standalone project rooted at the app's own directory.
      path = app.path;
      startCommand = app.suggestedStartCommand ?? "";
      workspaceBinding = null;
    }
    formError = null;
    syncRawFromFields();
    schedulePortCheck(port);
  }

  /** Re-apply the chosen app when the Tier-1/Tier-2 toggle flips. */
  function onWorkspaceModeToggle() {
    if (workspaceChosenApp) chooseWorkspaceApp(workspaceChosenApp);
  }

  /** Dismiss the monorepo picker and treat the root as a single project. */
  async function useWholeRepo() {
    const root = workspaceRootPath || path;
    path = root;
    workspaceScan = null;
    workspaceChosenPath = "";
    workspaceRootPath = "";
    workspaceFromRoot = false;
    workspaceChosenApp = null;
    workspaceBinding = null;
    detecting = true;
    try {
      await detectSingleFolder(root);
    } catch (e) {
      formError = e as CommandError;
    } finally {
      detecting = false;
    }
  }

  async function handleDroppedPaths(paths: string[]) {
    dropActive = false;
    if (!addProjectWizard.isOpen) return;
    if (paths.length === 0) return;
    if (paths.length > 1) {
      dropHint = "One project at a time. Using the first dropped path.";
    } else {
      dropHint = "";
    }
    try {
      const folder = await safeInvoke<string>("validate_project_folder", {
        path: paths[0],
      });
      await detect(folder);
    } catch (e) {
      formError = e as CommandError;
      dropHint = "";
    }
  }

  function onDragDropEvent(event: { payload: DragDropEvent }) {
    if (!addProjectWizard.isOpen) return;
    switch (event.payload.type) {
      case "enter":
      case "over":
        dropActive = true;
        break;
      case "leave":
        dropActive = false;
        break;
      case "drop":
        void handleDroppedPaths(event.payload.paths);
        break;
    }
  }

  // ----- L3: raw config round-trips -----
  function syncRawFromFields() {
    const obj = {
      id,
      name,
      path,
      type: kind,
      startCommand: startCommand || undefined,
      documentRoot: kind === "php" && documentRoot ? documentRoot : undefined,
      phpVersion: kind === "php" && phpVersion ? phpVersion : undefined,
      webServer: kind === "php" ? webServer : undefined,
      mobileRun: isMobileKind(kind) ? mobileRun : undefined,
      port: port ?? undefined,
      hostname,
      https,
      autoStart,
    };
    rawDraft = JSON.stringify(obj, null, 2);
  }

  function syncFieldsFromRaw() {
    if (!rawDraft.trim()) return;
    try {
      const parsed = JSON.parse(rawDraft);
      if (typeof parsed.id === "string") id = parsed.id;
      if (typeof parsed.name === "string") name = parsed.name;
      if (typeof parsed.path === "string") path = parsed.path;
      if (typeof parsed.type === "string") kind = parsed.type as ProjectType;
      if (typeof parsed.startCommand === "string")
        startCommand = parsed.startCommand;
      if (typeof parsed.documentRoot === "string")
        documentRoot = parsed.documentRoot;
      if (typeof parsed.phpVersion === "string") phpVersion = parsed.phpVersion;
      if (
        parsed.webServer === "caddy" ||
        parsed.webServer === "nginx" ||
        parsed.webServer === "apache"
      )
        webServer = parsed.webServer;
      if (parsed.mobileRun && typeof parsed.mobileRun === "object") {
        mobileRun = parsed.mobileRun as MobileRunConfig;
      }
      if (typeof parsed.port === "number") port = parsed.port;
      if (typeof parsed.hostname === "string") hostname = parsed.hostname;
      if (typeof parsed.https === "boolean") https = parsed.https;
      if (typeof parsed.autoStart === "boolean") autoStart = parsed.autoStart;
      formError = null;
    } catch (e) {
      formError = {
        code: "BAD_RAW_JSON",
        whatHappened: `Raw config is not valid JSON: ${String(e)}`,
        whyItMatters: "Fix the JSON to apply your edits, or revert via the fields above.",
        whoCausedIt: "user",
        actions: [],
      };
    }
  }

  // ----- Commit -----
  async function commit() {
    if (!path) {
      formError = {
        code: "BAD_INPUT",
        whatHappened: "Pick a project folder first.",
        whyItMatters: "PortBay needs to know where the project lives.",
        whoCausedIt: "user",
        actions: [],
      };
      return;
    }
    submitting = true;
    formError = null;
    try {
      if (portfile) {
        // .portbay.json import path. Validate every required secret is
        // filled before sending — backend rejects with SecretMissing
        // otherwise and the GUI would have to re-prompt anyway.
        const missing = (portfile.secrets ?? []).filter(
          (k) => !portfileSecrets[k] || portfileSecrets[k] === "",
        );
        if (missing.length > 0) {
          formError = {
            code: "BAD_INPUT",
            whatHappened: `Fill in ${missing.join(", ")} before importing.`,
            whyItMatters:
              "The .portbay.json lists these as secrets so they're never committed to the repo.",
            whoCausedIt: "user",
            actions: [],
          };
          submitting = false;
          return;
        }
        await safeInvoke<string>("import_portfile_commit", {
          input: {
            path,
            id: id || undefined,
            secrets: portfileSecrets,
          },
        });
      } else {
        await safeInvoke<ProjectView>("add_project", {
          input: {
            path,
            id: id || undefined,
            name: name || undefined,
            hostname: hostname || undefined,
            kind,
            port: port ?? undefined,
            startCommand: startCommand || undefined,
            documentRoot:
              kind === "php" && documentRoot ? documentRoot : undefined,
            phpVersion: kind === "php" && phpVersion ? phpVersion : undefined,
            webServer: kind === "php" ? webServer : undefined,
            mobileRun: isMobileKind(kind) ? mobileRun : undefined,
            https,
            autoStart,
            workspace: workspaceBinding ?? undefined,
          },
        });
      }
      // Refresh table to pick up the new row.
      await projects.refresh();
      // First successful add of any kind counts as completing
      // onboarding — write the marker so the user isn't bounced
      // back to /onboarding on next launch. Fire-and-forget; a
      // failed marker write doesn't undo a successful add.
      void onboarding.markOnboarded();
      errorBus.push({
        code: portfile ? "IMPORT_OK" : "ADD_OK",
        whatHappened: `${name || id} ${portfile ? "imported from .portbay.json" : "added"}.`,
        whyItMatters: "Start it from the projects table when you're ready.",
        whoCausedIt: "system",
        actions: [],
      });
      close();
    } catch (e) {
      formError = e as CommandError;
    } finally {
      submitting = false;
    }
  }

  async function cloneSandboxed() {
    if (!gitUrl.trim()) {
      formError = {
        code: "BAD_INPUT",
        whatHappened: "Paste a Git URL first.",
        whyItMatters: "PortBay needs a repository URL to clone into the sandbox imports folder.",
        whoCausedIt: "user",
        actions: [],
      };
      return;
    }
    gitCloneRunning = true;
    formError = null;
    try {
      const project = await safeInvoke<ProjectView>("clone_git_project_sandboxed", {
        input: {
          url: gitUrl,
          parentDir: gitParentDir.trim() || null,
          network: gitNetwork,
          ephemeral: true,
          startAfterImport: true,
        },
      });
      await projects.refresh();
      void onboarding.markOnboarded();
      errorBus.push({
        code: "SANDBOX_IMPORT_OK",
        whatHappened: `${project.name} cloned and started in Sandbox.`,
        whyItMatters: "Inspect logs and promote it to local when you trust it.",
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });
      close();
    } catch (e) {
      formError = e as CommandError;
    } finally {
      gitCloneRunning = false;
    }
  }

  function onKeydown(e: KeyboardEvent) {
    if (!addProjectWizard.isOpen) return;
    if (e.key === "Escape") close();
  }

  // Track form mutations so the raw view stays in sync until the user
  // opens the L3 editor and starts diverging.
  $effect(() => {
    if (!rawConfigOpen) syncRawFromFields();
  });

  onMount(() => {
    void getCurrentWebview().onDragDropEvent(onDragDropEvent).then((unlisten) => {
      dragUnlisten = unlisten;
    });
    return () => {
      untrack(() => {
        dragUnlisten?.();
        dragUnlisten = null;
      });
    };
  });
</script>

<svelte:window onkeydown={onKeydown} />

{#if addProjectWizard.isOpen}
  <!-- In-layout right-side panel (rendered into the grid rail by the root
       layout) — matches the project detail panel and every other side surface.
       Escape + the header close button dismiss it. -->
  <aside
    class="h-full w-full min-h-0 overflow-hidden bg-surface border-l border-border flex flex-col"
    aria-label="Add Project"
  >
    <header
      class="shrink-0 flex items-center justify-between px-5 py-4 border-b border-border"
    >
      <h2 class="text-base font-semibold">Add project</h2>
      <button
        type="button"
        onclick={close}
        title="Close"
        aria-label="Close add project"
        class="p-1.5 rounded-md text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
      >
        <Icon name="x" size={16} />
      </button>
    </header>

    <div class="flex-1 min-h-0 overflow-y-auto p-5 space-y-4">
      {#if formError}
        <ErrorEnvelope envelope={formError} tone="inline" />
      {/if}

      <!-- L1: folder picker -->
      <DashboardCard title="Project folder" flush>
        <div
          class="mb-3 rounded-md border border-dashed px-4 py-5 text-center transition-colors
                 {dropActive
            ? 'border-accent bg-accent/10 text-fg'
            : 'border-border bg-bg/50 text-fg-muted'}"
        >
          <div class="text-sm font-medium text-fg">Drop your project folder here</div>
          <div class="mt-1 text-xs text-fg-subtle">
            PortBay will validate the folder and run the same detection flow as Browse.
          </div>
          {#if dropHint}
            <div class="mt-2 text-xs text-status-unhealthy">{dropHint}</div>
          {/if}
        </div>
        <div class="flex items-center gap-2">
          <input
            type="text"
            value={path}
            placeholder="/path/to/your/project"
            oninput={(e) =>
              (path = (e.currentTarget as HTMLInputElement).value)}
            class="flex-1 px-3 py-2 rounded-md text-sm bg-bg border border-border focus:border-accent/60 outline-none text-fg placeholder-fg-subtle font-mono"
          />
          <button
            type="button"
            onclick={browse}
            class="px-3 py-2 text-xs rounded-md border border-border text-fg-muted hover:text-fg hover:border-border-strong transition-colors whitespace-nowrap"
          >
            Browse…
          </button>
          <button
            type="button"
            onclick={() => path && detect(path)}
            disabled={!path || detecting}
            class="px-3 py-2 text-xs rounded-md text-accent border border-accent/40 hover:bg-accent/10 disabled:opacity-50 transition-colors"
          >
            {detecting ? "Detecting…" : "Detect"}
          </button>
        </div>
        <p class="text-xs text-fg-subtle pt-2">
          Pick a folder; PortBay auto-detects the framework, picks a port,
          and generates a <span class="font-mono">.test</span> hostname.
        </p>
      </DashboardCard>

      <DashboardCard title="Clone in Sandbox" flush>
        {#snippet badge()}
          {#if canSandboxNew}
            <span class="text-[11px] text-fg-subtle">Sandboxed</span>
          {:else}
            <span class="text-[11px] text-fg-subtle" title="Upgrade to Pro for unlimited sandboxed projects">
              Limit reached
            </span>
          {/if}
        {/snippet}
        <div class="space-y-3">
          <div class="flex items-center gap-2">
            <input
              type="url"
              bind:value={gitUrl}
              placeholder="https://github.com/org/repo.git"
              disabled={!canSandboxNew || gitCloneRunning}
              class="flex-1 px-3 py-2 rounded-md text-sm bg-bg border border-border
                     focus:border-accent/60 outline-none text-fg placeholder-fg-subtle
                     font-mono disabled:opacity-55"
            />
            <select
              bind:value={gitNetwork}
              disabled={!canSandboxNew || gitCloneRunning}
              class="px-3 py-2 rounded-md text-xs bg-bg border border-border text-fg
                     disabled:opacity-55"
              title="Sandbox network policy"
            >
              <option value="outbound">Outbound</option>
              <option value="loopback_only">Loopback</option>
              <option value="blocked">Blocked</option>
              <option value="full">Full</option>
            </select>
            <button
              type="button"
              onclick={cloneSandboxed}
              disabled={!canSandboxNew || gitCloneRunning}
              class="inline-flex items-center gap-1.5 px-3 py-2 text-xs rounded-md
                     text-accent border border-accent/40 hover:bg-accent/10
                     disabled:opacity-50 transition-colors whitespace-nowrap"
            >
              <Icon
                name={gitCloneRunning ? "refresh-cw" : "shield"}
                size={12}
                class={gitCloneRunning ? "animate-spin" : ""}
              />
              {gitCloneRunning ? "Cloning…" : "Clone & run"}
            </button>
          </div>
          <div class="flex items-center gap-2">
            <input
              type="text"
              bind:value={gitParentDir}
              placeholder="Install folder (defaults to PortBay sandbox imports)"
              disabled={!canSandboxNew || gitCloneRunning}
              class="flex-1 px-3 py-2 rounded-md text-xs bg-bg border border-border
                     focus:border-accent/60 outline-none text-fg placeholder-fg-subtle
                     font-mono disabled:opacity-55"
            />
            <button
              type="button"
              onclick={browseSandboxInstallFolder}
              disabled={!canSandboxNew || gitCloneRunning}
              class="px-3 py-2 text-xs rounded-md border border-border text-fg-muted
                     hover:text-fg hover:border-border-strong disabled:opacity-50
                     transition-colors whitespace-nowrap"
            >
              Folder…
            </button>
          </div>
          <p class="text-xs text-fg-subtle">
            External repos are cloned into the selected install folder, registered
            with sandbox mode enabled, and started under the selected policy.
          </p>
        </div>
      </DashboardCard>

      {#if portfile}
        <!-- .portbay.json import banner + secrets prompt -->
        <DashboardCard title="Importing from .portbay.json" flush>
          <p class="text-xs text-fg-muted">
            Settings below are loaded from <span class="font-mono">.portbay.json</span>
            in this folder.
            {#if (portfile.secrets ?? []).length > 0}
              Fill the secrets to finish.
            {:else}
              Click Commit to register the project.
            {/if}
          </p>

          {#if portfileIdCollision}
            <div
              class="mt-3 flex items-start gap-1.5 rounded-md border border-status-crashed/40 bg-status-crashed/10 px-3 py-2 text-[11px] text-status-crashed"
              role="alert"
            >
              <Icon name="circle-alert" size={12} class="mt-0.5 shrink-0" />
              <span>
                A project with ID <span class="font-mono">{id}</span> already
                exists. Change the ID below before importing, or the import will
                be rejected.
              </span>
            </div>
          {/if}

          {#if (portfile.secrets ?? []).length > 0}
            <div class="grid grid-cols-[140px,1fr] gap-x-4 gap-y-2 items-center text-sm mt-3">
              {#each portfile.secrets ?? [] as secret (secret)}
                <label for={`wizard-secret-${secret}`} class="text-fg-muted font-mono text-xs">
                  {secret}
                </label>
                <input
                  id={`wizard-secret-${secret}`}
                  type="password"
                  value={portfileSecrets[secret] ?? ""}
                  oninput={(e) => {
                    const v = (e.currentTarget as HTMLInputElement).value;
                    portfileSecrets = { ...portfileSecrets, [secret]: v };
                  }}
                  placeholder="required"
                  class="px-2.5 py-1.5 rounded-md bg-bg border border-border focus:border-accent/60 outline-none text-fg font-mono"
                />
              {/each}
            </div>
            <p class="text-[11px] text-fg-subtle mt-2">
              Values stay local to this machine. The file in the repo only carries the
              names, not the values.
            </p>
          {/if}
        </DashboardCard>
      {/if}

      {#if workspaceScan}
        <!-- Monorepo app picker: choosing one fills the fields below with that
             app's sub-directory so only it runs (no turbo --parallel fan-out). -->
        <DashboardCard title="Monorepo detected" flush>
          <p class="text-xs text-fg-muted">
            This looks like a <span class="font-mono">{workspaceScan.tool}</span>
            monorepo. Pick the app to run — PortBay adds it as its own project so
            the other apps in the repo don't start.
          </p>
          <div class="mt-3 space-y-1.5">
            {#each workspaceScan.apps as app (app.path)}
              <button
                type="button"
                onclick={() => chooseWorkspaceApp(app)}
                class="w-full flex items-center justify-between gap-3 rounded-md border px-3 py-2 text-left transition-colors
                       {workspaceChosenPath === app.path
                  ? 'border-accent bg-accent/10'
                  : 'border-border bg-bg/50 hover:border-border-strong'}"
              >
                <span class="min-w-0">
                  <span class="block text-sm text-fg truncate">{app.suggestedName}</span>
                  <span class="block text-[11px] text-fg-subtle font-mono truncate">
                    {app.relDir}
                  </span>
                </span>
                <span class="flex items-center gap-2 shrink-0">
                  <span class="text-[10px] uppercase tracking-wide text-fg-muted">
                    {typeLabel[app.kind]}
                  </span>
                  {#if workspaceChosenPath === app.path}
                    <Icon name="check" size={14} class="text-accent" />
                  {/if}
                </span>
              </button>
            {/each}
          </div>
          <label
            class="mt-3 flex items-start gap-2 text-[11px] text-fg-muted cursor-pointer"
            title="For apps whose dev script needs the monorepo's build pipeline (e.g. Turbo builds dependencies first). Runs from the repo root with a workspace filter instead of the app's own folder."
          >
            <input
              type="checkbox"
              bind:checked={workspaceFromRoot}
              onchange={onWorkspaceModeToggle}
              class="mt-0.5 accent-accent"
            />
            <span>
              Run from the repo root with a
              <span class="font-mono">{workspaceScan.tool}</span> filter
              <span class="text-fg-subtle"
                >— only if the app's dev script needs the monorepo build
                pipeline.</span
              >
            </span>
          </label>
          <button
            type="button"
            onclick={useWholeRepo}
            class="mt-2 block text-[11px] text-fg-subtle hover:text-fg underline-offset-2 hover:underline"
          >
            Not a monorepo? Use the whole folder instead.
          </button>
        </DashboardCard>
      {/if}

      <!-- L2: standard fields -->
      <DashboardCard title="Settings" flush>
        <div class="grid grid-cols-[120px,1fr] gap-x-4 gap-y-3 items-center text-sm">
          <label for="wizard-name" class="text-fg-muted">Name</label>
          <input
            id="wizard-name"
            type="text"
            bind:value={name}
            class="px-2.5 py-1.5 rounded-md bg-bg border border-border focus:border-accent/60 outline-none text-fg"
          />

          <label for="wizard-id" class="text-fg-muted">ID</label>
          <input
            id="wizard-id"
            type="text"
            bind:value={id}
            class="px-2.5 py-1.5 rounded-md bg-bg border border-border focus:border-accent/60 outline-none text-fg font-mono"
          />

          <label for="wizard-host" class="text-fg-muted">Hostname</label>
          <input
            id="wizard-host"
            type="text"
            bind:value={hostname}
            class="px-2.5 py-1.5 rounded-md bg-bg border border-border focus:border-accent/60 outline-none text-fg font-mono"
          />

          <label for="wizard-port" class="text-fg-muted">Port</label>
          <div class="min-w-0">
            <input
              id="wizard-port"
              type="number"
              min="1"
              max="65535"
              value={port ?? ""}
              oninput={(e) => {
                const v = (e.currentTarget as HTMLInputElement).value;
                port = v ? Number(v) : null;
                schedulePortCheck(port);
              }}
              class="px-2.5 py-1.5 rounded-md bg-bg border outline-none text-fg font-mono w-32
                     {portConflict
                ? 'border-status-crashed/60 focus:border-status-crashed'
                : 'border-border focus:border-accent/60'}"
            />
            {#if portConflict}
              <div
                class="mt-1.5 flex items-start gap-1.5 text-[11px] text-status-crashed"
                role="alert"
              >
                <Icon name="circle-alert" size={11} class="mt-0.5 shrink-0" />
                <span class="break-all">
                  Port {port} is in use by {portConflict}. Stop that process or
                  pick a different port.
                </span>
              </div>
            {/if}
          </div>

          <label for="wizard-type" class="text-fg-muted">Type</label>
          <select
            id="wizard-type"
            bind:value={kind}
            class="px-2.5 py-1.5 rounded-md bg-bg border border-border focus:border-accent/60 outline-none text-fg w-40"
          >
            {#each Object.entries(typeLabel) as [val, lbl] (val)}
              <option value={val}>{lbl}</option>
            {/each}
          </select>

          <label for="wizard-cmd" class="text-fg-muted self-start pt-1.5">
            Start command
          </label>
          <input
            id="wizard-cmd"
            type="text"
            bind:value={startCommand}
            placeholder={commandPlaceholder}
            class="px-2.5 py-1.5 rounded-md bg-bg border border-border focus:border-accent/60 outline-none text-fg font-mono"
          />

          {#if kind === "php"}
            <label for="wizard-docroot" class="text-fg-muted">Document root</label>
            <input
              id="wizard-docroot"
              type="text"
              bind:value={documentRoot}
              placeholder="public"
              class="px-2.5 py-1.5 rounded-md bg-bg border border-border focus:border-accent/60 outline-none text-fg font-mono"
            />

            <label for="wizard-php-version" class="text-fg-muted">PHP version</label>
            <input
              id="wizard-php-version"
              type="text"
              bind:value={phpVersion}
              placeholder="8.3"
              class="px-2.5 py-1.5 rounded-md bg-bg border border-border focus:border-accent/60 outline-none text-fg font-mono w-32"
            />

            <label for="wizard-web-server" class="text-fg-muted">Web server</label>
            <select
              id="wizard-web-server"
              bind:value={webServer}
              class="px-2.5 py-1.5 rounded-md bg-bg border border-border focus:border-accent/60 outline-none text-fg w-40"
            >
              <option value="caddy">Caddy</option>
              <option value="nginx">Nginx</option>
              <option value="apache">Apache</option>
            </select>
            {#if startCommand.trim()}
              <span></span>
              <p class="text-[11px] text-fg-subtle">
                Custom PHP commands are reverse-proxied by Caddy. Leave the
                start command empty to use the selected generated backend.
              </p>
            {/if}
          {/if}

          <span class="text-fg-muted">Options</span>
          <div class="flex items-center gap-4">
            <label class="flex items-center gap-1.5 text-xs cursor-pointer">
              <input type="checkbox" bind:checked={https} class="accent-accent" />
              HTTPS
            </label>
            <label class="flex items-center gap-1.5 text-xs cursor-pointer">
              <input
                type="checkbox"
                bind:checked={autoStart}
                class="accent-accent"
              />
              Auto-start
            </label>
          </div>
        </div>
      </DashboardCard>

      <!-- L3: raw config -->
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
            Edits here override the fields above on blur. Press Tab out of the
            box to apply.
          </p>
          <div class="mt-2">
            <CodeEditor
              value={rawDraft}
              language="json"
              oninput={(value) => (rawDraft = value)}
              onblur={syncFieldsFromRaw}
              minHeight={240}
            />
          </div>
        {/if}
      </DashboardCard>
    </div>

    <footer
      class="shrink-0 flex items-center justify-end gap-2 px-5 py-3 border-t border-border"
    >
      <button
        type="button"
        onclick={close}
        class="px-3 py-1.5 text-sm rounded-md text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
      >
        Cancel
      </button>
      <button
        type="button"
        onclick={commit}
        disabled={!path || submitting}
        class="inline-flex items-center gap-1.5 px-4 py-1.5 text-sm rounded-md text-status-running border border-status-running/40 hover:bg-status-running/10 hover:border-status-running/60 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
      >
        {#if submitting}
          <Icon name="refresh-cw" size={14} class="animate-spin" />
          Adding…
        {:else}
          <Icon name="plus" size={14} />
          Add
        {/if}
      </button>
    </footer>
  </aside>
{/if}
