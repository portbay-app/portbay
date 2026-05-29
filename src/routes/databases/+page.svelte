<!--
  /databases — owned database instances manager.

  Left rail: the user's provisioned instances + an "Add Database" CTA that
  opens the wizard. Right pane: the selected instance's status, lifecycle
  toolbar (Start/Stop/Restart/Reveal data folder/Client/Remove), connection details, paths,
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
  import { confirmDialog } from "$lib/stores/confirm.svelte";
  import { databases } from "$lib/stores/databases.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import type {
    BackupSnapshot,
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
    starting: "bg-status-starting/15 text-status-starting",
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

  function revealDataFolder() {
    if (!selected) return;
    // Reveals the instance's data directory in Finder. (An in-app log viewer
    // for daemons is a separate, future card; until then this is honest about
    // what it does rather than mislabelling itself "Logs".)
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

  // ── Per-database (schema) management ──────────────────────────────────────
  // Only the SQL engines expose a schema namespace, and the instance must be
  // running to query it. The list loads when the selected instance changes.
  const SQL_ENGINES = ["mysql", "mariadb", "postgres"];
  let schemas = $state<string[]>([]);
  let schemasLoading = $state<boolean>(false);
  let schemaError = $state<string | null>(null);
  let newSchema = $state<string>("");
  let schemaBusy = $state<boolean>(false);

  const canManageSchemas = $derived(
    !!selected && SQL_ENGINES.includes(selected.engine),
  );

  async function loadSchemas() {
    if (!selected || !canManageSchemas || selected.status !== "running") {
      schemas = [];
      return;
    }
    schemasLoading = true;
    schemaError = null;
    try {
      schemas = await safeInvoke<string[]>("list_instance_databases", {
        id: selected.id,
      });
    } catch {
      // Connection refused etc. — keep the section usable, surface inline.
      schemas = [];
      schemaError = "Couldn't list databases. Is the instance running?";
    } finally {
      schemasLoading = false;
    }
  }

  // Reload whenever the selected instance or its run state changes.
  $effect(() => {
    // Touch the deps so the effect re-runs on change.
    void selected?.id;
    void selected?.status;
    void loadSchemas();
  });

  async function addSchema() {
    const name = newSchema.trim();
    if (!selected || !name) return;
    schemaBusy = true;
    try {
      await safeInvoke("create_instance_database", { id: selected.id, name });
      newSchema = "";
      await loadSchemas();
    } catch {
      /* toast already pushed */
    } finally {
      schemaBusy = false;
    }
  }

  async function dropSchema(name: string) {
    if (!selected) return;
    const choice = await confirmDialog.open({
      title: `Drop database "${name}"?`,
      message:
        "This permanently deletes the database and all its tables/data. This cannot be undone.",
      destructive: true,
      actions: [{ label: "Drop database", value: "drop", tone: "destructive" }],
    });
    if (choice !== "drop") return;
    schemaBusy = true;
    try {
      await safeInvoke("drop_instance_database", { id: selected.id, name });
      await loadSchemas();
    } catch {
      /* toast already pushed */
    } finally {
      schemaBusy = false;
    }
  }

  // ── Backups & restore ─────────────────────────────────────────────────────
  // SQL engines only (mysqldump / pg_dumpall → restore via the client).
  let backups = $state<BackupSnapshot[]>([]);
  let backupBusy = $state<boolean>(false);
  const canBackup = $derived(
    !!selected && SQL_ENGINES.includes(selected.engine),
  );

  async function loadBackups() {
    if (!selected || !canBackup) {
      backups = [];
      return;
    }
    try {
      backups = await safeInvoke<BackupSnapshot[]>("list_database_backups", {
        id: selected.id,
      });
    } catch {
      backups = [];
    }
  }

  // Reload backups when the selected instance changes.
  $effect(() => {
    void selected?.id;
    void loadBackups();
  });

  async function backupNow() {
    if (!selected || backupBusy) return;
    backupBusy = true;
    try {
      await safeInvoke("backup_database_instance", { id: selected.id });
      errorBus.push({
        code: "DB_BACKUP_OK",
        whatHappened: `Backed up ${selected.name}.`,
        whyItMatters: "A new snapshot was written to the backups folder.",
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });
      await loadBackups();
    } catch {
      /* toast already pushed */
    } finally {
      backupBusy = false;
    }
  }

  async function restoreBackup(snapshotId: string) {
    if (!selected) return;
    const choice = await confirmDialog.open({
      title: "Restore this backup?",
      message:
        "Restoring replays the snapshot over the current data — anything created since this backup is lost. The instance should be running.",
      destructive: true,
      actions: [{ label: "Restore", value: "restore", tone: "destructive" }],
    });
    if (choice !== "restore") return;
    backupBusy = true;
    try {
      await safeInvoke("restore_database_backup", {
        id: selected.id,
        snapshotId,
      });
      errorBus.push({
        code: "DB_RESTORE_OK",
        whatHappened: `Restored ${selected.name} from backup.`,
        whyItMatters: "The snapshot's data replaced the current contents.",
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });
      await loadSchemas();
    } catch {
      /* toast already pushed */
    } finally {
      backupBusy = false;
    }
  }

  async function deleteBackup(snapshotId: string) {
    if (!selected) return;
    backupBusy = true;
    try {
      await safeInvoke("delete_database_backup", {
        id: selected.id,
        snapshotId,
      });
      await loadBackups();
    } catch {
      /* toast already pushed */
    } finally {
      backupBusy = false;
    }
  }

  function formatBytes(n: number): string {
    if (n <= 0) return "0 B";
    const units = ["B", "KB", "MB", "GB"];
    const i = Math.min(units.length - 1, Math.floor(Math.log(n) / Math.log(1024)));
    const v = n / 1024 ** i;
    return `${i === 0 || v >= 100 ? Math.round(v) : v.toFixed(1)} ${units[i]}`;
  }

  function formatWhen(ms: number): string {
    if (!ms) return "—";
    try {
      return new Date(ms).toLocaleString();
    } catch {
      return "—";
    }
  }

  async function removeInstance() {
    if (!selected) return;
    const choice = await confirmDialog.open({
      title: `Remove "${selected.name}"?`,
      message:
        "Deregister only removes it from PortBay and leaves the data files in place — you can re-add it later.\n\nDelete data + deregister also erases its data directory. That is irreversible.",
      destructive: true,
      actions: [
        { label: "Deregister only", value: "deregister" },
        {
          label: "Delete data + deregister",
          value: "delete",
          tone: "destructive",
        },
      ],
    });
    if (choice === null) return; // cancelled — touch nothing
    const deleteData = choice === "delete";
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
      class="sticky top-0 z-10 px-4 pt-4 pb-3 bg-surface/95 border-b border-border/40"
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
          {#if selected.fileBased}
            <!--
              File-based engines (SQLite) have no daemon: there's nothing to
              start, stop, or restart — the file is always available. Show that
              state plainly instead of dead lifecycle buttons.
            -->
            <span
              class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md
                     text-[12px] font-medium bg-status-running/15 text-status-running"
            >
              <Icon name="check" size={11} />
              Always available (file-based)
            </span>
          {:else}
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
          {/if}
          <button
            type="button"
            onclick={revealDataFolder}
            class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md
                   border border-border bg-surface text-[12px] text-fg-muted
                   hover:bg-surface-2 hover:text-fg transition-colors"
          >
            <Icon name="folder" size={11} />
            Reveal data folder
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

        <!-- Databases (schemas) — SQL engines only -->
        {#if canManageSchemas}
          <article class="bg-surface border border-border/70 rounded-2xl px-5 py-4">
            <header class="flex items-center justify-between gap-2 mb-3.5">
              <div class="flex items-center gap-2">
                <Icon name="database" size={13} class="text-fg-muted" />
                <h3 class="text-[13px] font-semibold text-fg">Databases</h3>
              </div>
              {#if schemasLoading}
                <Icon
                  name="refresh-cw"
                  size={12}
                  class="text-fg-subtle animate-spin"
                />
              {/if}
            </header>

            {#if selected.status !== "running"}
              <p class="text-[12px] text-fg-subtle">
                Start the instance to view and manage its databases.
              </p>
            {:else}
              <!-- Create -->
              <form
                class="flex items-stretch gap-1.5 mb-3"
                onsubmit={(e) => {
                  e.preventDefault();
                  void addSchema();
                }}
              >
                <input
                  type="text"
                  bind:value={newSchema}
                  placeholder="new_database_name"
                  aria-label="New database name"
                  class="flex-1 min-w-0 px-3 h-9 rounded-md bg-bg border border-border
                         text-[12px] font-mono text-fg placeholder:text-fg-subtle
                         focus:outline-none focus:ring-1 focus:ring-accent/50
                         focus:border-accent/40 transition-colors"
                />
                <button
                  type="submit"
                  disabled={schemaBusy || !newSchema.trim()}
                  class="shrink-0 inline-flex items-center gap-1.5 h-9 px-3 rounded-md
                         bg-accent text-on-accent text-[12px] font-medium
                         hover:brightness-110 active:brightness-95
                         disabled:opacity-50 disabled:cursor-not-allowed transition shadow-sm"
                >
                  <Icon name="plus" size={12} />
                  Create
                </button>
              </form>

              {#if schemaError}
                <p class="text-[11.5px] text-status-crashed mb-2">{schemaError}</p>
              {/if}

              {#if schemas.length === 0 && !schemasLoading}
                <p class="text-[12px] text-fg-subtle">
                  No user databases yet. Create one above.
                </p>
              {:else}
                <ul class="space-y-1.5">
                  {#each schemas as s (s)}
                    <li
                      class="flex items-center justify-between gap-2 px-3 py-2
                             rounded-md bg-surface-2/50 border border-border/50"
                    >
                      <span class="text-[12.5px] font-mono text-fg truncate">{s}</span>
                      <button
                        type="button"
                        onclick={() => dropSchema(s)}
                        disabled={schemaBusy}
                        title="Drop database"
                        aria-label="Drop database {s}"
                        class="shrink-0 p-1 rounded text-fg-subtle hover:text-status-crashed
                               hover:bg-status-crashed/10 transition-colors disabled:opacity-50"
                      >
                        <Icon name="x" size={13} />
                      </button>
                    </li>
                  {/each}
                </ul>
              {/if}
            {/if}
          </article>
        {/if}

        <!-- Backups — SQL engines only -->
        {#if canBackup}
          <article class="bg-surface border border-border/70 rounded-2xl px-5 py-4">
            <header class="flex items-center justify-between gap-2 mb-3.5">
              <div class="flex items-center gap-2">
                <Icon name="folder" size={13} class="text-fg-muted" />
                <h3 class="text-[13px] font-semibold text-fg">Backups</h3>
              </div>
              <button
                type="button"
                onclick={backupNow}
                disabled={backupBusy || selected.status !== "running"}
                title={selected.status !== "running"
                  ? "Start the instance to back it up"
                  : "Create a snapshot now"}
                class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md
                       bg-accent text-on-accent text-[12px] font-medium
                       hover:brightness-110 active:brightness-95
                       disabled:opacity-50 disabled:cursor-not-allowed transition shadow-sm"
              >
                {#if backupBusy}
                  <Icon name="refresh-cw" size={11} class="animate-spin" />
                {:else}
                  <Icon name="plus" size={11} />
                {/if}
                Back up now
              </button>
            </header>

            {#if backups.length === 0}
              <p class="text-[12px] text-fg-subtle">
                No backups yet. Snapshots are kept for {7} days and stored under the
                backups folder.
              </p>
            {:else}
              <ul class="space-y-1.5">
                {#each backups as b (b.id)}
                  <li
                    class="flex items-center justify-between gap-2 px-3 py-2
                           rounded-md bg-surface-2/50 border border-border/50"
                  >
                    <div class="min-w-0">
                      <p class="text-[12.5px] text-fg truncate">{formatWhen(b.createdAt)}</p>
                      <p class="text-[11px] font-mono text-fg-subtle">
                        {formatBytes(b.sizeBytes)}
                      </p>
                    </div>
                    <div class="flex shrink-0 gap-1.5">
                      <button
                        type="button"
                        onclick={() => restoreBackup(b.id)}
                        disabled={backupBusy}
                        class="h-7 px-2 text-[11px] rounded border border-accent/40
                               text-accent hover:bg-accent/10 disabled:opacity-50 transition-colors"
                      >
                        Restore
                      </button>
                      <button
                        type="button"
                        onclick={() => deleteBackup(b.id)}
                        disabled={backupBusy}
                        title="Delete backup"
                        aria-label="Delete backup"
                        class="h-7 w-7 grid place-items-center rounded border border-border
                               text-fg-subtle hover:text-status-crashed hover:bg-status-crashed/10
                               disabled:opacity-50 transition-colors"
                      >
                        <Icon name="x" size={12} />
                      </button>
                    </div>
                  </li>
                {/each}
              </ul>
            {/if}
          </article>
        {/if}

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
                <!-- Presentational popover; the buttons inside are the real
                     controls (Tab-reachable). stopPropagation keeps the
                     window click-away handler from closing it on an in-menu
                     click such as the scrollbar. -->
                <div
                  role="presentation"
                  onclick={(e) => e.stopPropagation()}
                  class="absolute right-0 top-8 z-30 w-56 max-h-64 overflow-y-auto
                         rounded-lg border border-border bg-surface shadow-2xl py-1"
                >
                  {#each linkableProjects as p (p.id)}
                    <button
                      type="button"
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
