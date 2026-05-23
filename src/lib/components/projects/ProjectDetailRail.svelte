<!--
  ProjectDetailRail — inline-rail variant of the project detail surface.

  Replaces the slide-over modal as the default detail UX. The modal
  still exists for deep editing (env, advanced fields, raw config) —
  it's opened via the "Edit project…" button at the bottom of the
  rail, or the ellipsis menu in the table.

  Structure mirrors the design reference:
    - Header: avatar, name, status pill, close button.
    - URL chip — clicks open in browser.
    - Metadata <dl>: Group, Runtime, Port, Started, Command, Directory.
    - Quick Actions grid (2×2): Open in Browser, Open in Terminal,
      Reveal in Finder, View Logs.
    - Recent Activity — last few status events, sourced from a small
      ring kept locally in the rail (the projects store doesn't
      retain history; for now we mirror the current status as the
      single visible event until the reconcile loop emits richer
      audit events).
    - Checks — HTTPS cert valid, no port conflicts, Caddy running.
    - Footer link → opens the full edit modal.
-->
<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { openUrl } from "@tauri-apps/plugin-opener";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import StatusPill from "$lib/components/atoms/StatusPill.svelte";
  import ProjectAvatar from "$lib/components/atoms/ProjectAvatar.svelte";

  import { safeInvoke } from "$lib/ipc";
  import { errorBus } from "$lib/stores/errors.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import { groups } from "$lib/stores/groups.svelte";
  import { sidecars } from "$lib/stores/sidecars.svelte";
  import { logViewer } from "$lib/stores/logViewer.svelte";
  import { projectDetailPanel } from "$lib/stores/detailPanel.svelte";

  import type { ProjectView } from "$lib/types/projects";
  import { typeLabel } from "$lib/types/projects";
  import type { CertInfo } from "$lib/types/certs";

  // ---- Resolved project (driven by projects.selectedId) ----
  const project = $derived<ProjectView | null>(
    projects.selectedId === null
      ? null
      : (projects.value.find((p) => p.id === projects.selectedId) ?? null),
  );

  // Group subtitle (first matching group). Same logic as the row.
  const groupName = $derived.by<string | null>(() => {
    if (!project) return null;
    return (
      groups.value.find((g) => g.knownIds.includes(project.id))?.name ?? null
    );
  });

  // Started-at — derived from runtime.age (nanoseconds since process start).
  const startedDisplay = $derived.by<string | null>(() => {
    if (!project?.runtime) return null;
    const ageMs = project.runtime.age / 1_000_000;
    const startTs = Date.now() - ageMs;
    return new Date(startTs).toLocaleTimeString([], {
      hour: "numeric",
      minute: "2-digit",
    });
  });

  // Cert info — refreshed when the selection changes.
  let certInfo = $state<CertInfo | null>(null);
  let certError = $state<string | null>(null);

  async function loadCert() {
    if (!project || !project.https) {
      certInfo = null;
      certError = null;
      return;
    }
    try {
      certInfo = await safeInvoke<CertInfo>("cert_info", { id: project.id });
      certError = null;
    } catch (e) {
      const err = e as { code?: string; whatHappened?: string } | undefined;
      certInfo = null;
      certError =
        err && err.code !== "PROJECT_NOT_FOUND"
          ? (err.whatHappened ?? null)
          : null;
    }
  }

  // Reload cert when the selection changes.
  $effect(() => {
    const id = project?.id;
    if (!id) return;
    untrack(() => void loadCert());
  });

  // ---- Quick actions ----
  async function openInBrowser() {
    if (!project) return;
    try {
      await safeInvoke("open_project", { id: project.id });
    } catch {
      /* toast */
    }
  }

  async function revealInFinder() {
    if (!project) return;
    try {
      await openUrl(`file://${project.path}`);
    } catch {
      /* opener pushes its own toast */
    }
  }

  function viewLogs() {
    if (!project) return;
    logViewer.show(project.id);
  }

  async function openInTerminal() {
    if (!project) return;
    // No dedicated IPC for "open Terminal here" — punt to the file
    // manager so the user is one keystroke away from `cd $(pwd)`.
    // A follow-up command lands when we wire iTerm/Terminal/Warp.
    try {
      await openUrl(`file://${project.path}`);
      errorBus.push({
        code: "TERMINAL_FALLBACK",
        whatHappened: "Opened the project folder.",
        whyItMatters:
          "PortBay doesn't have a terminal integration yet — opened Finder so you can drag it into your terminal.",
        whoCausedIt: "system",
        severity: "info",
        actions: [],
      });
    } catch {
      /* opener pushes its own toast */
    }
  }

  function openEditor() {
    if (!project) return;
    projectDetailPanel.show(project.id);
  }

  function deselect() {
    projects.select(null);
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
      /* quiet — no clipboard permission */
    }
  }

  // Checks block — boolean true means "OK".
  const checks = $derived.by(() => {
    if (!project) {
      return [] as { label: string; ok: boolean; detail: string }[];
    }
    return [
      {
        label: "HTTPS certificate",
        ok: !project.https || (certInfo !== null && certError === null),
        detail:
          !project.https
            ? "Not used"
            : certInfo
              ? "Trusted"
              : certError ?? "Pending",
      },
      {
        label: "Port conflicts",
        ok: project.status !== "port_conflict",
        detail:
          project.status === "port_conflict" ? "Conflict" : "None",
      },
      {
        label: "Caddy",
        ok: sidecars.value.caddy.status === "running",
        detail:
          sidecars.value.caddy.status === "running" ? "Running" : "Down",
      },
    ];
  });
