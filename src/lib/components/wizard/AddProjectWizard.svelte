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
  import { projects } from "$lib/stores/projects.svelte";
  import { addProjectWizard } from "$lib/stores/wizard.svelte";
  import type { CommandError } from "$lib/types/error";
  import type { PortbayFile } from "$lib/types/portfile";
  import type { ProjectType, ProjectView } from "$lib/types/projects";
  import { typeLabel } from "$lib/types/projects";
  import type { DetectedProject } from "$lib/types/wizard";

  // ----- Form state -----
  let path = $state<string>("");
  let id = $state<string>("");
  let name = $state<string>("");
  let hostname = $state<string>("");
  let port = $state<number | null>(null);
  let startCommand = $state<string>("");
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

  function resetForm() {
    path = "";
    id = "";
    name = "";
    hostname = "";
    port = null;
    startCommand = "";
    kind = "custom";
    https = true;
    autoStart = false;
    rawConfigOpen = false;
    rawDraft = "";
    dropActive = false;
    dropHint = "";
    portfile = null;
    portfileSecrets = {};
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

  async function detect(folderPath: string) {
    path = folderPath;
    detecting = true;
    formError = null;
    portfile = null;
    portfileSecrets = {};
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
        kind = file.type;
        https = file.https;
        autoStart = file.autoStart;
        // id is derived on the backend from the folder's last component.
        const seg = folderPath.split("/").filter(Boolean).pop() ?? "imported";
        id = seg
          .toLowerCase()
          .replace(/[^a-z0-9]+/g, "-")
          .replace(/^-+|-+$/g, "");
        syncRawFromFields();
        return;
      }

      const det = await safeInvoke<DetectedProject>("detect_project", {
        path: folderPath,
      });
      id = det.suggestedId;
      name = det.suggestedName;
      hostname = det.suggestedHostname;
      port = det.suggestedPort;
      startCommand = det.suggestedStartCommand ?? "";
      kind = det.kind;
      syncRawFromFields();
    } catch (e) {
      // safeInvoke already toasted; surface inline too so the user knows
      // the form didn't autofill.
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
            https,
            autoStart,
          },
        });
      }
      // Refresh table to pick up the new row.
      await projects.refresh();
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
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="fixed inset-0 z-40 bg-bg/60 backdrop-blur-sm"
    onclick={close}
  ></div>

  <aside
    class="fixed inset-y-0 right-0 z-50 w-[600px] max-w-[90vw] bg-surface border-l border-border shadow-2xl flex flex-col"
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
          <input
            id="wizard-port"
            type="number"
            min="1"
            max="65535"
            value={port ?? ""}
            oninput={(e) => {
              const v = (e.currentTarget as HTMLInputElement).value;
              port = v ? Number(v) : null;
            }}
            class="px-2.5 py-1.5 rounded-md bg-bg border border-border focus:border-accent/60 outline-none text-fg font-mono w-32"
          />

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
            placeholder="pnpm dev"
            class="px-2.5 py-1.5 rounded-md bg-bg border border-border focus:border-accent/60 outline-none text-fg font-mono"
          />

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
