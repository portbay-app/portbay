<!--
  SshProblems — the Problems tab, ported from Lapce's Problems panel but adapted
  honestly to a remote host. Lapce feeds its panel from LSP diagnostics on open
  files; the SSH workspace has neither, so this surfaces a host-health digest
  instead: failed systemd units, disk pressure, high load, and memory pressure,
  gathered with one cheap probe over the cached exec session (the same
  `ssh_exec_run` path Processes/GPU/Logs use) and parsed in `hostProblems.ts`.

  Layout mirrors Lapce: severity sections (Errors, Warnings) → grouped by source
  with a collapsible header → entries with a severity icon, message, and detail.
  Only real, detected problems are shown; an unavailable probe (e.g. no
  systemctl) just contributes nothing rather than a fake "all clear" row.
-->
<script lang="ts">
  import Icon from "$lib/components/atoms/Icon.svelte";
  import { invokeQuiet } from "$lib/ipc";
  import { relativeTime } from "$lib/ssh/hostFormat";
  import {
    HOST_PROBLEMS_PROBE,
    parseHostProblems,
    type HostProblem,
    type ProblemSeverity,
  } from "$lib/ssh/hostProblems";
  import { connectWithPrompt } from "$lib/ssh/connectWithPrompt";
  import type { ExecResult } from "$lib/types/sshTunnels";

  let {
    connectionId,
    label,
    active = false,
  }: { connectionId: string; label: string; active?: boolean } = $props();

  let problems = $state<HostProblem[]>([]);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let stampedAt = $state<number | null>(null);
  // Collapsed source groups, keyed `${severity}:${source}`.
  let collapsed = $state<Record<string, boolean>>({});

  const errorCount = $derived(problems.filter((p) => p.severity === "error").length);
  const warningCount = $derived(problems.filter((p) => p.severity === "warning").length);

  // Sections in Lapce's order (Errors then Warnings), each grouping its problems
  // by source so related issues collapse together.
  interface Group {
    source: string;
    items: HostProblem[];
  }
  interface Section {
    severity: ProblemSeverity;
    label: string;
    groups: Group[];
  }
  const sections = $derived.by<Section[]>(() => {
    const order: { severity: ProblemSeverity; label: string }[] = [
      { severity: "error", label: "Errors" },
      { severity: "warning", label: "Warnings" },
    ];
    return order
      .map(({ severity, label: secLabel }) => {
        const inSeverity = problems.filter((p) => p.severity === severity);
        const bySource = new Map<string, HostProblem[]>();
        for (const p of inSeverity) {
          const list = bySource.get(p.source) ?? [];
          list.push(p);
          bySource.set(p.source, list);
        }
        return {
          severity,
          label: secLabel,
          groups: [...bySource.entries()].map(([source, items]) => ({ source, items })),
        };
      })
      .filter((s) => s.groups.length > 0);
  });

  async function refresh() {
    if (loading) return;
    loading = true;
    error = null;
    try {
      const result = await connectWithPrompt(connectionId, label, (cred) =>
        invokeQuiet<ExecResult>("ssh_exec_run", {
          input: {
            connectionId,
            command: HOST_PROBLEMS_PROBE,
            password: cred?.kind === "password" ? cred.secret : undefined,
            passphrase: cred?.kind === "passphrase" ? cred.secret : undefined,
          },
        }),
      );
      problems = parseHostProblems(result.stdout ?? "");
      stampedAt = Math.floor(Date.now() / 1000);
    } catch {
      /* connectWithPrompt already toasted the real failure */
      error = "Couldn't run the host-health checks.";
    } finally {
      loading = false;
    }
  }

  // Auto-run the checks the first time the tab is opened, then latch — toggling
  // back keeps the snapshot (re-run with Refresh). The session is already warm
  // from the workspace, so this adds no extra credential prompt.
  let autoLoaded = false;
  $effect(() => {
    if (active && !autoLoaded) {
      autoLoaded = true;
      void refresh();
    }
  });

  function toggleGroup(severity: ProblemSeverity, source: string) {
    const key = `${severity}:${source}`;
    collapsed = { ...collapsed, [key]: !collapsed[key] };
  }
</script>

