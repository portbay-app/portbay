<!--
  ProjectDbConnections — read-only "Database" section for the detail panel.

  Parses the project's on-disk `.env` (backend `project_db_connections`) and
  shows each DB_* connection with per-field copy buttons, a masked password
  with a reveal toggle, and an "Open in DB client" button that hands the
  scheme URL (mysql:// etc.) to whatever app registered it (TablePlus, Sequel
  Ace, DBeaver). Renders nothing when the project has no DB_* vars, so it's
  safe to always mount.

  Password reveal is a click toggle rather than hover-only: hover can't be
  triggered by keyboard, and this panel went through the a11y sweep.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { openUrl } from "$lib/security/openUrl";

  import { DashboardCard, Icon } from "$lib/components/atoms";
  import DatabaseWorkbench from "$lib/components/databases/DatabaseWorkbench.svelte";
  import { safeInvoke } from "$lib/ipc";
  import { databases } from "$lib/stores/databases.svelte";
  import { errorBus } from "$lib/stores/errors.svelte";
  import type {
    ProjectDbConnection,
    ProjectDbProvision,
  } from "$lib/types/databases";
  import type { ProjectView } from "$lib/types/projects";

  interface Props {
    project: ProjectView;
  }
  let { project }: Props = $props();

  let connections = $state<ProjectDbConnection[]>([]);
  /** Connection names whose password is currently revealed. */
  let revealed = $state<Set<string>>(new Set());

  // ── Provisioning ──────────────────────────────────────────────────────────
  // A dedicated database can be provisioned on any *running* SQL instance.
  const SQL_ENGINES = ["mysql", "mariadb", "postgres"];
  let provisioning = $state<boolean>(false);
  let pickerOpen = $state<boolean>(false);

  const provisionable = $derived(
    databases.value.filter(
      (i) => i.status === "running" && SQL_ENGINES.includes(i.engine),
    ),
  );

  const embeddedInstances = $derived(
    databases.value.filter(
      (i) =>
        i.linkedProjects.includes(project.id) &&
        ["mysql", "mariadb", "postgres", "sqlite"].includes(i.engine),
    ),
  );

  const initialSchema = $derived(
    connections.find((conn) => conn.database && conn.driver !== "sqlite")
      ?.database ?? null,
  );

  onMount(() => {
    void databases.refresh();
  });

  /** Strong, alphanumeric password (Web Crypto) — alnum so it needs no SQL/URL
      escaping, matching the backend's validation. */
  function genPassword(len = 24): string {
    const alphabet =
      "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    const arr = new Uint32Array(len);
    crypto.getRandomValues(arr);
    return Array.from(arr, (n) => alphabet[n % alphabet.length]).join("");
  }

  async function provision(instanceId: string) {
    if (provisioning) return;
    provisioning = true;
    pickerOpen = false;
    try {
      const res = await safeInvoke<ProjectDbProvision>(
        "provision_project_database",
        { projectId: project.id, instanceId, password: genPassword() },
      );
      errorBus.push({
        code: "DB_PROVISIONED",
        category: "infrastructure",
        whatHappened: `Database "${res.database}" provisioned for ${project.name}.`,
        whyItMatters:
          "DB_* and DATABASE_URL were written to this project's .env. Restart the project to pick them up.",
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });
      await load();
    } catch {
      /* safeInvoke toasted the failure */
    } finally {
      provisioning = false;
    }
  }

  function onProvisionClick() {
    if (provisionable.length === 1) {
      void provision(provisionable[0].id);
    } else {
      pickerOpen = !pickerOpen;
    }
  }

  async function load() {
    revealed = new Set();
    try {
      connections = await safeInvoke<ProjectDbConnection[]>(
        "project_db_connections",
        { id: project.id },
      );
    } catch {
      // safeInvoke toasts; an empty section is the right fallback.
      connections = [];
    }
  }

  // Reload when the open project changes.
  $effect(() => {
    const _id = project.id;
    void load();
  });

  function toggleReveal(name: string) {
    const next = new Set(revealed);
    if (next.has(name)) {
      next.delete(name);
    } else {
      next.add(name);
    }
    revealed = next;
  }

  async function copy(text: string, _label: string) {
    if (!text) return;
    // No notification — copying is self-evident. Quietly ignore a missing perm.
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      /* no clipboard permission — quietly ignore */
    }
  }

  async function openInClient(conn: ProjectDbConnection) {
    if (!conn.url) return;
    try {
      await openUrl(conn.url);
    } catch {
      /* opener pushes its own toast */
    }
  }

  // Plain (non-secret) fields rendered per connection, in display order.
  const FIELDS: Array<{ label: string; key: "host" | "port" | "database" | "username" }> = [
    { label: "Host", key: "host" },
    { label: "Port", key: "port" },
    { label: "Database", key: "database" },
    { label: "Username", key: "username" },
  ];
</script>

<svelte:window onclick={() => (pickerOpen = false)} />

