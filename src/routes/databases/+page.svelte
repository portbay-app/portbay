<!--
  /databases — owned database instances manager.

  Left rail: the user's provisioned instances + an "Add Database" CTA that
  opens the wizard. Right pane: the selected instance's status, lifecycle
  toolbar (Start/Stop/Restart/Logs/Client/Remove), connection details, paths,
  and project links. All instances are PortBay-supervised through Process
  Compose; lifecycle hits real IPC and refreshes status.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { revealItemInDir } from "@tauri-apps/plugin-opener";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import StatusDot from "$lib/components/atoms/StatusDot.svelte";
  import DatabaseMark from "$lib/components/databases/DatabaseMark.svelte";

  import { safeInvoke } from "$lib/ipc";
  import { errorBus } from "$lib/stores/errors.svelte";
  import { databases } from "$lib/stores/databases.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import type {
    DatabaseInstanceView,
    InstanceStatus,
  } from "$lib/types/databases";

  let query = $state<string>("");
  let copiedToken = $state<string | null>(null);
  let linkPickerOpen = $state<boolean>(false);

  onMount(() => {
    void databases.refresh();
    void projects.start();
  });

  const selected = $derived<DatabaseInstanceView | null>(databases.selected);

  const filtered = $derived.by(() => {
    const q = query.trim().toLowerCase();
    if (!q) return databases.value;
    return databases.value.filter(
      (d) =>
        d.name.toLowerCase().includes(q) ||
        d.engineLabel.toLowerCase().includes(q) ||
        d.engine.toLowerCase().includes(q),
    );
  });

  const statusToneClass: Record<InstanceStatus, string> = {
    running: "bg-status-running/15 text-status-running",
    stopped: "bg-fg-subtle/15 text-fg-subtle",
    starting: "bg-amber-500/15 text-amber-300",
    errored: "bg-status-crashed/15 text-status-crashed",
  };

  const statusText: Record<InstanceStatus, string> = {
    running: "Running",
    stopped: "Stopped",
    starting: "Starting",
    errored: "Error",
  };

  // Projects available to link (not already linked to the selected instance).
  const linkableProjects = $derived.by(() => {
    if (!selected) return [];
    const linked = new Set(selected.linkedProjects);
    return projects.value.filter((p) => !linked.has(p.id));
  });

  function projectName(id: string): string {
    return projects.value.find((p) => p.id === id)?.name ?? id;
  }

  async function copy(text: string) {
    if (!text) return;
    try {
      await navigator.clipboard.writeText(text);
      copiedToken = text;
      setTimeout(() => {
        if (copiedToken === text) copiedToken = null;
      }, 1500);
    } catch {
      /* no clipboard permission */
    }
  }

  async function reveal(path: string) {
    if (!path) return;
    try {
      await revealItemInDir(path);
    } catch {
      /* opener pushes a toast */
    }
  }

  function openLogs() {
    if (!selected) return;
    // PortBay writes each daemon's log to <logs>/db-<id>.log. Until the
    // in-app log viewer is wired for daemons, reveal the data dir so the
    // user can tail it; the dedicated viewer is a fast follow.
    reveal(selected.dataDir);
  }

  async function openClient() {
    if (!selected) return;
    databases.setBusy(selected.id, "client", true);
    try {
      await safeInvoke("open_database_client", { id: selected.id });
    } catch {
      /* toast already pushed */
    } finally {
      databases.setBusy(selected.id, "client", false);
    }
  }

  async function removeInstance() {
    if (!selected) return;
    const deleteData = confirm(
      `Remove "${selected.name}"?\n\nClick OK to also delete its data directory (irreversible), or Cancel to keep the data files and just deregister.`,
    );
    databases.setBusy(selected.id, "remove", true);
    try {
      await safeInvoke("remove_database_instance", {
        id: selected.id,
        deleteData,
      });
      errorBus.push({
        code: "DB_REMOVED",
        whatHappened: `${selected.name} removed.`,
        whyItMatters: deleteData
          ? "The data directory was deleted."
          : "The data directory was left in place.",
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });
      await databases.refresh();
    } catch {
      /* toast already pushed */
    } finally {
      databases.setBusy(selected.id, "remove", false);
    }
  }

  async function toggleAutoStart() {
    if (!selected) return;
    databases.setBusy(selected.id, "autostart", true);
    try {
      await safeInvoke("set_database_auto_start", {
        id: selected.id,
        autoStart: !selected.autoStart,
      });
      await databases.refresh();
    } catch {
      /* toast already pushed */
    } finally {
      databases.setBusy(selected.id, "autostart", false);
    }
  }

  async function linkProject(projectId: string) {
    if (!selected) return;
    linkPickerOpen = false;
    try {
      await safeInvoke("link_database_to_project", {
        id: selected.id,
        projectId,
      });
      errorBus.push({
        code: "DB_LINKED",
        whatHappened: `Linked to ${projectName(projectId)}.`,
        whyItMatters:
          "Connection env vars (DATABASE_URL, DB_*) are injected into that project on its next start.",
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });
      await databases.refresh();
    } catch {
      /* toast already pushed */
    }
  }

  async function unlinkProject(projectId: string) {
    if (!selected) return;
    try {
      await safeInvoke("unlink_database_from_project", {
        id: selected.id,
        projectId,
      });
      await databases.refresh();
    } catch {
      /* toast already pushed */
    }
  }

  function onWindowClick() {
    if (linkPickerOpen) linkPickerOpen = false;
  }
