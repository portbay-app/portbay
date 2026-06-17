<!--
  /inspector — HTTP request inspector.

  A live, DevTools-Network-style view of the traffic flowing through Caddy:
  method, host + path, status, latency, and the matched project. Streams from
  the `portbay://request` events the Rust tailer emits off Caddy's JSON access
  log; backfills from `recent_requests` on open. Filters (project, errors-only,
  path substring) are local; rendering is capped so heavy traffic can't lag.
-->
<script lang="ts">
  import { onMount } from "svelte";

  import { Icon, EmptyState } from "$lib/components/atoms";
  import ProjectSelector from "$lib/components/shared/ProjectSelector.svelte";
  import { httpInspector } from "$lib/stores/httpInspector.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import type { RequestEntry } from "$lib/types/inspector";

  // Local filter state — bound directly to the controls.
  let projectFilter = $state<string>("");
  let errorsOnly = $state<boolean>(false);
  let pathQuery = $state<string>("");

  // The currently-expanded row (by object identity — entries are stable).
  let expanded = $state<RequestEntry | null>(null);

  // Cap the rendered rows so a burst of traffic never janks the table.
  const RENDER_CAP = 500;

  const rendered = $derived.by(() => {
    const q = pathQuery.trim().toLowerCase();
    const out = httpInspector.entries.filter((e) => {
      if (projectFilter && e.projectId !== projectFilter) return false;
      if (errorsOnly && e.status < 400) return false;
      if (q && !e.uri.toLowerCase().includes(q) && !e.host.toLowerCase().includes(q))
        return false;
      return true;
    });
    out.reverse(); // newest first
    return out.slice(0, RENDER_CAP);
  });

  const projectName = $derived.by(() => {
    const map = new Map(projects.value.map((p) => [p.id, p.name]));
    return (id?: string) => (id ? (map.get(id) ?? id) : "—");
  });

  onMount(() => {
    httpInspector.start();
    return () => httpInspector.stop();
  });

  function statusClass(s: number): string {
    if (s >= 500) return "text-status-crashed";
    if (s >= 400) return "text-status-unhealthy";
    if (s >= 300) return "text-fg-muted";
    return "text-status-running";
  }

  function fmtTime(ts: number): string {
    return new Date(ts).toLocaleTimeString();
  }

  function fmtLatency(ms: number): string {
    if (ms < 1) return "<1 ms";
    return `${ms.toFixed(ms < 100 ? 1 : 0)} ms`;
  }

  function toggle(e: RequestEntry) {
    expanded = expanded === e ? null : e;
  }

  async function clearAll() {
    expanded = null;
    await httpInspector.clear();
  }
</script>