<div class="flex h-full min-h-0 flex-col">
  <header class="flex items-center gap-2 border-b border-border/60 px-6 py-3">
    <Icon name="alert-triangle" size={15} class="text-fg-muted" />
    <div class="min-w-0 flex-1">
      <h2 class="text-[13px] font-semibold text-fg">Problems</h2>
      <p class="text-[11px] text-fg-subtle">
        {#if stampedAt}
          {#if problems.length === 0}
            No problems detected · {relativeTime(stampedAt)}
          {:else}
            {errorCount} error{errorCount === 1 ? "" : "s"}, {warningCount} warning{warningCount === 1 ? "" : "s"} · {relativeTime(stampedAt)}
          {/if}
        {:else}
          Host-health digest — systemd, disk, load, memory
        {/if}
      </p>
    </div>
    <button
      type="button"
      onclick={refresh}
      disabled={loading}
      class="inline-flex h-7 items-center gap-1.5 rounded-md px-2 text-[12px] text-fg-muted hover:bg-surface-2 hover:text-fg disabled:opacity-50"
      title="Re-run host-health checks"
    >
      <Icon name="refresh-cw" size={13} class={loading ? "animate-spin" : ""} /> Refresh
    </button>
  </header>

  <div class="min-h-0 flex-1 overflow-y-auto">
    {#if error}
      <div class="m-4 rounded-md border border-status-crashed/40 bg-status-crashed/10 p-3 text-[12px] text-status-crashed">
        {error}
      </div>
    {:else if loading && problems.length === 0}
      <p class="p-6 text-center text-[12px] text-fg-subtle">Running host-health checks…</p>
    {:else if problems.length === 0}
      <div class="flex flex-col items-center justify-center gap-2 px-6 py-12 text-center">
        <Icon name="circle-check" size={22} class="text-status-running" />
        <p class="text-[12.5px] text-fg-muted">
          {stampedAt ? "No problems detected on this host." : "Open to run the host-health checks."}
        </p>
        <p class="max-w-xs text-[11px] text-fg-subtle">
          Checks failed systemd units, disk and memory pressure, and load. Probes
          a host doesn't support are skipped, not reported as problems.
        </p>
      </div>
    {:else}
      {#each sections as section (section.severity)}
        <div class="px-2 py-2">
          <div class="flex items-center gap-1.5 px-2 py-1 text-[11px] font-semibold uppercase tracking-wide text-fg-subtle">
            <Icon
              name={section.severity === "error" ? "circle-alert" : "alert-triangle"}
              size={12}
              class={section.severity === "error" ? "text-status-crashed" : "text-status-unhealthy"}
            />
            {section.label}
            <span class="font-normal normal-case text-fg-muted">
              {section.groups.reduce((n, g) => n + g.items.length, 0)}
            </span>
          </div>

          {#each section.groups as group (group.source)}
            {@const key = `${section.severity}:${group.source}`}
            {@const isCollapsed = collapsed[key]}
            <button
              type="button"
              onclick={() => toggleGroup(section.severity, group.source)}
              class="flex w-full items-center gap-1 rounded px-2 py-1 text-left text-[12px] text-fg-muted hover:bg-surface-2/60"
            >
              <Icon name={isCollapsed ? "chevron-right" : "chevron-down"} size={12} class="shrink-0 text-fg-subtle" />
              <span class="font-medium text-fg">{group.source}</span>
              <span class="text-fg-subtle">({group.items.length})</span>
            </button>

            {#if !isCollapsed}
              <ul class="mb-1">
                {#each group.items as p, i (i)}
                  <li class="flex items-start gap-2 rounded px-2 py-1.5 pl-7 hover:bg-surface-2/40">
                    <Icon
                      name={p.severity === "error" ? "circle-alert" : "alert-triangle"}
                      size={13}
                      class={`mt-0.5 shrink-0 ${p.severity === "error" ? "text-status-crashed" : "text-status-unhealthy"}`}
                    />
                    <div class="min-w-0 flex-1">
                      <p class="text-[12.5px] text-fg">{p.title}</p>
                      {#if p.detail}
                        <p class="mt-0.5 break-words font-mono text-[11px] text-fg-subtle">{p.detail}</p>
                      {/if}
                      {#if p.hint}
                        <p class="mt-0.5 text-[11px] text-fg-muted">{p.hint}</p>
                      {/if}
                    </div>
                  </li>
                {/each}
              </ul>
            {/if}
          {/each}
        </div>
      {/each}
    {/if}
  </div>
</div>
