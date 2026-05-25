<!--
  /sandbox — the projects PortBay is running in isolation.

  PortBay's sandbox is NOT a "run any GitHub repo in the cloud" box. It's a
  local guarantee: a project flagged `sandboxed` runs under an OS-level wrapper
  with a network policy you pick, so untrusted or experimental code can't read
  your files, reach your other PortBay services, or call home unless you allow
  it. The toggle + policy live on each project's detail panel (Pro); this page
  is the roll-up — every sandboxed project, what its policy actually permits,
  and what the sandbox has *blocked* (the one thing competitors never show).

  Data: `list_projects` (the projects store) filtered to `sandboxed`. Per-card
  actions reuse the same IPC the detail panel already drives:
  `start_project_sandboxed`, `stop_project`, `open_project`,
  `promote_project_to_local`, and `sandbox_violations`.
-->
<script lang="ts">
  import { onMount } from "svelte";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import type { IconName } from "$lib/components/atoms/Icon.svelte";
  import StatusDot from "$lib/components/atoms/StatusDot.svelte";

  import { safeInvoke } from "$lib/ipc";
  import { projects } from "$lib/stores/projects.svelte";
  import { entitlements } from "$lib/stores/entitlements.svelte";
  import { projectDetailPanel } from "$lib/stores/detailPanel.svelte";
  import { addProjectWizard } from "$lib/stores/wizard.svelte";
  import { errorBus } from "$lib/stores/errors.svelte";
  import { dns } from "$lib/stores/dns.svelte";
  import { displayStatusLabel } from "$lib/types/status";
  import type {
    ProjectView,
    SandboxNetworkPolicy,
  } from "$lib/types/projects";

  // Only sandboxed projects appear here. A project can be sandboxed and
  // stopped — the row still belongs on this page.
  const sandboxed = $derived<ProjectView[]>(
    projects.value.filter((p) => p.sandboxed),
  );

  const runningCount = $derived(
    sandboxed.filter((p) => p.status === "running").length,
  );

  onMount(() => {
    // start() is idempotent (the root layout already owns the status
    // listener); calling it here just guarantees a fresh list + live dots
    // if the user deep-links straight to /sandbox.
    void projects.start();
  });

  // ── Network policy: the honest description of what each level permits ──────
  // Ordered from most-isolated to least so the UI can colour "tightness".
  type Iso = "tight" | "moderate" | "open";

  const POLICY: Record<
    SandboxNetworkPolicy,
    { label: string; blurb: string; icon: IconName; iso: Iso }
  > = {
    blocked: {
      label: "Blocked",
      blurb: "No network at all — not even loopback. Fully air-gapped.",
      icon: "lock",
      iso: "tight",
    },
    loopback_only: {
      label: "Loopback only",
      blurb:
        "Reaches 127.0.0.1 and nothing else — no internet, no LAN, no other PortBay services.",
      icon: "shield",
      iso: "tight",
    },
    outbound: {
      label: "Outbound",
      blurb: "Can reach the internet, but not your other local services.",
      icon: "globe",
      iso: "moderate",
    },
    full: {
      label: "Full",
      blurb: "Unrestricted — the same network access a normal local project has.",
      icon: "globe",
      iso: "open",
    },
  };

  function policyOf(p: ProjectView): SandboxNetworkPolicy {
    return p.sandbox?.network ?? "loopback_only";
  }

  // Green = well-contained, amber = the loosest policy (a heads-up, not an
  // error). Neutral sits in between. Reuses the shared status tokens.
  const isoDot: Record<Iso, string> = {
    tight: "bg-status-running",
    moderate: "bg-fg-subtle",
    open: "bg-status-unhealthy",
  };
  const isoText: Record<Iso, string> = {
    tight: "text-status-running",
    moderate: "text-fg-muted",
    open: "text-status-unhealthy",
  };
  const isoTint: Record<Iso, string> = {
    tight: "bg-status-running/12",
    moderate: "bg-surface-2",
    open: "bg-status-unhealthy/12",
  };

  // ── Formatting ─────────────────────────────────────────────────────────
  function fmtUptime(ageNs: number): string {
    const s = Math.floor(ageNs / 1_000_000_000);
    if (s < 60) return `${s}s`;
    const m = Math.floor(s / 60);
    if (m < 60) return `${m}m ${s % 60}s`;
    const h = Math.floor(m / 60);
    if (h < 24) return `${h}h ${m % 60}m`;
    return `${Math.floor(h / 24)}d ${h % 24}h`;
  }

  function fmtMem(bytes: number): string {
    const mb = bytes / 1024 / 1024;
    return mb >= 1024 ? `${(mb / 1024).toFixed(1)} GB` : `${Math.round(mb)} MB`;
  }

  function allPorts(p: ProjectView): number[] {
    return [p.port, ...p.extraPorts].filter(
      (n): n is number => typeof n === "number",
    );
  }

  // ── Secret masking — a sandbox detail view shouldn't leak credentials at a
  // glance. Anything that looks like a secret is masked until revealed. ─────
  const SECRET_KEY = /(SECRET|TOKEN|PASSWORD|PASSWD|KEY|DSN|CREDENTIAL|PRIVATE|_URL$|URL$)/i;
  let revealed = $state<Record<string, boolean>>({});

  function isSecret(key: string): boolean {
    return SECRET_KEY.test(key);
  }
  function revealKey(pid: string, key: string): string {
    return `${pid}::${key}`;
  }

  // ── Actions ──────────────────────────────────────────────────────────────
  let busy = $state<string | null>(null);

  async function start(p: ProjectView) {
    if (busy) return;
    busy = p.id;
    projects.beginTransition(p.id, "start");
    try {
      await dns.ensureReady();
      await safeInvoke("start_project_sandboxed", {
        id: p.id,
        options: {
          network: policyOf(p),
          ephemeral: p.sandbox?.ephemeral ?? false,
        },
      });
    } catch {
      projects.failTransition(p.id);
      /* safeInvoke already pushed a toast */
    } finally {
      busy = null;
    }
  }

  async function stop(p: ProjectView) {
    if (busy) return;
    busy = p.id;
    projects.beginTransition(p.id, "stop");
    try {
      await safeInvoke("stop_project", { id: p.id });
    } catch {
      projects.failTransition(p.id);
    } finally {
      busy = null;
    }
  }

  function openInBrowser(p: ProjectView) {
    void safeInvoke("open_project", { id: p.id }).catch(() => {});
  }

  function openDetails(p: ProjectView) {
    // The detail panel is where the network policy + ephemeral flag are
    // edited (Pro). Keep that the single source of truth.
    projectDetailPanel.show(p.id);
  }

  async function promote(p: ProjectView) {
    if (busy) return;
    busy = p.id;
    try {
      await safeInvoke("promote_project_to_local", { id: p.id });
      await projects.refresh();
      errorBus.push({
        code: "SANDBOX_PROMOTED",
        whatHappened: `${p.name} will run locally on the next start.`,
        whyItMatters: "The sandbox wrapper was removed from this project.",
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });
    } catch {
      /* toast already pushed */
    } finally {
      busy = null;
    }
  }

  // ── Violations — what the sandbox actually blocked. Lazy-loaded per card. ──
  type Violations = { loading: boolean; lines: string[] | null };
  let violations = $state<Record<string, Violations>>({});

  async function loadViolations(p: ProjectView) {
    violations[p.id] = { loading: true, lines: violations[p.id]?.lines ?? null };
    try {
      const lines = await safeInvoke<string[]>("sandbox_violations", {
        id: p.id,
        limit: 250,
      });
      violations[p.id] = { loading: false, lines: lines ?? [] };
    } catch {
      violations[p.id] = { loading: false, lines: [] };
    }
  }

  function addProject() {
    addProjectWizard.requestAdd();
  }
