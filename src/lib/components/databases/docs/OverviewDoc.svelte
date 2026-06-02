<script lang="ts">
  /**
   * OverviewDoc — instance management panel, rendered in the Overview tab.
   * Contains: status + lifecycle toolbar, connection details + copy,
   * Databases (schemas) card, Backups card (list/create/restore/delete),
   * Paths card, Linked projects (link/unlink), auto-start toggle.
   * Quick-action buttons at top: Browse tables (ERD), New query, Schema diagram.
   */
  import { onMount } from "svelte";

  import Icon, { type IconName } from "$lib/components/atoms/Icon.svelte";
  import StatusDot from "$lib/components/atoms/StatusDot.svelte";
  import DatabaseMark from "$lib/components/databases/DatabaseMark.svelte";

  import { safeInvoke } from "$lib/ipc";
  import { errorBus } from "$lib/stores/errors.svelte";
  import { confirmDialog } from "$lib/stores/confirm.svelte";
  import { databases } from "$lib/stores/databases.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import { dbWorkspace } from "$lib/stores/dbWorkspace.svelte";
  import type {
    BackupSnapshot,
    DatabaseInstanceView,
    InstanceStatus,
  } from "$lib/types/databases";

  interface Props {
    instance: DatabaseInstanceView;
  }

  let { instance }: Props = $props();

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

  // ── Clipboard ──────────────────────────────────────────────────────────────
  let copiedToken = $state<string | null>(null);

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

  // ── Finder reveal ──────────────────────────────────────────────────────────
  async function reveal(path: string) {
    if (!path) return;
    try {
      await safeInvoke("reveal_in_finder", { path });
    } catch {
      /* safeInvoke already pushed the toast */
    }
  }

  function revealDataFolder() {
    reveal(instance.dataDir);
  }

  // ── Quick actions ────────────────────────────────────────────────────────
  // "Browse tables" needs a concrete table to open (the workspace has no
  // "all tables" tab), so load the schema and land on the first one. If the
  // database has no tables yet, fall back to the schema diagram so the tile
  // still does something useful.
  async function browseTables() {
    const schema = await dbWorkspace.loadSchema(instance.id);
    const first = schema?.tables?.[0];
    if (first) dbWorkspace.openTable(instance.id, first.schema ?? null, first.name);
    else dbWorkspace.openErd(instance.id);
  }

  type QuickAction = { label: string; hint: string; icon: IconName; run: () => void };
  const quickActions: QuickAction[] = [
    { label: "Browse tables", hint: "Open the data browser", icon: "grid-2x2", run: browseTables },
    {
      label: "New query",
      hint: "Run SQL in a scratchpad",
      icon: "terminal",
      run: () => dbWorkspace.openQuery(instance.id),
    },
    {
      label: "Schema diagram",
      hint: "Tables and relations",
      icon: "share",
      run: () => dbWorkspace.openErd(instance.id),
    },
    {
      label: "Query builder",
      hint: "Compose SQL visually",
      icon: "layers",
      run: () => dbWorkspace.openBuilder(instance.id),
    },
  ];

  // ── Lifecycle actions ──────────────────────────────────────────────────────
  const busyLifecycle = $derived(
    databases.isBusy(instance.id, "start") ||
      databases.isBusy(instance.id, "stop") ||
      databases.isBusy(instance.id, "restart"),
  );

  async function openClient() {
    databases.setBusy(instance.id, "client", true);
    try {
      await safeInvoke("open_database_client", { id: instance.id });
    } catch {
      /* toast already pushed */
    } finally {
      databases.setBusy(instance.id, "client", false);
    }
  }

  async function removeInstance() {
    const choice = await confirmDialog.open({
      title: `Remove "${instance.name}"?`,
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
    if (choice === null) return;
    const deleteData = choice === "delete";
    databases.setBusy(instance.id, "remove", true);
    try {
      await safeInvoke("remove_database_instance", {
        id: instance.id,
        deleteData,
      });
      errorBus.push({
        code: "DB_REMOVED",
        category: "infrastructure",
        whatHappened: `${instance.name} removed.`,
        whyItMatters: deleteData
          ? "The data directory was deleted."
          : "The data directory was left in place.",
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });
      dbWorkspace.closeInstance(instance.id);
      await databases.refresh();
    } catch {
      /* toast already pushed */
    } finally {
      databases.setBusy(instance.id, "remove", false);
    }
  }

  async function toggleAutoStart() {
    databases.setBusy(instance.id, "autostart", true);
    try {
      await safeInvoke("set_database_auto_start", {
        id: instance.id,
        autoStart: !instance.autoStart,
      });
      await databases.refresh();
    } catch {
      /* toast already pushed */
    } finally {
      databases.setBusy(instance.id, "autostart", false);
    }
  }

  // ── Schemas (Databases card) ───────────────────────────────────────────────
  const SQL_ENGINES = ["mysql", "mariadb", "postgres"];
  const canManageSchemas = $derived(SQL_ENGINES.includes(instance.engine));

  let schemas = $state<string[]>([]);
  let schemasLoading = $state(false);
  let schemaError = $state<string | null>(null);
  let newSchema = $state("");
  let schemaBusy = $state(false);

  async function loadSchemas() {
    if (!canManageSchemas || instance.status !== "running") {
      schemas = [];
      return;
    }
    schemasLoading = true;
    schemaError = null;
    try {
      schemas = await safeInvoke<string[]>("list_instance_databases", {
        id: instance.id,
      });
    } catch {
      schemas = [];
      schemaError = "Couldn't list databases. Is the instance running?";
    } finally {
      schemasLoading = false;
    }
  }

  $effect(() => {
    void instance.id;
    void instance.status;
    void loadSchemas();
  });

  async function addSchema() {
    const name = newSchema.trim();
    if (!name) return;
    schemaBusy = true;
    try {
      await safeInvoke("create_instance_database", { id: instance.id, name });
      newSchema = "";
      await loadSchemas();
    } catch {
      /* toast already pushed */
    } finally {
      schemaBusy = false;
    }
  }

  async function dropSchema(name: string) {
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
      await safeInvoke("drop_instance_database", { id: instance.id, name });
      await loadSchemas();
    } catch {
      /* toast already pushed */
    } finally {
      schemaBusy = false;
    }
  }

  // ── Backups ────────────────────────────────────────────────────────────────
  const canBackup = $derived(SQL_ENGINES.includes(instance.engine));

  let backups = $state<BackupSnapshot[]>([]);
  let backupBusy = $state(false);

  async function loadBackups() {
    if (!canBackup) {
      backups = [];
      return;
    }
    try {
      backups = await safeInvoke<BackupSnapshot[]>("list_database_backups", {
        id: instance.id,
      });
    } catch {
      backups = [];
    }
  }

  $effect(() => {
    void instance.id;
    void loadBackups();
  });

  async function backupNow() {
    if (backupBusy) return;
    backupBusy = true;
    try {
      await safeInvoke("backup_database_instance", { id: instance.id });
      errorBus.push({
        code: "DB_BACKUP_OK",
        category: "infrastructure",
        whatHappened: `Backed up ${instance.name}.`,
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
        id: instance.id,
        snapshotId,
      });
      errorBus.push({
        code: "DB_RESTORE_OK",
        category: "infrastructure",
        whatHappened: `Restored ${instance.name} from backup.`,
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
    backupBusy = true;
    try {
      await safeInvoke("delete_database_backup", {
        id: instance.id,
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

  // ── Linked projects ────────────────────────────────────────────────────────
  let linkPickerOpen = $state(false);

  const linkableProjects = $derived.by(() => {
    const linked = new Set(instance.linkedProjects);
    return projects.value.filter((p) => !linked.has(p.id));
  });

  function projectName(id: string): string {
    return projects.value.find((p) => p.id === id)?.name ?? id;
  }

  async function linkProject(projectId: string) {
    linkPickerOpen = false;
    try {
      await safeInvoke("link_database_to_project", {
        id: instance.id,
        projectId,
      });
      errorBus.push({
        code: "DB_LINKED",
        category: "infrastructure",
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
    try {
      await safeInvoke("unlink_database_from_project", {
        id: instance.id,
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

  onMount(() => {
    void projects.start();
  });
</script>

<svelte:window onclick={onWindowClick} />

<div class="h-full overflow-y-auto">
  <div class="px-8 py-6 space-y-4">

    <!-- Header: instance identity + status + lifecycle toolbar -->
    <header
      class="pb-5 border-b border-border/70
             flex items-start justify-between gap-4 flex-wrap"
    >
      <div class="min-w-0 flex items-start gap-3">
        <DatabaseMark id={instance.engine} size={34} class="shrink-0 mt-0.5" />
        <div class="min-w-0">
          <h1
            class="text-[20px] font-semibold tracking-tight text-fg
                   flex items-center gap-2.5 flex-wrap"
          >
            {instance.name}
            <span
              class="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-full
                     text-[11px] font-medium {statusToneClass[instance.status]}"
              title={instance.fileBased
                ? "Always available — SQLite is file-based, so there's no daemon to start or stop."
                : undefined}
            >
              <StatusDot
                status={instance.status === "running" ? "running" : "stopped"}
                size="sm"
              />
              {statusText[instance.status]}
            </span>
          </h1>
          <p class="mt-1 text-[12px] text-fg-muted">
            {instance.engineLabel}{instance.version ? ` ${instance.version}` : ""} · port {instance.port}
          </p>
          {#if !instance.binaryAvailable}
            <p class="mt-1 text-[11px] text-status-crashed">
              The {instance.engineLabel} binary is no longer on this machine —
              reinstall the engine to run this instance.
            </p>
          {/if}
        </div>
      </div>

      <!-- Lifecycle toolbar -->
      <div class="flex items-center gap-1.5 shrink-0 flex-wrap">
        {#if !instance.fileBased}
          {#if instance.status === "running" || instance.status === "starting"}
            <button
              type="button"
              onclick={() => databases.action(instance.id, "stop")}
              disabled={busyLifecycle}
              class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md
                     text-[12px] font-medium text-on-accent bg-status-crashed
                     hover:brightness-110 active:brightness-95
                     disabled:opacity-50 disabled:cursor-not-allowed transition shadow-sm"
            >
              {#if databases.isBusy(instance.id, "stop")}
                <Icon name="refresh-cw" size={11} class="animate-spin" />
              {:else}
                <Icon name="square" size={11} class="fill-current" />
              {/if}
              Stop
            </button>
          {:else}
            <button
              type="button"
              onclick={() => databases.action(instance.id, "start")}
              disabled={busyLifecycle || !instance.binaryAvailable}
              class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md
                     text-[12px] font-medium text-on-accent bg-status-running
                     hover:brightness-110 active:brightness-95
                     disabled:opacity-50 disabled:cursor-not-allowed transition shadow-sm"
            >
              {#if databases.isBusy(instance.id, "start")}
                <Icon name="refresh-cw" size={11} class="animate-spin" />
              {:else}
                <Icon name="play" size={11} class="fill-current" />
              {/if}
              Start
            </button>
          {/if}

          <button
            type="button"
            onclick={() => databases.action(instance.id, "restart")}
            disabled={busyLifecycle || instance.status !== "running"}
            class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md
                   border border-border bg-surface text-[12px] text-fg-muted
                   hover:bg-surface-2 hover:text-fg transition-colors
                   disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {#if databases.isBusy(instance.id, "restart")}
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
          disabled={databases.isBusy(instance.id, "client")}
          class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md
                 border border-border bg-surface text-[12px] text-fg-muted
                 hover:bg-surface-2 hover:text-fg transition-colors
                 disabled:opacity-50 disabled:cursor-not-allowed"
        >
          {#if databases.isBusy(instance.id, "client")}
            <Icon name="refresh-cw" size={11} class="animate-spin" />
          {:else}
            <Icon name="terminal" size={11} />
          {/if}
          Terminal client
        </button>

        <button
          type="button"
          onclick={removeInstance}
          disabled={databases.anyBusy(instance.id)}
          class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md
                 border border-status-crashed/40 text-status-crashed
                 hover:bg-status-crashed/10 transition-colors text-[12px]
                 disabled:opacity-50 disabled:cursor-not-allowed"
        >
          {#if databases.isBusy(instance.id, "remove")}
            <Icon name="refresh-cw" size={11} class="animate-spin" />
          {:else}
            <Icon name="x" size={11} />
          {/if}
          Remove
        </button>
      </div>
    </header>

    <!-- Quick actions — four launcher tiles into the workspace's main views. -->
    <div class="grid grid-cols-2 lg:grid-cols-4 gap-2.5">
      {#each quickActions as qa (qa.label)}
        <button
          type="button"
          onclick={qa.run}
          class="group/qa relative flex items-center gap-3 rounded-xl border border-border/70
                 bg-surface px-3.5 py-3 text-left
                 transition-[transform,border-color,background-color,box-shadow] duration-200 ease-out
                 hover:-translate-y-0.5 hover:border-accent/40 hover:bg-surface-2/60
                 hover:shadow-[0_10px_28px_-16px_rgba(0,0,0,0.55)]
                 active:translate-y-0 active:scale-[0.98]
                 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-accent/40"
        >
          <span
            class="inline-flex h-9 w-9 shrink-0 items-center justify-center rounded-lg
                   bg-surface-2 text-fg-muted ring-1 ring-inset ring-border/60
                   transition-colors group-hover/qa:bg-accent/10 group-hover/qa:text-accent
                   group-hover/qa:ring-accent/30"
          >
            <Icon
              name={qa.icon}
              size={16}
              class="transition-transform duration-200 group-hover/qa:-rotate-12"
            />
          </span>
          <span class="min-w-0">
            <span class="block text-[12.5px] font-medium text-fg leading-tight">{qa.label}</span>
            <span class="block text-[11px] text-fg-subtle leading-snug truncate">{qa.hint}</span>
          </span>
        </button>
      {/each}
    </div>

    <!-- Connection details -->
    <article class="bg-surface border border-border/70 rounded-2xl px-5 py-4">
      <header class="flex items-center gap-2 mb-3.5">
        <Icon name="link" size={13} class="text-fg-muted" />
        <h3 class="text-[13px] font-semibold text-fg">Connection</h3>
      </header>
      <div class="grid grid-cols-1 md:grid-cols-2 gap-x-5 gap-y-4">
        {#snippet connField(label: string, value: string, isPath = false)}
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

        {@render connField("Connection URL", instance.connectionUrl)}
        {@render connField("Host", "127.0.0.1")}
        {@render connField("Port", instance.port.toString())}
        {@render connField("Account", instance.account || "(none — no auth by default)")}
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
            <Icon name="refresh-cw" size={12} class="text-fg-subtle animate-spin" />
          {/if}
        </header>

        {#if instance.status !== "running"}
          <p class="text-[12px] text-fg-subtle">
            Start the instance to view and manage its databases.
          </p>
        {:else}
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
            disabled={backupBusy || instance.status !== "running"}
            title={instance.status !== "running"
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

        {@render pathField("Data directory", instance.dataDir)}
        {#if instance.configPath}
          {@render pathField("Config file", instance.configPath)}
        {/if}
        {#if instance.socketPath}
          {@render pathField("Socket", instance.socketPath)}
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
            <div
              role="presentation"
              onclick={(e) => e.stopPropagation()}
              onkeydown={(e) => e.stopPropagation()}
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

      {#if instance.linkedProjects.length === 0}
        <p class="text-[12px] text-fg-subtle">
          Not linked to any project. Linking injects
          <span class="font-mono text-fg-muted">DATABASE_URL</span> and
          <span class="font-mono text-fg-muted">DB_*</span> env vars into
          the project when it starts.
        </p>
      {:else}
        <ul class="space-y-1.5">
          {#each instance.linkedProjects as pid (pid)}
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

    <!-- Auto-start footer -->
    <footer class="flex items-center justify-between gap-3 pt-1 pb-4">
      <label class="flex items-center gap-2.5 cursor-pointer select-none">
        <input
          type="checkbox"
          checked={instance.autoStart}
          onchange={toggleAutoStart}
          disabled={databases.isBusy(instance.id, "autostart")}
          class="accent-accent"
        />
        <span class="text-[12.5px] text-fg">
          Start automatically when PortBay launches
        </span>
      </label>
      <button
        type="button"
        onclick={() => instance.dataDir && reveal(instance.dataDir)}
        class="inline-flex items-center gap-1.5 h-9 px-3 rounded-md
               border border-border bg-surface text-[12px] text-fg-muted
               hover:bg-surface-2 hover:text-fg transition-colors"
      >
        <Icon name="folder" size={12} />
        Open Data Folder
      </button>
    </footer>
  </div>
</div>