{#if connections.length > 0 || provisionable.length > 0 || embeddedInstances.length > 0}
  <DashboardCard title="Database" flush>
    <div class="relative flex items-start justify-between gap-2 mb-2">
      <p class="text-[11px] text-fg-subtle">
        {#if connections.length > 0}
          Parsed from this project's <span class="font-mono">.env</span>. Read-only.
        {:else}
          No database connection yet.
        {/if}
      </p>
      {#if provisionable.length > 0}
        <button
          type="button"
          onclick={(e) => {
            e.stopPropagation();
            onProvisionClick();
          }}
          disabled={provisioning}
          title="Create a dedicated database for this project on a running instance"
          class="shrink-0 inline-flex items-center gap-1 px-2 h-7 rounded-md
                 border border-accent/40 text-accent text-[11px]
                 hover:bg-accent/10 disabled:opacity-50 transition-colors"
        >
          {#if provisioning}
            <Icon name="refresh-cw" size={10} class="animate-spin" />
            Provisioning…
          {:else}
            <Icon name="plus" size={11} />
            Provision
          {/if}
        </button>
        {#if pickerOpen}
          <div
            role="presentation"
            onclick={(e) => e.stopPropagation()}
            class="absolute right-0 top-8 z-30 w-56 max-h-64 overflow-y-auto
                   rounded-lg border border-border bg-surface shadow-2xl py-1"
          >
            {#each provisionable as inst (inst.id)}
              <button
                type="button"
                onclick={() => provision(inst.id)}
                class="w-full text-left px-3 py-1.5 text-[12px] text-fg-muted
                       hover:bg-surface-2 hover:text-fg transition-colors truncate"
              >
                {inst.name} · {inst.engineLabel}
              </button>
            {/each}
          </div>
        {/if}
      {/if}
    </div>

    {#if connections.length === 0}
      <p class="text-[12px] text-fg-subtle">
        Provision a dedicated database on a running instance — PortBay creates
        it, adds a login user, and writes the <span class="font-mono">DB_*</span>
        vars into this project's <span class="font-mono">.env</span>.
      </p>
    {:else}
    <div class="space-y-3">
      {#each connections as conn (conn.name)}
        <div class="rounded-md border border-border overflow-hidden">
          <div
            class="flex items-center gap-2 px-2.5 py-1.5 bg-surface-2 text-[11px]"
          >
            <span class="font-medium text-fg">{conn.name}</span>
            {#if conn.driver}
              <span class="font-mono text-fg-subtle">{conn.driver}</span>
            {/if}
            {#if conn.url}
              <button
                type="button"
                onclick={() => openInClient(conn)}
                title="Open in your default database client"
                class="ml-auto inline-flex items-center gap-1 px-1.5 py-0.5 rounded
                       text-accent hover:bg-accent/10 transition-colors"
              >
                <Icon name="external-link" size={11} />
                Open in DB client
              </button>
            {/if}
          </div>

          <dl
            class="grid grid-cols-[90px,1fr] gap-x-3 gap-y-1.5 px-2.5 py-2 text-xs"
          >
            {#each FIELDS as field (field.key)}
              {@const value = conn[field.key]}
              {#if value}
                <dt class="text-fg-muted">{field.label}</dt>
                <dd class="flex items-center gap-2 min-w-0">
                  <span class="text-fg font-mono truncate">{value}</span>
                  <button
                    type="button"
                    onclick={() => copy(value, field.label)}
                    title="Copy {field.label.toLowerCase()}"
                    aria-label="Copy {field.label.toLowerCase()}"
                    class="p-0.5 rounded text-fg-subtle hover:text-fg shrink-0"
                  >
                    <Icon name="link" size={11} />
                  </button>
                </dd>
              {/if}
            {/each}

            {#if conn.password}
              <dt class="text-fg-muted">Password</dt>
              <dd class="flex items-center gap-2 min-w-0">
                <span class="text-fg font-mono truncate">
                  {revealed.has(conn.name) ? conn.password : "••••••••"}
                </span>
                <button
                  type="button"
                  onclick={() => toggleReveal(conn.name)}
                  title={revealed.has(conn.name) ? "Hide password" : "Reveal password"}
                  aria-label={revealed.has(conn.name)
                    ? "Hide password"
                    : "Reveal password"}
                  class="p-0.5 rounded text-fg-subtle hover:text-fg shrink-0"
                >
                  <Icon name={revealed.has(conn.name) ? "x" : "eye"} size={11} />
                </button>
                <button
                  type="button"
                  onclick={() => copy(conn.password, "Password")}
                  title="Copy password"
                  aria-label="Copy password"
                  class="p-0.5 rounded text-fg-subtle hover:text-fg shrink-0"
                >
                  <Icon name="link" size={11} />
                </button>
              </dd>
            {/if}
          </dl>
        </div>
      {/each}
    </div>
    {/if}
  </DashboardCard>

  {#if embeddedInstances.length > 0}
    <div class="space-y-2">
      <div class="flex items-center gap-2 px-0.5">
        <Icon name="link" size={12} class="text-fg-subtle" />
        <span class="text-[11px] uppercase tracking-wide text-fg-subtle">
          Linked database{embeddedInstances.length === 1 ? "" : "s"}
        </span>
        <span class="text-[11px] text-fg-subtle">
          · managed by PortBay, attached from the Databases page
        </span>
      </div>
      <div class="space-y-3">
        {#each embeddedInstances as instance (instance.id)}
          <DatabaseWorkbench {instance} {initialSchema} compact />
        {/each}
      </div>
    </div>
  {/if}
{/if}