<div class="flex flex-col h-full min-h-0">
  <header class="px-6 pt-6 pb-3 shrink-0">
    <div class="flex items-center justify-between gap-3">
      <div>
        <h1 class="text-lg font-semibold text-fg flex items-center gap-2">
          <Icon name="activity" size={18} />
          Request Inspector
        </h1>
        <p class="text-[12.5px] text-fg-subtle mt-0.5">
          Live HTTP traffic through Caddy — method, status, latency, and the
          matched project.
        </p>
      </div>
      <button
        type="button"
        onclick={clearAll}
        class="inline-flex items-center gap-1.5 px-2.5 py-1.5 rounded-md
               text-[12px] text-fg-muted hover:text-fg bg-surface-2/60
               hover:bg-surface-2 border border-border/60 transition-colors"
      >
        <Icon name="refresh-cw" size={12} />
        Clear
      </button>
    </div>

    <!-- Filters -->
    <div class="flex flex-wrap items-center gap-2 mt-3">
      <ProjectSelector
        projects={projects.value}
        selectedId={projectFilter === "" ? null : projectFilter}
        includeAllOption={true}
        allOptionLabel="All projects"
        onselect={(id) => {
          projectFilter = id ?? "";
        }}
      />

      <label
        class="inline-flex items-center gap-1.5 text-[12px] text-fg-muted
               px-2 py-1 rounded-md border border-border/60 cursor-pointer
               select-none"
      >
        <input type="checkbox" bind:checked={errorsOnly} class="accent-accent" />
        Errors only (≥ 400)
      </label>

      <div class="relative flex-1 min-w-[180px] max-w-sm">
        <Icon
          name="search"
          size={12}
          class="absolute left-2 top-1/2 -translate-y-1/2 text-fg-subtle"
        />
        <input
          type="text"
          bind:value={pathQuery}
          placeholder="Filter by path or host…"
          class="w-full text-[12px] bg-surface-2 border border-border rounded-md
                 pl-7 pr-2 py-1 text-fg placeholder:text-fg-subtle
                 focus:outline-none focus:ring-1 focus:ring-accent/40"
        />
      </div>

      <span class="text-[11.5px] text-fg-subtle tabular-nums ml-auto">
        {rendered.length}
        {#if httpInspector.entries.length > rendered.length}
          / {httpInspector.entries.length}
        {/if}
        request{rendered.length === 1 ? "" : "s"}
      </span>
    </div>
  </header>

  <div class="flex-1 min-h-0 overflow-y-auto px-6 pb-6">
    {#if rendered.length === 0}
      <EmptyState
        icon="activity"
        title="No requests captured yet"
        description="Open one of your projects in the browser — requests routed through Caddy will appear here in real time."
      />
    {:else}
      <div class="rounded-xl border border-border bg-surface overflow-hidden">
        <table class="w-full text-left border-collapse">
        <thead>
          <tr class="text-[11px] uppercase tracking-wide text-fg-subtle">
            <th class="font-medium px-4 py-2">Time</th>
            <th class="font-medium px-4 py-2">Method</th>
            <th class="font-medium px-4 py-2">Host · Path</th>
            <th class="font-medium px-4 py-2 text-right">Status</th>
            <th class="font-medium px-4 py-2 text-right">Latency</th>
            <th class="font-medium px-4 py-2">Project</th>
          </tr>
        </thead>
        <tbody>
          {#each rendered as e (e)}
            <tr
              onclick={() => toggle(e)}
              class="border-t border-border/60 text-[12.5px] cursor-pointer
                     hover:bg-surface-2/60 transition-colors"
            >
              <td class="py-1.5 px-4 font-mono tabular-nums text-fg-subtle whitespace-nowrap">
                {fmtTime(e.ts)}
              </td>
              <td class="py-1.5 px-4 font-mono text-fg-muted">{e.method}</td>
              <td class="py-1.5 px-4 min-w-0">
                <span class="text-fg-subtle">{e.host}</span><span
                  class="text-fg font-mono">{e.uri}</span
                >
              </td>
              <td class="py-1.5 px-4 text-right font-mono tabular-nums {statusClass(e.status)}">
                {e.status}
              </td>
              <td class="py-1.5 px-4 text-right font-mono tabular-nums text-fg-muted whitespace-nowrap">
                {fmtLatency(e.durationMs)}
              </td>
              <td class="py-1.5 px-4 text-fg-muted truncate">{projectName(e.projectId)}</td>
            </tr>
            {#if expanded === e}
              <tr class="bg-surface-2/40">
                <td colspan="6" class="px-4 py-2">
                  <div class="text-[11.5px] space-y-1.5">
                    <div class="flex flex-wrap gap-x-6 gap-y-1 text-fg-muted font-mono">
                      <span>scheme: {e.host}</span>
                      <span>bytes: {e.size}</span>
                      <span>ts: {new Date(e.ts).toISOString()}</span>
                    </div>
                    {#if e.reqHeaders && Object.keys(e.reqHeaders).length > 0}
                      <div>
                        <p class="text-fg-subtle uppercase tracking-wide text-[10px] mb-1">
                          Request headers
                        </p>
                        <div class="font-mono text-[11px] text-fg-muted space-y-0.5">
                          {#each Object.entries(e.reqHeaders) as [k, vals] (k)}
                            <div>
                              <span class="text-fg">{k}:</span>
                              {vals.join(", ")}
                            </div>
                          {/each}
                        </div>
                      </div>
                    {:else}
                      <p class="text-fg-subtle">No request headers logged.</p>
                    {/if}
                  </div>
                </td>
              </tr>
            {/if}
          {/each}
        </tbody>
        </table>
      </div>
    {/if}
  </div>
</div>
