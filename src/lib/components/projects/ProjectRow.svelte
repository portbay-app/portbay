<!--
  ProjectRow — one row of the redesigned projects table.

  Columns: Project (avatar + name + group subtitle), Stack (icon +
  label), URL (clickable), Port, Status (dot + label), Actions
  (primary stop/start + ellipsis menu).

  Row click selects the project — the right rail shows the detail.
  Editing the project is an explicit "Edit…" action in the ellipsis
  menu (or the rail's footer link) so a stray click doesn't pop the
  heavy modal.

  Inline error envelopes appear in a follow-up row when an action
  fails — same shape as the previous design.
-->
<script lang="ts">
  import Icon from "$lib/components/atoms/Icon.svelte";
  import StatusDot from "$lib/components/atoms/StatusDot.svelte";
  import StackIcon from "$lib/components/atoms/StackIcon.svelte";
  import ProjectAvatar from "$lib/components/atoms/ProjectAvatar.svelte";
  import ErrorEnvelope from "$lib/components/errors/ErrorEnvelope.svelte";

  import { safeInvoke } from "$lib/ipc";
  import { startProject } from "$lib/actions/startProject";
  import { groups } from "$lib/stores/groups.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import { dns } from "$lib/stores/dns.svelte";
  import { density } from "$lib/stores/density.svelte";
  import { sidecars } from "$lib/stores/sidecars.svelte";

  import type { CommandError } from "$lib/types/error";
  import type { ProjectView } from "$lib/types/projects";
  import { displayStatusLabel } from "$lib/types/status";
  import {
    typeLabel,
    effectiveWebServer,
    webServerLabel,
    webServerWarningEnvelope,
  } from "$lib/types/projects";

  import ProjectRowMenu from "./ProjectRowMenu.svelte";
  import OpenInButton from "./OpenInButton.svelte";

  interface Props {
    project: ProjectView;
  }
  let { project }: Props = $props();

  let busy = $state<"start" | "stop" | "restart" | null>(null);
  // Dismissible inline routing warning shown after a successful start when
  // Caddy or dnsmasq is not running. Null means no warning / dismissed.
  let routingWarning = $state<"caddy" | "dnsmasq" | "both" | null>(null);
  let restartingRouting = $state(false);

  const isSelected = $derived(projects.selectedId === project.id);
  // The status the row *shows* — optimistic overlay while a Play/Stop is in
  // flight, otherwise the real status. Drives the dot, label, and which
  // primary button appears.
  const display = $derived(projects.displayStatusOf(project));
  const showStop = $derived(
    display === "running" || display === "starting" || display === "stopping",
  );
  // Primary-button mode: an in-flight action (busy) wins so the spinner tracks
  // the actual IPC; otherwise the displayed status decides stop vs start.
  const buttonMode = $derived<"starting" | "stopping" | "stop" | "start">(
    busy === "start"
      ? "starting"
      : busy === "stop"
        ? "stopping"
        : showStop
          ? "stop"
          : "start",
  );
  const compact = $derived(density.value === "compact");
  const cellClass = $derived(compact ? "py-2 px-3" : "py-3 px-4");

  const inlineError = $derived(projects.lastErrors[project.id] ?? null);

  // Web-server setup advisory (e.g. nginx/apache not installed) — derived
  // backend state, shown as an inline *warning* envelope so a project that only
  // serves PortBay's placeholder explains itself instead of looking broken.
  // Split the backend's one-sentence-each message into the envelope's
  // what/why lines; clears itself on the next list fetch once the binary is in.
  const webServerWarning = $derived(
    webServerWarningEnvelope(project.webServerWarning),
  );

  // Subtitle = first group the project belongs to. Projects in zero
  // groups fall back to the type label so the row never feels empty.
  // Subtitle under the name is the project's group, if any. We deliberately
  // don't fall back to the stack label here — the Stack column already shows
  // it (icon + label), so repeating it under the name is redundant. Ungrouped
  // rows show just the name, vertically centred against the avatar.
  const groupSubtitle = $derived.by<string | null>(() => {
    const g = groups.value.find((g) => g.knownIds.includes(project.id));
    return g ? g.name : null;
  });

  const statusText = $derived(displayStatusLabel(display));

  // The web server actually fronting this project (PHP doc-root projects
  // only); null when the choice doesn't apply, so we don't mislabel a Node
  // app as "Caddy".
  const server = $derived(effectiveWebServer(project));

  async function run(op: "start" | "stop" | "restart") {
    if (busy) return;
    busy = op;
    // Flip the row to its optimistic state *now*, before any await, so the UI
    // responds on the click rather than after the IPC round-trip. A restart
    // ends up running, so it reads as a start. The real status event (or a
    // failure below) reconciles it.
    projects.beginTransition(project.id, op === "stop" ? "stop" : "start");
    try {
      switch (op) {
        case "start": {
          await dns.ensureReady();
          // Resolves a port conflict via a confirm + force-quit.
          const r = await startProject(project.id, project.name);
          if (r.kind === "declined") {
            projects.failTransition(project.id); // nothing started — roll back
            break;
          }
          if (r.kind === "error") throw r.error;
          // After a successful start, check if routing sidecars are up.
          // Only warn when the project uses a .test (or https) hostname — a
          // port-only project is reachable via localhost regardless.
          if (project.hostname) {
            const snap = sidecars.value;
            const caddyDown = snap.caddy.status !== "running";
            const dnsDown = snap.dnsmasq.status !== "running";
            if (caddyDown && dnsDown) routingWarning = "both";
            else if (caddyDown) routingWarning = "caddy";
            else if (dnsDown) routingWarning = "dnsmasq";
          }
          break;
        }
        case "stop":
          routingWarning = null;
          await safeInvoke("stop_project", { id: project.id });
          break;
        case "restart":
          await safeInvoke("restart_project", { id: project.id });
          break;
      }
      projects.clearError(project.id);
    } catch (err) {
      projects.failTransition(project.id); // roll the optimistic overlay back
      projects.setError(project.id, err as CommandError);
    } finally {
      busy = null;
    }
  }

  /** Restart Caddy and/or dnsmasq to fix routing, then dismiss the warning. */
  async function fixRouting() {
    if (restartingRouting) return;
    restartingRouting = true;
    try {
      if (routingWarning === "caddy" || routingWarning === "both") {
        await safeInvoke("restart_caddy");
      }
      if (routingWarning === "dnsmasq" || routingWarning === "both") {
        await safeInvoke("restart_dnsmasq");
      }
      await sidecars.refresh();
      routingWarning = null;
    } catch {
      // safeInvoke already pushed a toast; leave the warning visible.
    } finally {
      restartingRouting = false;
    }
  }

  async function openUrl(e: MouseEvent) {
    e.stopPropagation();
    try {
      await safeInvoke("open_project", { id: project.id });
    } catch {
      /* toast already pushed */
    }
  }

  async function revealInFinder(e: MouseEvent) {
    e.stopPropagation();
    try {
      // Reveals the project inside its parent (e.g. `Sites/` with the project
      // folder highlighted), which matches what most users expect from "Reveal
      // in Finder". `reveal_in_finder` also surfaces a toast on failure.
      await safeInvoke("reveal_in_finder", { path: project.path });
    } catch {
      /* safeInvoke already pushed the toast */
    }
  }
</script>

<tr
  onclick={() => projects.select(project.id)}
  data-selected={isSelected}
  class="border-b border-border text-sm cursor-pointer transition-colors
         hover:bg-surface-2
         data-[selected=true]:bg-accent/10
         data-[selected=true]:ring-1 data-[selected=true]:ring-inset
         data-[selected=true]:ring-accent/40"
>
  <!-- Project: avatar + name + group subtitle -->
  <td class={cellClass}>
    <div class="flex items-center gap-3 min-w-0">
      <ProjectAvatar
        id={project.id}
        name={project.name}
        type={project.type}
        size={32}
      />
      <div class="min-w-0 leading-tight">
        <p class="text-[13.5px] font-semibold text-fg truncate">
          {project.name}
        </p>
        {#if groupSubtitle}
          <p class="text-[11px] text-fg-subtle truncate">
            {groupSubtitle}
          </p>
        {/if}
      </div>
    </div>
  </td>

  <!-- Stack -->
  <td class={cellClass}>
    <div class="flex items-center gap-2 text-fg-muted text-[12px]">
      <StackIcon type={project.type} size={16} />
      <span class="truncate">{typeLabel[project.type]}</span>
      {#if server}
        <span
          class="shrink-0 px-1.5 py-0.5 rounded bg-surface-2 text-fg-subtle
                 text-[10.5px] border border-border/50"
          title="Served by {webServerLabel[server]}"
        >
          {webServerLabel[server]}
        </span>
      {/if}
      {#if project.sandboxed}
        <span
          class="shrink-0 inline-flex items-center gap-1 px-1.5 py-0.5 rounded
                 bg-accent/10 text-accent text-[10.5px] border border-accent/30"
          title="Running with PortBay sandbox profile"
        >
          <Icon name="shield" size={11} /> Sandbox
        </span>
      {/if}
    </div>
  </td>

  <!-- URL -->
  <td class={cellClass}>
    <button
      type="button"
      onclick={openUrl}
      class="inline-flex items-center gap-1 text-[12px] text-accent
             hover:text-accent-hover hover:underline truncate"
      title="Open {project.url}"
    >
      <span class="truncate">{project.url}</span>
      <Icon name="external-link" size={11} class="shrink-0 opacity-70" />
    </button>
  </td>

  <!-- Port -->
  <td class="{cellClass} text-fg-muted font-mono text-[12px] tabular-nums">
    {project.port ?? "—"}
  </td>

  <!-- Status -->
  <td class={cellClass}>
    <span class="inline-flex items-center gap-1.5 text-[12px]">
      <StatusDot status={display} size="md" />
      <span
        class="text-fg-muted"
        class:text-status-running={display === "running"}
        class:text-status-unhealthy={display === "unhealthy" ||
          display === "port_conflict"}
        class:text-status-crashed={display === "crashed"}
      >
        {statusText}
      </span>
    </span>
  </td>

  <!--
    Actions cell — secondary icon strip (Open URL, Reveal, Open in)
    followed by the primary start/stop button and the overflow menu.
    The secondary icons used to live in the right rail only; with the
    rail now hidden by default they earn a spot in the row so common
    actions are one click from idle.
  -->
  <td class={cellClass}>
    <div class="flex items-center gap-0.5 justify-end">
      <button
        type="button"
        onclick={openUrl}
        title="Open in browser"
        aria-label="Open {project.url} in browser"
        class="inline-flex items-center justify-center w-7 h-7 rounded-md
               text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
      >
        <Icon name="globe" size={13} />
      </button>

      <button
        type="button"
        onclick={revealInFinder}
        title="Reveal in Finder"
        aria-label="Reveal {project.name} in Finder"
        class="inline-flex items-center justify-center w-7 h-7 rounded-md
               text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
      >
        <Icon name="folder" size={13} />
      </button>

      <div onclick={(e) => e.stopPropagation()} role="presentation">
        <OpenInButton projectId={project.id} variant="icon" />
      </div>

      <span class="w-px h-5 bg-border/60 mx-1" aria-hidden="true"></span>

      {#if buttonMode === "stop" || buttonMode === "stopping"}
        <button
          type="button"
          onclick={(e) => {
            e.stopPropagation();
            void run("stop");
          }}
          disabled={busy !== null}
          title="Stop {project.name}"
          aria-label="Stop {project.name}"
          class="inline-flex items-center justify-center w-8 h-8 rounded-md
                 text-on-accent bg-status-crashed hover:brightness-110
                 active:brightness-95 disabled:opacity-50 transition"
        >
          {#if buttonMode === "stopping"}
            <Icon name="refresh-cw" size={12} class="animate-spin" />
          {:else}
            <Icon name="square" size={11} class="fill-current" />
          {/if}
        </button>
      {:else}
        <button
          type="button"
          onclick={(e) => {
            e.stopPropagation();
            void run("start");
          }}
          disabled={busy !== null}
          title="Start {project.name}"
          aria-label="Start {project.name}"
          class="inline-flex items-center justify-center w-8 h-8 rounded-md
                 text-on-accent bg-status-running hover:brightness-110
                 active:brightness-95 disabled:opacity-50 transition"
        >
          {#if buttonMode === "starting"}
            <Icon name="refresh-cw" size={12} class="animate-spin" />
          {:else}
            <Icon name="play" size={12} class="fill-current" />
          {/if}
        </button>
      {/if}

      <ProjectRowMenu {project} />
    </div>
  </td>
</tr>

<!-- Inline error envelope -->
{#if inlineError}
  <tr
    class="bg-surface-2/50"
    onclick={(e) => e.stopPropagation()}
  >
    <td colspan="6" class="px-4 py-2">
      <div class="flex items-start gap-2">
        <div class="flex-1 min-w-0">
          <ErrorEnvelope envelope={inlineError} tone="inline" />
        </div>
        <button
          type="button"
          onclick={() => projects.clearError(project.id)}
          title="Dismiss error"
          aria-label="Dismiss inline error"
          class="shrink-0 mt-1 p-1 rounded-md text-fg-subtle hover:text-fg hover:bg-surface-2 transition-colors"
        >
          <Icon name="x" size={14} />
        </button>
      </div>
    </td>
  </tr>
{/if}

<!-- Web-server setup advisory (derived; no dismiss — clears when fixed) -->
{#if webServerWarning}
  <tr
    class="bg-surface-2/50"
    onclick={(e) => e.stopPropagation()}
  >
    <td colspan="6" class="px-4 py-2">
      <ErrorEnvelope envelope={webServerWarning} tone="inline" />
    </td>
  </tr>
{/if}

<!-- Routing warning: project started but Caddy/dnsmasq is down -->
{#if routingWarning}
  {@const svcLabel = routingWarning === "both"
    ? "Caddy and dnsmasq are"
    : routingWarning === "caddy"
      ? "Caddy is"
      : "dnsmasq is"}
  <tr
    class="bg-surface-2/50"
    onclick={(e) => e.stopPropagation()}
  >
    <td colspan="6" class="px-4 py-2">
      <div class="flex items-start gap-2">
        <div class="flex-1 min-w-0">
          <ErrorEnvelope
            envelope={{
              code: "ROUTING_DOWN",
              whatHappened: `Started, but ${svcLabel} down — ${project.hostname} won't resolve.`,
              whyItMatters: "Your project is running but the local DNS or reverse proxy isn't, so the .test URL will time out in the browser.",
              whoCausedIt: "system",
              category: "infrastructure",
              severity: "warning",
              actions: [],
            }}
            tone="inline"
          />
        </div>
        <div class="flex shrink-0 items-center gap-1.5 mt-1">
          <button
            type="button"
            onclick={fixRouting}
            disabled={restartingRouting}
            class="inline-flex items-center gap-1 px-2 py-1 rounded-md text-xs
                   border border-border text-fg hover:border-border-strong
                   hover:bg-surface-2 disabled:opacity-50 transition-colors"
          >
            {#if restartingRouting}
              <Icon name="refresh-cw" size={11} class="animate-spin" /> Restarting…
            {:else}
              <Icon name="refresh-cw" size={11} /> Restart {routingWarning === "caddy" ? "Caddy" : routingWarning === "dnsmasq" ? "dnsmasq" : "both"}
            {/if}
          </button>
          <button
            type="button"
            onclick={() => (routingWarning = null)}
            title="Dismiss"
            aria-label="Dismiss routing warning"
            class="p-1 rounded-md text-fg-subtle hover:text-fg hover:bg-surface-2 transition-colors"
          >
            <Icon name="x" size={13} />
          </button>
        </div>
      </div>
    </td>
  </tr>
{/if}