</script>

<svelte:window onclick={onWindowClick} />

<div class="h-full flex">
  <!-- Left rail -->
  <aside
    class="w-[300px] shrink-0 border-r border-border bg-surface/40
           overflow-y-auto flex flex-col"
    aria-label="Database instances"
  >
    <header
      class="sticky top-0 z-10 px-4 pt-4 pb-3 bg-surface/40 backdrop-blur-sm border-b border-border/40"
    >
      <h2 class="text-[13px] font-semibold text-fg mb-2.5">Databases</h2>
      <div class="relative">
        <Icon
          name="search"
          size={12}
          class="absolute left-2.5 top-1/2 -translate-y-1/2 text-fg-subtle pointer-events-none"
        />
        <input
          type="search"
          bind:value={query}
          placeholder="Search instances…"
          aria-label="Search database instances"
          class="w-full pl-7 pr-2 h-8 rounded-md bg-surface/80 border border-border/60
                 text-[12px] text-fg placeholder:text-fg-subtle
                 focus:outline-none focus:ring-1 focus:ring-accent/60
                 focus:border-accent/40 transition-colors"
        />
      </div>
      <button
        type="button"
        onclick={() => databases.showWizard()}
        class="mt-2.5 w-full inline-flex items-center justify-center gap-2 h-9
               rounded-lg bg-accent text-on-accent text-[12.5px] font-medium
               hover:brightness-110 active:brightness-95 transition shadow-sm"
      >
        <Icon name="plus" size={13} />
        Add Database
      </button>
    </header>

    <div class="px-2 py-2 space-y-1 flex-1 min-h-0">
      {#if databases.loading && databases.value.length === 0}
        <p class="px-2 py-4 text-[12px] text-fg-subtle">Loading…</p>
      {:else if databases.value.length === 0}
        <div class="px-3 py-8 text-center">
          <Icon name="database" size={22} class="text-fg-subtle mx-auto" />
          <p class="mt-2.5 text-[12px] text-fg-muted">No databases yet.</p>
          <p class="mt-1 text-[11px] text-fg-subtle leading-relaxed">
            Click “Add Database” to provision a local MySQL, PostgreSQL,
            Redis, or MongoDB instance.
          </p>
        </div>
      {:else if filtered.length === 0}
        <p class="px-2 py-4 text-[12px] text-fg-subtle">
          No instances match “{query}”.
        </p>
      {:else}
        {#each filtered as db (db.id)}
          {@const isActive = databases.selectedId === db.id}
          <div
            role="button"
            tabindex="0"
            onclick={() => databases.select(db.id)}
            onkeydown={(e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                databases.select(db.id);
              }
            }}
            class="w-full flex items-center gap-3 px-2.5 py-2 rounded-lg
                   text-left transition-colors cursor-pointer
                   focus-visible:outline-none focus-visible:ring-2
                   focus-visible:ring-accent/40
                   {isActive
              ? 'bg-accent/10 ring-1 ring-inset ring-accent/40'
              : 'hover:bg-surface-2/60'}"
          >
            <DatabaseMark id={db.engine} size={32} class="shrink-0" />
            <div class="min-w-0 flex-1 leading-tight">
              <div class="flex items-center gap-1.5">
                <span
                  class="inline-block w-1.5 h-1.5 rounded-full shrink-0
                         {db.status === 'running'
                    ? 'bg-status-running'
                    : db.status === 'errored'
                      ? 'bg-status-crashed'
                      : 'bg-fg-subtle/60'}"
                  aria-hidden="true"
                ></span>
                <span class="text-[13px] font-semibold text-fg truncate">
                  {db.name}
                </span>
              </div>
              <p class="text-[11px] font-mono text-fg-subtle truncate">
                {db.engineLabel}{db.version ? ` ${db.version}` : ""} · :{db.port}
              </p>
            </div>
            <span
              class="shrink-0 inline-flex items-center px-1.5 py-0.5 rounded-md
                     text-[10.5px] font-medium {statusToneClass[db.status]}"
            >
              {statusText[db.status]}
            </span>
          </div>
        {/each}
      {/if}
    </div>
  </aside>

  <!-- Right pane -->
  <section class="flex-1 min-w-0 overflow-y-auto">
    {#if !selected}
      <div class="h-full flex items-center justify-center">
        <div class="text-center max-w-sm px-6">
          <Icon name="database" size={28} class="text-fg-subtle mx-auto" />
          <p class="mt-3 text-[13px] text-fg-muted">
            {databases.value.length === 0
              ? "Add a database to get started."
              : "Select an instance from the sidebar."}
          </p>
        </div>
      </div>
    {:else}
      {@const busyLifecycle =
        databases.isBusy(selected.id, "start") ||
        databases.isBusy(selected.id, "stop") ||
        databases.isBusy(selected.id, "restart")}
      <!-- Header -->
      <header
        class="px-8 pt-7 pb-5 border-b border-border/70
               flex items-start justify-between gap-4 flex-wrap"
      >
        <div class="min-w-0 flex items-start gap-3">
          <DatabaseMark id={selected.engine} size={34} class="shrink-0 mt-0.5" />
          <div class="min-w-0">
            <h1
              class="text-[20px] font-semibold tracking-tight text-fg
                     flex items-center gap-2.5 flex-wrap"
            >
              {selected.name}
              <span
                class="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-full
                       text-[11px] font-medium {statusToneClass[selected.status]}"
              >
                <StatusDot
                  status={selected.status === "running" ? "running" : "stopped"}
                  size="sm"
                />
                {statusText[selected.status]}
              </span>
            </h1>
            <p class="mt-1 text-[12px] text-fg-muted">
              {selected.engineLabel}{selected.version
                ? ` ${selected.version}`
                : ""} · port {selected.port}
            </p>
            {#if !selected.binaryAvailable}
              <p class="mt-1 text-[11px] text-status-crashed">
                The {selected.engineLabel} binary is no longer on this machine —
                reinstall the engine to run this instance.
              </p>
            {/if}
          </div>
        </div>

        <!-- Action toolbar -->
        <div class="flex items-center gap-1.5 shrink-0 flex-wrap">
          {#if selected.status === "running" || selected.status === "starting"}
            <button
              type="button"
              onclick={() => databases.action(selected.id, "stop")}
              disabled={busyLifecycle}
              class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md
                     text-[12px] font-medium text-on-accent bg-status-crashed
                     hover:brightness-110 active:brightness-95
                     disabled:opacity-50 disabled:cursor-not-allowed transition shadow-sm"
            >
              {#if databases.isBusy(selected.id, "stop")}
                <Icon name="refresh-cw" size={11} class="animate-spin" />
              {:else}
                <Icon name="square" size={11} class="fill-current" />
              {/if}
              Stop
            </button>
          {:else}
            <button
              type="button"
              onclick={() => databases.action(selected.id, "start")}
              disabled={busyLifecycle || !selected.binaryAvailable}
              class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md
                     text-[12px] font-medium text-on-accent bg-status-running
                     hover:brightness-110 active:brightness-95
                     disabled:opacity-50 disabled:cursor-not-allowed transition shadow-sm"
            >
              {#if databases.isBusy(selected.id, "start")}
                <Icon name="refresh-cw" size={11} class="animate-spin" />
              {:else}
                <Icon name="play" size={11} class="fill-current" />
              {/if}
              Start
            </button>
          {/if}

          <button
            type="button"
            onclick={() => databases.action(selected.id, "restart")}
            disabled={busyLifecycle || selected.status !== "running"}
            class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md
                   border border-border bg-surface text-[12px] text-fg-muted
                   hover:bg-surface-2 hover:text-fg transition-colors
                   disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {#if databases.isBusy(selected.id, "restart")}
              <Icon name="refresh-cw" size={11} class="animate-spin" />
            {:else}
              <Icon name="refresh-cw" size={11} />
            {/if}
            Restart
          </button>
          <button
            type="button"
            onclick={openLogs}
            class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md
                   border border-border bg-surface text-[12px] text-fg-muted
                   hover:bg-surface-2 hover:text-fg transition-colors"
          >
            <Icon name="file-text" size={11} />
            Logs
          </button>
          <button
            type="button"
            onclick={openClient}
            disabled={databases.isBusy(selected.id, "client")}
            class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md
                   border border-border bg-surface text-[12px] text-fg-muted
                   hover:bg-surface-2 hover:text-fg transition-colors
                   disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {#if databases.isBusy(selected.id, "client")}
              <Icon name="refresh-cw" size={11} class="animate-spin" />
            {:else}
              <Icon name="terminal" size={11} />
            {/if}
            Client
          </button>
          <button
            type="button"
            onclick={removeInstance}
            disabled={databases.anyBusy(selected.id)}
            class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md
                   border border-status-crashed/40 text-status-crashed
                   hover:bg-status-crashed/10 transition-colors text-[12px]
                   disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {#if databases.isBusy(selected.id, "remove")}
              <Icon name="refresh-cw" size={11} class="animate-spin" />
            {:else}
              <Icon name="x" size={11} />
            {/if}
            Remove
          </button>
        </div>
      </header>

      <div class="px-8 py-6 space-y-4">
        <!-- Connection -->
        <article class="bg-surface border border-border/70 rounded-2xl px-5 py-4">
          <header class="flex items-center gap-2 mb-3.5">
            <Icon name="link" size={13} class="text-fg-muted" />
            <h3 class="text-[13px] font-semibold text-fg">Connection</h3>
          </header>
          <div class="space-y-4">
            {#snippet field(label: string, value: string, isPath = false)}
              <div class="min-w-0">
                <span class="block text-[11px] font-medium text-fg-muted mb-1.5">
                  {label}
                </span>
                <div class="flex items-stretch gap-1.5">
                  <input
                    type="text"
                    value={value}
                    readonly
                    title={value}
                    class="flex-1 min-w-0 px-3 h-9 rounded-md bg-surface-2/60
                           border border-border/60 text-[12px] font-mono text-fg
                           focus:outline-none focus:ring-1 focus:ring-accent/50
                           selection:bg-accent/30"
                  />
                  <button
                    type="button"
                    onclick={() => copy(value)}
                    disabled={!value}
                    title={copiedToken === value ? "Copied!" : "Copy"}
                    aria-label="Copy {label}"
                    class="shrink-0 inline-flex items-center justify-center w-9 h-9
                           rounded-md border border-border/60 text-fg-muted
                           hover:text-fg hover:bg-surface-2 transition-colors
                           disabled:opacity-40 {copiedToken === value
                      ? 'text-status-running'
                      : ''}"
                  >
                    <Icon name={copiedToken === value ? "check" : "link"} size={13} />
                  </button>
                  {#if isPath && value}
                    <button
                      type="button"
                      onclick={() => reveal(value)}
                      title="Reveal in Finder"
                      aria-label="Reveal {label}"
                      class="shrink-0 inline-flex items-center justify-center w-9 h-9
                             rounded-md border border-accent/40 text-accent
                             hover:bg-accent/10 transition-colors"
                    >
                      <Icon name="folder" size={13} />
                    </button>
                  {/if}
                </div>
              </div>
            {/snippet}

            <div class="grid grid-cols-1 md:grid-cols-2 gap-x-5 gap-y-4">
              {@render field("Connection URL", selected.connectionUrl)}
              {@render field("Host", "127.0.0.1")}
              {@render field("Port", selected.port.toString())}
              {@render field(
                "Account",
                selected.account || "(none — no auth by default)",
              )}
            </div>
          </div>
        </article>

        <!-- Paths / Storage -->
        <article class="bg-surface border border-border/70 rounded-2xl px-5 py-4">
          <header class="flex items-center gap-2 mb-3.5">
            <Icon name="folder" size={13} class="text-fg-muted" />
            <h3 class="text-[13px] font-semibold text-fg">Paths / Storage</h3>
          </header>
          <div class="space-y-4">
            {#snippet pathField(label: string, value: string)}
              <div class="min-w-0">
                <span class="block text-[11px] font-medium text-fg-muted mb-1.5">
                  {label}
                </span>
                <div class="flex items-stretch gap-1.5">
                  <input
                    type="text"
                    value={value}
                    readonly
                    title={value}
                    placeholder="—"
                    class="flex-1 min-w-0 px-3 h-9 rounded-md bg-surface-2/60
                           border border-border/60 text-[12px] font-mono text-fg
                           focus:outline-none focus:ring-1 focus:ring-accent/50
                           selection:bg-accent/30"
                  />
                  <button
                    type="button"
                    onclick={() => copy(value)}
                    disabled={!value}
                    title={copiedToken === value ? "Copied!" : "Copy"}
                    aria-label="Copy {label}"
                    class="shrink-0 inline-flex items-center justify-center w-9 h-9
                           rounded-md border border-border/60 text-fg-muted
                           hover:text-fg hover:bg-surface-2 transition-colors
                           disabled:opacity-40 {copiedToken === value
                      ? 'text-status-running'
                      : ''}"
                  >
                    <Icon name={copiedToken === value ? "check" : "link"} size={13} />
                  </button>
                  {#if value}
                    <button
                      type="button"
                      onclick={() => reveal(value)}
                      title="Reveal in Finder"
                      aria-label="Reveal {label}"
                      class="shrink-0 inline-flex items-center justify-center w-9 h-9
                             rounded-md border border-accent/40 text-accent
                             hover:bg-accent/10 transition-colors"
                    >
                      <Icon name="folder" size={13} />
                    </button>
                  {/if}
                </div>
              </div>
            {/snippet}

            {@render pathField("Data directory", selected.dataDir)}
            {#if selected.configPath}
              {@render pathField("Config file", selected.configPath)}
            {/if}
            {#if selected.socketPath}
              {@render pathField("Socket", selected.socketPath)}
            {/if}
          </div>
        </article>

        <!-- Linked projects -->
        <article class="bg-surface border border-border/70 rounded-2xl px-5 py-4">
          <header class="flex items-center justify-between gap-2 mb-3.5">
            <div class="flex items-center gap-2">
              <Icon name="layers" size={13} class="text-fg-muted" />
              <h3 class="text-[13px] font-semibold text-fg">Linked projects</h3>
            </div>
            <div class="relative">
              <button
                type="button"
                onclick={(e) => {
                  e.stopPropagation();
                  linkPickerOpen = !linkPickerOpen;
                }}
                disabled={linkableProjects.length === 0}
                class="inline-flex items-center gap-1.5 h-7 px-2.5 rounded-md
                       border border-border bg-surface text-[11.5px] text-fg-muted
                       hover:bg-surface-2 hover:text-fg transition-colors
                       disabled:opacity-50 disabled:cursor-not-allowed"
                title={linkableProjects.length === 0
                  ? "No more projects to link"
                  : "Link a project"}
              >
                <Icon name="plus" size={11} />
                Link project
              </button>
              {#if linkPickerOpen}
                <!-- svelte-ignore a11y_interactive_supports_focus -->
                <!-- svelte-ignore a11y_click_events_have_key_events -->
                <div
                  role="menu"
                  onclick={(e) => e.stopPropagation()}
                  class="absolute right-0 top-8 z-30 w-56 max-h-64 overflow-y-auto
                         rounded-lg border border-border bg-surface shadow-2xl py-1"
                >
                  {#each linkableProjects as p (p.id)}
                    <button
                      type="button"
                      role="menuitem"
                      onclick={() => linkProject(p.id)}
                      class="w-full text-left px-3 py-1.5 text-[12px]
                             text-fg-muted hover:bg-surface-2 hover:text-fg
                             transition-colors truncate"
                    >
                      {p.name}
                    </button>
                  {/each}
                </div>
              {/if}
            </div>
          </header>

          {#if selected.linkedProjects.length === 0}
            <p class="text-[12px] text-fg-subtle">
              Not linked to any project. Linking injects
              <span class="font-mono text-fg-muted">DATABASE_URL</span> and
              <span class="font-mono text-fg-muted">DB_*</span> env vars into
              the project when it starts.
            </p>
          {:else}
            <ul class="space-y-1.5">
              {#each selected.linkedProjects as pid (pid)}
                <li
                  class="flex items-center justify-between gap-2 px-3 py-2
                         rounded-md bg-surface-2/50 border border-border/50"
                >
                  <span class="text-[12.5px] text-fg truncate">
                    {projectName(pid)}
                  </span>
                  <button
                    type="button"
                    onclick={() => unlinkProject(pid)}
                    title="Unlink"
                    aria-label="Unlink {projectName(pid)}"
                    class="shrink-0 p-1 rounded text-fg-subtle hover:text-status-crashed
                           hover:bg-status-crashed/10 transition-colors"
                  >
                    <Icon name="x" size={13} />
                  </button>
                </li>
              {/each}
            </ul>
          {/if}
        </article>

        <!-- Footer: auto-start -->
        <footer class="flex items-center justify-between gap-3 pt-1">
          <label class="flex items-center gap-2.5 cursor-pointer select-none">
            <input
              type="checkbox"
              checked={selected.autoStart}
              onchange={toggleAutoStart}
              disabled={databases.isBusy(selected.id, "autostart")}
              class="accent-accent"
            />
            <span class="text-[12.5px] text-fg">
              Start automatically when PortBay launches
            </span>
          </label>
          <button
            type="button"
            onclick={() => selected.dataDir && reveal(selected.dataDir)}
            class="inline-flex items-center gap-1.5 h-9 px-3 rounded-md
                   border border-border bg-surface text-[12px] text-fg-muted
                   hover:bg-surface-2 hover:text-fg transition-colors"
          >
            <Icon name="folder" size={12} />
            Open Data Folder
          </button>
        </footer>
      </div>
    {/if}
  </section>
</div>