</script>

<div class="h-full overflow-y-auto">
  <div class="max-w-5xl mx-auto px-6 py-6 space-y-5">
    <!-- Header -->
    <header class="flex items-start gap-4 flex-wrap">
      <div class="min-w-0 flex-1">
        <h1 class="text-[20px] font-semibold text-fg leading-none">Sandbox</h1>
        <p class="mt-1.5 text-[12.5px] text-fg-subtle leading-relaxed max-w-[60ch]">
          Projects you run in isolation. Each one gets its own process and a
          network policy you choose — so untrusted or experimental code can't
          read your files or reach your other services unless you let it.
        </p>
      </div>

      <button
        type="button"
        onclick={addProject}
        class="shrink-0 inline-flex items-center gap-1.5 h-8 px-3 rounded-lg
               text-[12px] font-medium bg-accent text-on-accent
               hover:brightness-110 active:scale-[0.98] transition"
      >
        <Icon name="plus" size={14} />
        Add project
      </button>
    </header>

    {#if sandboxed.length === 0}
      <!-- Empty state — explain the concept and the real way to enter it. -->
      <div
        class="rounded-xl border border-dashed border-border px-6 py-12 text-center"
      >
        <span
          class="inline-grid place-items-center w-12 h-12 rounded-xl bg-surface-2 text-fg-subtle mx-auto"
        >
          <Icon name="package" size={24} />
        </span>
        <p class="mt-3 text-[13px] font-medium text-fg">
          Nothing is sandboxed yet
        </p>
        <p class="mt-1.5 text-[12px] text-fg-subtle leading-relaxed max-w-md mx-auto">
          Open any project, choose <span class="text-fg-muted">Run in Sandbox</span>,
          pick a network policy, and start it. It'll show up here with a live
          view of what the sandbox is letting through — and what it's blocking.
          {#if !entitlements.isPro}
            <span class="block mt-2 text-fg-subtle">
              Free accounts can sandbox up to 2 projects — Pro is unlimited.
            </span>
          {/if}
        </p>
        <button
          type="button"
          onclick={addProject}
          class="mt-4 inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg
                 text-[12.5px] text-status-running border border-status-running/40
                 hover:bg-status-running/10 hover:border-status-running/60
                 active:scale-[0.98] transition"
        >
          <Icon name="plus" size={14} />
          Add project
        </button>
      </div>
    {:else}
      <!-- Summary line -->
      <div
        class="flex items-center gap-x-5 gap-y-1 flex-wrap text-[12px] text-fg-subtle"
      >
        <span class="inline-flex items-center gap-1.5">
          <Icon name="package" size={13} class="text-fg-subtle" />
          <span class="text-fg font-medium tabular-nums">{sandboxed.length}</span>
          sandboxed
        </span>
        <span class="inline-flex items-center gap-1.5">
          <span
            class="inline-block w-1.5 h-1.5 rounded-full bg-status-running"
            aria-hidden="true"
          ></span>
          <span class="text-fg font-medium tabular-nums">{runningCount}</span>
          running
        </span>
      </div>

      <!-- One card per sandboxed project -->
      <div class="space-y-4">
        {#each sandboxed as p, i (p.id)}
          {@const policy = POLICY[policyOf(p)]}
          {@const ports = allPorts(p)}
          {@const envEntries = Object.entries(p.env)}
          {@const display = projects.displayStatusOf(p)}
          {@const isRunning = p.status === "running"}
          {@const v = violations[p.id]}
          <article
            class="sb-card rounded-xl border border-border bg-surface overflow-hidden"
            style="--i:{i}"
          >
            <!-- Card header -->
            <div class="flex items-start gap-4 px-5 pt-5 pb-4">
              <span
                class="shrink-0 grid place-items-center w-11 h-11 rounded-xl {isoTint[
                  policy.iso
                ]} {isoText[policy.iso]}"
                title="Network policy: {policy.label}"
              >
                <Icon name={policy.icon} size={20} />
              </span>

              <div class="min-w-0 flex-1">
                <div class="flex items-center gap-2.5 flex-wrap">
                  <h2 class="text-[15px] font-semibold text-fg leading-none truncate">
                    {p.name}
                  </h2>
                  <span
                    class="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-md
                           text-[11px] font-medium bg-surface-2 text-fg-muted"
                  >
                    <StatusDot status={display} size="sm" />
                    {displayStatusLabel(display)}
                  </span>
                  {#if p.sandbox?.ephemeral}
                    <span
                      class="inline-flex items-center gap-1 px-1.5 py-0.5 rounded
                             text-[10px] font-semibold uppercase tracking-wide
                             bg-surface-2 text-fg-subtle"
                      title="Temp & cache are wiped before every start"
                    >
                      <Icon name="zap" size={10} />
                      Ephemeral
                    </span>
                  {/if}
                </div>
                <p class="mt-1 text-[11.5px] font-mono text-fg-subtle truncate">
                  {p.hostname}
                </p>
              </div>

              <!-- Actions -->
              <div class="shrink-0 flex items-center gap-1">
                {#if isRunning}
                  <button
                    type="button"
                    onclick={() => stop(p)}
                    disabled={busy === p.id}
                    title="Stop sandbox"
                    aria-label="Stop {p.name}"
                    class="p-1.5 rounded-md text-fg-subtle hover:text-fg hover:bg-surface-2
                           active:scale-95 transition disabled:opacity-50"
                  >
                    <Icon name="square" size={14} />
                  </button>
                  <button
                    type="button"
                    onclick={() => openInBrowser(p)}
                    title="Open {p.url}"
                    aria-label="Open {p.name} in browser"
                    class="p-1.5 rounded-md text-fg-subtle hover:text-fg hover:bg-surface-2
                           active:scale-95 transition"
                  >
                    <Icon name="external-link" size={14} />
                  </button>
                {:else}
                  <button
                    type="button"
                    onclick={() => start(p)}
                    disabled={busy === p.id}
                    title="Start in sandbox"
                    aria-label="Start {p.name} in sandbox"
                    class="inline-flex items-center gap-1.5 h-7 px-2.5 rounded-md
                           text-[12px] font-medium text-status-running
                           border border-status-running/40 hover:bg-status-running/10
                           active:scale-[0.98] transition disabled:opacity-50"
                  >
                    <Icon name="play" size={12} />
                    Start
                  </button>
                {/if}
                <button
                  type="button"
                  onclick={() => openDetails(p)}
                  title="Open project details"
                  aria-label="Open details for {p.name}"
                  class="p-1.5 rounded-md text-fg-subtle hover:text-fg hover:bg-surface-2
                         active:scale-95 transition"
                >
                  <Icon name="settings" size={14} />
                </button>
              </div>
            </div>

            <!-- Network policy banner — the honest "what this actually does" -->
            <div
              class="mx-5 mb-4 flex items-start gap-2.5 rounded-lg {isoTint[
                policy.iso
              ]} px-3 py-2.5"
            >
              <span
                class="mt-px inline-block w-1.5 h-1.5 rounded-full shrink-0 {isoDot[
                  policy.iso
                ]}"
                aria-hidden="true"
              ></span>
              <p class="text-[12px] leading-relaxed">
                <span class="font-semibold {isoText[policy.iso]}">
                  {policy.label}
                </span>
                <span class="text-fg-muted"> — {policy.blurb}</span>
              </p>
            </div>

            <!-- Facts + the three columns: Services · Ports · Environment -->
            <div class="border-t border-border/60 grid grid-cols-1 sm:grid-cols-3 divide-y sm:divide-y-0 sm:divide-x divide-border/60">
              <!-- Services -->
              <section class="px-5 py-4 min-w-0">
                <h3 class="text-[11px] font-medium uppercase tracking-wide text-fg-subtle">
                  Services
                </h3>
                {#if p.services.length > 0}
                  <ul class="mt-2.5 space-y-1.5">
                    {#each p.services as svc (svc)}
                      <li class="flex items-center gap-2 text-[12.5px] text-fg">
                        <StatusDot status={display} size="sm" />
                        <span class="truncate">{svc}</span>
                      </li>
                    {/each}
                  </ul>
                {:else}
                  <p class="mt-2.5 text-[12px] text-fg-subtle">
                    Single process — no extra services.
                  </p>
                {/if}
                <dl class="mt-3 pt-3 border-t border-border/60 space-y-1.5 text-[11.5px]">
                  <div class="flex items-center justify-between gap-3">
                    <dt class="text-fg-subtle">Type</dt>
                    <dd class="font-mono text-fg-muted">{p.type}</dd>
                  </div>
                  {#if p.runtime}
                    <div class="flex items-center justify-between gap-3">
                      <dt class="text-fg-subtle">Uptime</dt>
                      <dd class="font-mono tabular-nums text-fg-muted">
                        {fmtUptime(p.runtime.age)}
                      </dd>
                    </div>
                    <div class="flex items-center justify-between gap-3">
                      <dt class="text-fg-subtle">Memory</dt>
                      <dd class="font-mono tabular-nums text-fg-muted">
                        {fmtMem(p.runtime.memBytes)}
                      </dd>
                    </div>
                  {/if}
                </dl>
              </section>

              <!-- Ports -->
              <section class="px-5 py-4 min-w-0">
                <h3 class="text-[11px] font-medium uppercase tracking-wide text-fg-subtle">
                  Ports
                </h3>
                {#if ports.length > 0}
                  <ul class="mt-2.5 space-y-2">
                    {#each ports as port, idx (port)}
                      <li class="flex items-center gap-2 text-[12.5px]">
                        <StatusDot status={display} size="sm" />
                        <span class="font-mono tabular-nums text-fg">{port}</span>
                        <span
                          class="text-[9.5px] font-semibold uppercase tracking-wide
                                 px-1 py-px rounded bg-surface-2 text-fg-subtle"
                        >
                          {p.https ? "HTTPS" : "HTTP"}
                        </span>
                        {#if idx === 0}
                          <span class="ml-auto font-mono text-[11px] text-fg-subtle truncate">
                            {p.hostname}
                          </span>
                        {/if}
                      </li>
                    {/each}
                  </ul>
                {:else}
                  <p class="mt-2.5 text-[12px] text-fg-subtle">
                    No listening ports.
                  </p>
                {/if}
              </section>

              <!-- Environment -->
              <section class="px-5 py-4 min-w-0">
                <h3 class="text-[11px] font-medium uppercase tracking-wide text-fg-subtle">
                  Environment
                </h3>
                {#if envEntries.length > 0}
                  <dl class="mt-2.5 space-y-2">
                    {#each envEntries as [key, value] (key)}
                      {@const rk = revealKey(p.id, key)}
                      {@const secret = isSecret(key)}
                      <div class="min-w-0">
                        <dt class="text-[10.5px] font-mono text-fg-subtle truncate">
                          {key}
                        </dt>
                        <dd class="flex items-center gap-1.5 min-w-0">
                          <span class="font-mono text-[11.5px] text-fg-muted truncate">
                            {#if secret && !revealed[rk]}
                              ••••••••••
                            {:else}
                              {value}
                            {/if}
                          </span>
                          {#if secret}
                            <button
                              type="button"
                              onclick={() => (revealed[rk] = !revealed[rk])}
                              title={revealed[rk] ? "Hide value" : "Reveal value"}
                              aria-label={revealed[rk]
                                ? `Hide ${key}`
                                : `Reveal ${key}`}
                              class="shrink-0 p-0.5 rounded text-fg-subtle hover:text-fg
                                     hover:bg-surface-2 active:scale-90 transition"
                            >
                              <Icon
                                name={revealed[rk] ? "eye" : "lock"}
                                size={12}
                              />
                            </button>
                          {/if}
                        </dd>
                      </div>
                    {/each}
                  </dl>
                {:else}
                  <p class="mt-2.5 text-[12px] text-fg-subtle">
                    No environment variables set.
                  </p>
                {/if}
              </section>
            </div>

            <!-- Footer: what the sandbox blocked + promote -->
            <div
              class="border-t border-border/60 px-5 py-3 flex items-center gap-3 flex-wrap"
            >
              {#if v?.lines == null}
                <button
                  type="button"
                  onclick={() => loadViolations(p)}
                  disabled={v?.loading}
                  class="inline-flex items-center gap-1.5 text-[12px] text-fg-muted
                         hover:text-fg active:scale-[0.98] transition disabled:opacity-50"
                >
                  <Icon
                    name={v?.loading ? "refresh-cw" : "shield"}
                    size={13}
                    class={v?.loading ? "animate-spin" : ""}
                  />
                  {v?.loading ? "Checking…" : "Check what was blocked"}
                </button>
              {:else if v.lines.length === 0}
                <span class="inline-flex items-center gap-1.5 text-[12px] text-status-running">
                  <Icon name="check" size={13} />
                  Nothing blocked — clean run.
                </span>
              {:else}
                <div class="w-full min-w-0">
                  <div class="flex items-center gap-1.5 text-[12px] text-status-unhealthy">
                    <Icon name="circle-alert" size={13} />
                    <span class="font-medium tabular-nums">{v.lines.length}</span>
                    blocked connection{v.lines.length === 1 ? "" : "s"}
                  </div>
                  <ul
                    class="mt-2 max-h-32 overflow-y-auto rounded-lg bg-bg border border-border/60
                           px-3 py-2 space-y-1"
                  >
                    {#each v.lines as line, idx (idx)}
                      <li class="font-mono text-[11px] text-fg-muted break-all">
                        {line}
                      </li>
                    {/each}
                  </ul>
                </div>
              {/if}

              <button
                type="button"
                onclick={() => promote(p)}
                disabled={busy === p.id}
                title="Stop sandboxing this project — it'll run locally next time"
                class="ml-auto inline-flex items-center gap-1.5 text-[12px] text-accent
                       hover:underline active:scale-[0.98] transition disabled:opacity-50"
              >
                <Icon name="check" size={13} />
                Promote to local
              </button>
            </div>
          </article>
        {/each}
      </div>
    {/if}
  </div>
</div>

<style>
  /* Cards fade up on first paint, lightly staggered (Emil: ease-out, <320ms,
     transform+opacity only). A security view should feel calm, so the motion
     is restrained and disabled entirely under reduced-motion. */
  .sb-card {
    animation: sb-in 300ms cubic-bezier(0.23, 1, 0.32, 1) backwards;
    animation-delay: calc(var(--i) * 45ms);
  }
  @keyframes sb-in {
    from {
      opacity: 0;
      transform: translateY(6px);
    }
    to {
      opacity: 1;
      transform: none;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .sb-card {
      animation: none;
    }
  }
</style>