</script>

{#if project}
  <div class="flex flex-col gap-4">
    <!-- Header -->
    <header class="flex items-start justify-between gap-3">
      <div class="flex items-center gap-3 min-w-0">
        <ProjectAvatar id={project.id} name={project.name} size={40} />
        <div class="min-w-0 leading-tight">
          <p class="text-[15px] font-semibold text-fg truncate">
            {project.name}
          </p>
          <StatusPill status={project.status} />
        </div>
      </div>
      <button
        type="button"
        onclick={deselect}
        title="Close detail"
        aria-label="Close detail"
        class="p-1.5 rounded-md text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
      >
        <Icon name="x" size={15} />
      </button>
    </header>

    <!-- URL chip -->
    <button
      type="button"
      onclick={openInBrowser}
      class="inline-flex items-center justify-between gap-2 px-3 py-2
             rounded-lg bg-surface-2/60 hover:bg-surface-2
             text-[12px] text-accent hover:text-accent-hover
             border border-border/60 transition-colors w-full"
      title="Open in browser"
    >
      <span class="truncate font-mono">{project.url}</span>
      <Icon name="external-link" size={12} class="shrink-0 opacity-80" />
    </button>

    <!-- Metadata -->
    <dl class="grid grid-cols-[88px,1fr] gap-x-3 gap-y-2 text-[12px]">
      <dt class="text-fg-muted">Group</dt>
      <dd class="text-fg flex items-center gap-1.5 min-w-0">
        <Icon name="folder" size={11} class="text-fg-subtle" />
        <span class="truncate">{groupName ?? "—"}</span>
      </dd>

      <dt class="text-fg-muted">Runtime</dt>
      <dd class="text-fg truncate">{typeLabel[project.type]}</dd>

      <dt class="text-fg-muted">Port</dt>
      <dd class="text-fg font-mono tabular-nums">
        {project.port ?? "—"}
        {#if project.https}
          <span class="text-fg-subtle ml-1">(HTTPS)</span>
        {/if}
      </dd>

      <dt class="text-fg-muted">Started</dt>
      <dd class="text-fg">{startedDisplay ?? "—"}</dd>

      {#if project.startCommand}
        <dt class="text-fg-muted">Command</dt>
        <dd class="text-fg font-mono text-[11px] truncate">
          {project.startCommand}
        </dd>
      {/if}

      <dt class="text-fg-muted">Directory</dt>
      <dd class="flex items-center gap-1.5 min-w-0">
        <span class="text-fg font-mono text-[11px] truncate">
          {project.path}
        </span>
        <button
          type="button"
          onclick={() => copyToClipboard(project.path, "Path")}
          title="Copy path"
          class="shrink-0 p-0.5 rounded text-fg-subtle hover:text-fg"
        >
          <Icon name="link" size={11} />
        </button>
      </dd>
    </dl>

    <!-- Quick Actions -->
    <section>
      <h3 class="text-[11px] uppercase tracking-wide text-fg-subtle mb-2">
        Quick Actions
      </h3>
      <div class="grid grid-cols-2 gap-2">
        <button
          type="button"
          onclick={openInBrowser}
          class="inline-flex items-center gap-2 px-3 py-2 rounded-md
                 border border-border bg-surface hover:bg-surface-2
                 text-[12px] text-fg-muted hover:text-fg transition-colors"
        >
          <Icon name="globe" size={13} /> Open in Browser
        </button>
        <button
          type="button"
          onclick={openInTerminal}
          class="inline-flex items-center gap-2 px-3 py-2 rounded-md
                 border border-border bg-surface hover:bg-surface-2
                 text-[12px] text-fg-muted hover:text-fg transition-colors"
        >
          <Icon name="terminal" size={13} /> Open in Terminal
        </button>
        <button
          type="button"
          onclick={revealInFinder}
          class="inline-flex items-center gap-2 px-3 py-2 rounded-md
                 border border-border bg-surface hover:bg-surface-2
                 text-[12px] text-fg-muted hover:text-fg transition-colors"
        >
          <Icon name="folder" size={13} /> Reveal in Finder
        </button>
        <button
          type="button"
          onclick={viewLogs}
          class="inline-flex items-center gap-2 px-3 py-2 rounded-md
                 border border-border bg-surface hover:bg-surface-2
                 text-[12px] text-fg-muted hover:text-fg transition-colors"
        >
          <Icon name="file-text" size={13} /> View Logs
        </button>
      </div>
    </section>

    <!-- Recent Activity -->
    <section>
      <div class="flex items-baseline justify-between mb-2">
        <h3 class="text-[11px] uppercase tracking-wide text-fg-subtle">
          Recent Activity
        </h3>
      </div>
      <ul class="space-y-1.5 text-[12px]">
        <li class="flex items-center gap-2">
          <span
            class="w-1.5 h-1.5 rounded-full shrink-0
                   {project.status === 'running'
              ? 'bg-status-running'
              : 'bg-fg-subtle'}"
          ></span>
          <span class="text-fg-muted">
            {project.status === "running" ? "Started" : "Status"}
          </span>
          <span class="ml-auto text-fg-subtle">
            {startedDisplay ?? "—"}
          </span>
        </li>
        {#if project.runtime}
          <li class="flex items-center gap-2">
            <span class="w-1.5 h-1.5 rounded-full bg-fg-subtle shrink-0"></span>
            <span class="text-fg-muted">Restarts</span>
            <span class="ml-auto text-fg-subtle tabular-nums">
              {project.runtime.restarts}
            </span>
          </li>
        {/if}
      </ul>
    </section>

    <!-- Checks -->
    <section>
      <h3 class="text-[11px] uppercase tracking-wide text-fg-subtle mb-2">
        Checks
      </h3>
      <ul class="space-y-1.5 text-[12px]">
        {#each checks as c (c.label)}
          <li class="flex items-center gap-2">
            <span
              class="inline-flex items-center justify-center w-4 h-4 rounded-full shrink-0
                     {c.ok
                ? 'bg-status-running/15 text-status-running'
                : 'bg-status-unhealthy/15 text-status-unhealthy'}"
            >
              <Icon name={c.ok ? "check" : "circle-alert"} size={10} />
            </span>
            <span class="text-fg-muted">{c.label}</span>
            <span
              class="ml-auto {c.ok
                ? 'text-fg-muted'
                : 'text-status-unhealthy'}"
            >
              {c.detail}
            </span>
          </li>
        {/each}
      </ul>
    </section>

    <!-- Footer escape hatch -->
    <div class="pt-2 mt-2 border-t border-border/70">
      <button
        type="button"
        onclick={openEditor}
        class="w-full inline-flex items-center justify-center gap-1.5 px-3 py-2
               rounded-md border border-border text-[12px] text-fg-muted
               hover:text-fg hover:bg-surface-2 transition-colors"
      >
        <Icon name="pencil" size={12} /> Edit project…
      </button>
    </div>
  </div>
{:else}
  <div
    class="h-full flex flex-col items-center justify-center text-center
           text-fg-subtle gap-2 px-4 py-12"
  >
    <Icon name="eye" size={20} />
    <p class="text-[12.5px]">No project selected.</p>
    <p class="text-[11px]">
      Click a row to see status, quick actions, and checks.
    </p>
  </div>
{/if}
