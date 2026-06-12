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
  import { untrack } from "svelte";
  import { revealItemInDir } from "@tauri-apps/plugin-opener";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import OpenInButton from "./OpenInButton.svelte";
  import StatusPill from "$lib/components/atoms/StatusPill.svelte";
  import ProjectAvatar from "$lib/components/atoms/ProjectAvatar.svelte";
  import MobileDestinationPicker from "./MobileDestinationPicker.svelte";

  import { safeInvoke } from "$lib/ipc";
  import { startProject } from "$lib/actions/startProject";
  import { projects } from "$lib/stores/projects.svelte";
  import { groups } from "$lib/stores/groups.svelte";
  import { sidecars } from "$lib/stores/sidecars.svelte";
  import { logViewer } from "$lib/stores/logViewer.svelte";
  import { mobilePhase } from "$lib/stores/mobilePhase.svelte";
  import { projectDetailPanel } from "$lib/stores/detailPanel.svelte";

  import type { CommandError } from "$lib/types/error";
  import type { ProjectView } from "$lib/types/projects";
  import { typeLabel } from "$lib/types/projects";
  import {
    isMobileType,
    mobilePhaseDisplay,
    mobilePhaseLabel,
    type MobilePreflightCheck,
  } from "$lib/types/mobile";
  import { createCertInfo } from "$lib/stores/certInfo.svelte";

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

  // ---- Mobile cockpit (Flutter / Xcode / Android / Expo) ----
  // The rail is the destination-picker + truthful-run-feedback surface for
  // mobile kinds; web-shaped affordances (URL, port, cert checks) hide and
  // their mobile equivalents render instead. Same component, branched per
  // section, so cert/port logic stays single-source.
  const isMobile = $derived(project !== null && isMobileType(project.type));
  void mobilePhase.start();

  const phaseEntry = $derived(project ? mobilePhase.get(project.id) : null);
  // Show the phase pill while a run is in flight (or its build-failed
  // verdict); otherwise fall back to the base status pill.
  const showPhasePill = $derived(
    isMobile &&
      project !== null &&
      phaseEntry !== null &&
      (project.status !== "stopped" || phaseEntry.phase === "build_failed"),
  );

  // Ticking clock for the elapsed counter on in-flight phases (Xcode's
  // activity-area pattern: labeled phase + elapsed + cancel).
  let nowTick = $state(Date.now());
  $effect(() => {
    if (!showPhasePill) return;
    const t = setInterval(() => (nowTick = Date.now()), 1_000);
    return () => clearInterval(t);
  });
  const phaseElapsed = $derived.by<string | null>(() => {
    if (!phaseEntry) return null;
    if (phaseEntry.phase === "connected" || phaseEntry.phase === "build_failed")
      return null;
    const secs = Math.max(0, Math.floor((nowTick - phaseEntry.since) / 1000));
    const m = Math.floor(secs / 60);
    const s = secs % 60;
    return m > 0 ? `${m}:${String(s).padStart(2, "0")}` : `${s}s`;
  });

  // Run / Stop in the rail header (mobile only). Mirrors the row's optimistic
  // transition handling; mobile runs don't need the dns.ensureReady() step.
  let lifecycleBusy = $state<"start" | "stop" | null>(null);
  const railDisplay = $derived(
    project ? projects.displayStatusOf(project) : "stopped",
  );
  const railShowStop = $derived(
    railDisplay === "running" ||
      railDisplay === "starting" ||
      railDisplay === "stopping",
  );

  async function railStart() {
    if (!project || lifecycleBusy) return;
    lifecycleBusy = "start";
    projects.beginTransition(project.id, "start");
    try {
      const r = await startProject(project.id, project.name);
      if (r.kind === "declined") {
        projects.failTransition(project.id);
        return;
      }
      if (r.kind === "error") throw r.error;
      projects.clearError(project.id);
    } catch (err) {
      projects.failTransition(project.id);
      projects.setError(project.id, err as CommandError);
    } finally {
      lifecycleBusy = null;
    }
  }

  async function railStop() {
    if (!project || lifecycleBusy) return;
    lifecycleBusy = "stop";
    projects.beginTransition(project.id, "stop");
    try {
      await safeInvoke("stop_project", { id: project.id });
      projects.clearError(project.id);
    } catch (err) {
      projects.failTransition(project.id);
      projects.setError(project.id, err as CommandError);
    } finally {
      lifecycleBusy = null;
    }
  }

  // Toolchain pre-flight (mobile Checks) — actionable error states instead of
  // a crashed run with a raw log.
  let preflight = $state<MobilePreflightCheck[] | null>(null);
  let preflightLoading = $state(false);
  async function loadPreflight() {
    if (!project) return;
    preflightLoading = true;
    try {
      preflight = await safeInvoke<MobilePreflightCheck[]>("mobile_preflight", {
        id: project.id,
      });
    } catch {
      preflight = [];
    } finally {
      preflightLoading = false;
    }
  }
  $effect(() => {
    const id = project?.id;
    const mobile = isMobile;
    if (!id) return;
    preflight = null;
    if (mobile) untrack(() => void loadPreflight());
  });

  // Flutter hot reload / restart (SIGUSR1/2 to the live `flutter run`).
  let reloadBusy = $state<"reload" | "restart" | null>(null);
  async function hotReload(kind: "reload" | "restart") {
    if (!project || reloadBusy) return;
    reloadBusy = kind;
    try {
      await safeInvoke(
        kind === "reload" ? "mobile_hot_reload" : "mobile_hot_restart",
        { id: project.id },
      );
    } catch {
      /* toast already pushed */
    } finally {
      reloadBusy = null;
    }
  }

  async function openSimulator() {
    if (!project) return;
    try {
      await safeInvoke("open_mobile_simulator", { id: project.id });
    } catch {
      /* toast already pushed */
    }
  }

  // Cert info — refreshed when the selection changes. Shared loader so the
  // rail and the detail panel can't drift on cert-loading semantics.
  const cert = createCertInfo();
  const certInfo = $derived(cert.info);
  const certError = $derived(cert.error);

  function loadCert() {
    return cert.load(project);
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
      await safeInvoke("reveal_in_finder", { path: project.path });
    } catch {
      /* safeInvoke already pushed the toast */
    }
  }

  function viewLogs() {
    if (!project) return;
    logViewer.show(project.id);
  }

  function openEditor() {
    if (!project) return;
    projectDetailPanel.show(project.id);
  }

  function deselect() {
    projects.select(null);
  }

  async function copyToClipboard(text: string, _label: string) {
    // No notification — copying is self-evident. Quietly ignore a missing perm.
    try {
      await navigator.clipboard.writeText(text);
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
        ok:
          !project.https ||
          (certInfo !== null &&
            certError === null &&
            (certInfo.status === "ready" ||
              certInfo.status === "regenerateNeeded")),
        detail:
          !project.https
            ? "Not used"
            : certInfo
              ? certInfo.status === "ready"
                ? "Ready"
                : certInfo.status === "regenerateNeeded"
                  ? "Renew soon"
                  : "Needs attention"
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
        <ProjectAvatar
          id={project.id}
          name={project.name}
          type={project.type}
          size={40}
        />
        <div class="min-w-0 leading-tight">
          <p class="text-[15px] font-semibold text-fg truncate">
            {project.name}
          </p>
          {#if showPhasePill && phaseEntry}
            <!-- Truthful mobile sub-state: green only once actually attached. -->
            <span class="inline-flex items-center gap-1.5">
              <StatusPill
                status={mobilePhaseDisplay(phaseEntry.phase)}
                label={mobilePhaseLabel[phaseEntry.phase]}
              />
              {#if phaseElapsed}
                <span class="text-[10.5px] text-fg-subtle tabular-nums">
                  {phaseElapsed}
                </span>
              {/if}
            </span>
          {:else}
            <StatusPill status={project.status} />
          {/if}
        </div>
      </div>
      <div class="flex items-center gap-1 shrink-0">
        {#if isMobile}
          <!-- Run / Stop live in the rail header for mobile — the rail is the
               cockpit where you pick a destination, then run on it. Stop
               doubles as build cancel. -->
          {#if railShowStop}
            <button
              type="button"
              onclick={() => void railStop()}
              disabled={lifecycleBusy !== null}
              title="Stop {project.name}"
              aria-label="Stop {project.name}"
              class="inline-flex items-center justify-center w-7 h-7 rounded-md
                     text-on-accent bg-status-crashed hover:brightness-110
                     active:brightness-95 disabled:opacity-50 transition"
            >
              {#if lifecycleBusy === "stop"}
                <Icon name="refresh-cw" size={11} class="animate-spin" />
              {:else}
                <Icon name="square" size={10} class="fill-current" />
              {/if}
            </button>
          {:else}
            <button
              type="button"
              onclick={() => void railStart()}
              disabled={lifecycleBusy !== null}
              title="Run {project.name} on the selected destination"
              aria-label="Run {project.name}"
              class="inline-flex items-center justify-center w-7 h-7 rounded-md
                     text-on-accent bg-status-running hover:brightness-110
                     active:brightness-95 disabled:opacity-50 transition"
            >
              {#if lifecycleBusy === "start"}
                <Icon name="refresh-cw" size={11} class="animate-spin" />
              {:else}
                <Icon name="play" size={11} class="fill-current" />
              {/if}
            </button>
          {/if}
        {/if}
        <button
          type="button"
          onclick={deselect}
          title="Close detail"
          aria-label="Close detail"
          class="p-1.5 rounded-md text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
        >
          <Icon name="x" size={15} />
        </button>
      </div>
    </header>

    {#if isMobile}
      <!-- Destination — the Xcode-style pre-run ritual replaces the URL chip.
           Selection persists to MobileRunConfig.device; the row's plain Play
           runs whatever is pinned here. -->
      <div class="space-y-1">
        <MobileDestinationPicker {project} />
        {#if project.mobileRun?.target || project.mobileRun?.flavor}
          <p class="px-1 text-[10.5px] text-fg-subtle truncate">
            {#if project.mobileRun?.target}
              <span class="font-mono">{project.mobileRun.target}</span>
            {/if}
            {#if project.mobileRun?.target && project.mobileRun?.flavor}·{/if}
            {#if project.mobileRun?.flavor}
              <span class="font-mono">{project.mobileRun.flavor}</span>
            {/if}
          </p>
        {/if}
      </div>
    {:else}
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
    {/if}

    <!-- Metadata -->
    <dl class="grid grid-cols-[88px,1fr] gap-x-3 gap-y-2 text-[12px]">
      {#if groupName}
        <!-- Ungrouped projects skip the row entirely — a "—" placeholder
             carries no information. -->
        <dt class="text-fg-muted">Group</dt>
        <dd class="text-fg flex items-center gap-1.5 min-w-0">
          <Icon name="folder" size={11} class="text-fg-subtle" />
          <span class="truncate">{groupName}</span>
        </dd>
      {/if}

      <dt class="text-fg-muted">Runtime</dt>
      <dd class="text-fg truncate">{typeLabel[project.type]}</dd>

      {#if !isMobile}
        <!-- Port/URL rows are web-shaped and wrong for mobile — the
             destination block above covers the run target instead. -->
        <dt class="text-fg-muted">Port</dt>
        <dd class="text-fg font-mono tabular-nums">
          {project.port ?? "—"}
          {#if project.https}
            <span class="text-fg-subtle ml-1">(HTTPS)</span>
          {/if}
        </dd>
      {/if}

      <dt class="text-fg-muted">Started</dt>
      <dd class="text-fg">{startedDisplay ?? "—"}</dd>

      {#if project.startCommand && !isMobile}
        <!-- Mobile start commands are generated multi-step scripts; the
             truncated one-liner reads as noise here. -->
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
        {#if isMobile}
          {#if project.type === "flutter"}
            <button
              type="button"
              onclick={() => void hotReload("reload")}
              disabled={reloadBusy !== null || !railShowStop}
              title={railShowStop
                ? "Hot reload (inject changed code, keep state)"
                : "Start the app to enable hot reload"}
              class="inline-flex items-center gap-2 px-3 py-2 rounded-md
                     border border-border bg-surface hover:bg-surface-2
                     text-[12px] text-fg-muted hover:text-fg transition-colors
                     disabled:opacity-50"
            >
              <Icon
                name="zap"
                size={13}
                class={reloadBusy === "reload" ? "animate-pulse" : ""}
              /> Hot Reload
            </button>
            <button
              type="button"
              onclick={() => void hotReload("restart")}
              disabled={reloadBusy !== null || !railShowStop}
              title={railShowStop
                ? "Hot restart (rebuild app state)"
                : "Start the app to enable hot restart"}
              class="inline-flex items-center gap-2 px-3 py-2 rounded-md
                     border border-border bg-surface hover:bg-surface-2
                     text-[12px] text-fg-muted hover:text-fg transition-colors
                     disabled:opacity-50"
            >
              <Icon
                name="rotate-cw"
                size={13}
                class={reloadBusy === "restart" ? "animate-spin" : ""}
              /> Hot Restart
            </button>
          {/if}
          {#if project.type !== "android"}
            <button
              type="button"
              onclick={() => void openSimulator()}
              class="inline-flex items-center gap-2 px-3 py-2 rounded-md
                     border border-border bg-surface hover:bg-surface-2
                     text-[12px] text-fg-muted hover:text-fg transition-colors"
            >
              <Icon name="smartphone" size={13} /> Open Simulator
            </button>
          {/if}
        {:else}
          <button
            type="button"
            onclick={openInBrowser}
            class="inline-flex items-center gap-2 px-3 py-2 rounded-md
                   border border-border bg-surface hover:bg-surface-2
                   text-[12px] text-fg-muted hover:text-fg transition-colors"
          >
            <Icon name="globe" size={13} /> Open in Browser
          </button>
        {/if}
        <OpenInButton projectId={project.id} />

        <button
          type="button"
          onclick={revealInFinder}
          title="Reveal in Finder"
          class="inline-flex items-center gap-2 px-3 py-2 rounded-md
                 border border-border bg-surface hover:bg-surface-2
                 text-[12px] text-fg-muted hover:text-fg transition-colors"
        >
          <Icon name="finder" size={13} /> Reveal
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
        {#if isMobile && mobilePhase.historyOf(project.id).length > 0}
          <!-- Phase transitions of the current/last run, newest first —
               the mobile equivalent of richer audit events. -->
          {#each mobilePhase.historyOf(project.id) as t (t.ts + t.phase)}
            <li class="flex items-center gap-2">
              <span
                class="w-1.5 h-1.5 rounded-full shrink-0
                       {t.phase === 'connected'
                  ? 'bg-status-running'
                  : t.phase === 'build_failed'
                    ? 'bg-status-crashed'
                    : 'bg-status-starting'}"
              ></span>
              <span class="text-fg-muted">
                {mobilePhaseLabel[t.phase]}{t.detail ? ` — ${t.detail}` : ""}
              </span>
              <span class="ml-auto text-fg-subtle shrink-0">
                {new Date(t.ts).toLocaleTimeString([], {
                  hour: "numeric",
                  minute: "2-digit",
                })}
              </span>
            </li>
          {/each}
        {:else}
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
        {/if}
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

    <!-- Checks — web kinds get cert/port/Caddy; mobile kinds get toolchain
         pre-flight (Xcode tools, simulators, adb, AVDs, flutter on PATH). -->
    <section>
      <div class="flex items-baseline justify-between mb-2">
        <h3 class="text-[11px] uppercase tracking-wide text-fg-subtle">
          Checks
        </h3>
        {#if isMobile}
          <button
            type="button"
            onclick={() => void loadPreflight()}
            disabled={preflightLoading}
            title="Re-run toolchain checks"
            aria-label="Re-run toolchain checks"
            class="p-0.5 rounded text-fg-subtle hover:text-fg disabled:opacity-50
                   transition-colors"
          >
            <Icon
              name="refresh-cw"
              size={11}
              class={preflightLoading ? "animate-spin" : ""}
            />
          </button>
        {/if}
      </div>
      <ul class="space-y-1.5 text-[12px]">
        {#if isMobile}
          {#if preflight === null}
            <li class="text-fg-subtle text-[11.5px]">Checking toolchain…</li>
          {:else}
            {#each preflight as c (c.label)}
              <li class="flex items-start gap-2">
                <span
                  class="inline-flex items-center justify-center w-4 h-4 rounded-full shrink-0
                         {c.ok
                    ? 'bg-status-running/15 text-status-running'
                    : 'bg-status-unhealthy/15 text-status-unhealthy'}"
                >
                  <Icon name={c.ok ? "check" : "circle-alert"} size={10} />
                </span>
                <span class="text-fg-muted shrink-0">{c.label}</span>
                <span
                  class="ml-auto text-right {c.ok
                    ? 'text-fg-muted'
                    : 'text-status-unhealthy'}"
                >
                  {c.detail}
                </span>
              </li>
            {/each}
          {/if}
        {:else}
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
        {/if}
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
