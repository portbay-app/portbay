<!--
  SshGpu — a point-in-time GPU readout for the host: the sibling of Processes,
  but for the thing an ML researcher actually stares at all day. Runs `nvidia-smi`
  over the exec layer (shared credential-prompt flow), parses it, and shows one
  card per GPU — utilization, VRAM, temperature, power, and which process/user
  owns each board. Honest about being a snapshot, not a live `nvitop`: Refresh
  re-runs. Degrades to a clean empty state on hosts without an NVIDIA GPU.
-->
<script lang="ts">
  import Icon from "$lib/components/atoms/Icon.svelte";
  import { connectWithPrompt } from "$lib/ssh/connectWithPrompt";
  import { fetchGpuReadout, type GpuReadout, type GpuStat } from "$lib/ssh/gpuSnapshot";
  import { relativeTime } from "$lib/ssh/hostFormat";
  import { mlWatch } from "$lib/stores/mlWatch.svelte";

  let {
    connectionId,
    label,
    active = false,
  }: { connectionId: string; label: string; active?: boolean } = $props();

  let readout = $state<GpuReadout | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let stampedAt = $state<number | null>(null);

  async function refresh() {
    if (loading) return;
    loading = true;
    error = null;
    try {
      const r = await fetchGpuReadout(connectionId, label);
      readout = r;
      stampedAt = Math.floor(Date.now() / 1000);
      if (!r.available) error = "no-gpu";
    } catch {
      /* connectWithPrompt already toasted the real failure */
    } finally {
      loading = false;
    }
  }

  // Auto-load on first open, like Processes — the session is already warm from
  // the workspace, so this adds no extra prompt. Latched so toggling back keeps
  // the existing snapshot until the user re-runs it.
  let autoLoaded = false;
  $effect(() => {
    if (active && !autoLoaded) {
      autoLoaded = true;
      void refresh();
    }
  });

  function gb(mb: number | null): string {
    if (mb == null) return "—";
    return mb >= 1024 ? `${(mb / 1024).toFixed(1)} GB` : `${mb} MB`;
  }

  function memPct(g: GpuStat): number | null {
    if (g.memTotalMb && g.memUsedMb != null) {
      return Math.round((g.memUsedMb / g.memTotalMb) * 100);
    }
    return null;
  }

  // VRAM is the OOM constraint, so a near-full bar is a real warning. High
  // utilization, by contrast, is what researchers *want* — never alarm it.
  function vramTone(pct: number | null): string {
    if (pct == null) return "bg-accent";
    if (pct >= 97) return "bg-status-crashed";
    if (pct >= 88) return "bg-status-unhealthy";
    return "bg-accent";
  }

  function tempTone(c: number | null): string {
    if (c == null) return "text-fg-muted";
    if (c >= 87) return "text-status-crashed";
    if (c >= 75) return "text-status-unhealthy";
    return "text-fg-muted";
  }

  function utilTone(pct: number | null): string {
    if (pct == null) return "text-fg-subtle";
    if (pct === 0) return "text-fg-subtle";
    return "text-fg";
  }

  const subtitle = $derived(
    !readout || !readout.available
      ? "nvidia-smi — point-in-time, not live"
      : [
          stampedAt ? `Snapshot · ${relativeTime(stampedAt)}` : null,
          readout.driver ? `Driver ${readout.driver}` : null,
          readout.cuda ? `CUDA ${readout.cuda}` : null,
        ]
          .filter(Boolean)
          .join(" · "),
  );
</script>

<div class="flex h-full min-h-0 flex-col">
  <header class="flex items-center gap-2 border-b border-border/60 px-6 py-3">
    <Icon name="cpu" size={15} class="text-fg-muted" />
    <div class="min-w-0 flex-1">
      <h2 class="text-[13px] font-semibold text-fg">GPU</h2>
      <p class="truncate text-[11px] text-fg-subtle">{subtitle}</p>
    </div>
    <button
      type="button"
      onclick={refresh}
      disabled={loading}
      class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md text-[12px] font-medium border border-border text-fg-muted hover:text-fg hover:bg-surface-2 disabled:opacity-50"
    >
      <Icon name="refresh-cw" size={12} class={loading ? "animate-spin" : ""} />
      {readout?.gpus.length ? "Refresh" : "Load"}
    </button>
  </header>

  <div class="min-h-0 flex-1 overflow-y-auto">
    {#if error === "no-gpu" && !readout?.gpus.length}
      <!-- Graceful degradation: a normal, non-alarming state for non-GPU hosts. -->
      <div class="flex h-full flex-col items-center justify-center gap-2 px-6 text-center">
        <div class="grid h-10 w-10 place-items-center rounded-full border border-border/70 bg-surface-2/40">
          <Icon name="cpu" size={18} class="text-fg-subtle" />
        </div>
        <p class="text-[12.5px] font-medium text-fg-muted">No NVIDIA GPU detected</p>
        <p class="max-w-xs text-[11.5px] text-fg-subtle leading-relaxed">
          <code class="font-mono">nvidia-smi</code> isn't available on {label}. This panel lights up
          on hosts with NVIDIA hardware.
        </p>
      </div>
    {:else if !readout?.gpus.length}
      <div class="flex h-full items-center justify-center">
        <button
          type="button"
          onclick={refresh}
          disabled={loading}
          class="inline-flex items-center gap-2 rounded-lg border border-border px-3.5 py-2 text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg disabled:opacity-50"
        >
          <Icon name={loading ? "refresh-cw" : "cpu"} size={14} class={loading ? "animate-spin" : ""} />
          {loading ? "Reading GPUs…" : "Load GPU status"}
        </button>
      </div>
    {:else}
      <div class="grid gap-3 p-4 @2xl:grid-cols-2">
        {#each readout.gpus as g (g.uuid)}
          {@const pct = memPct(g)}
          {@const watched = mlWatch.isWatched(connectionId, "gpu", String(g.index))}
          <section class="rounded-xl border border-border/70 bg-surface px-4 py-3.5">
            <!-- Title row: index badge + model, with utilization as the headline. -->
            <div class="flex items-start gap-2.5">
              <span class="mt-0.5 shrink-0 rounded-md bg-surface-2 px-1.5 py-0.5 font-mono text-[11px] font-semibold text-fg-muted">
                GPU {g.index}
              </span>
              <h3 class="min-w-0 flex-1 truncate text-[12.5px] font-medium text-fg" title={g.name}>
                {g.name}
              </h3>
              <div class="shrink-0 text-right">
                <div class="font-mono text-[18px] font-semibold leading-none tabular-nums {utilTone(g.utilization)}">
                  {g.utilization != null ? `${g.utilization}%` : "—"}
                </div>
                <div class="mt-0.5 text-[10px] uppercase tracking-wide text-fg-subtle">
                  {g.utilization === 0 ? "idle" : "util"}
                </div>
              </div>
            </div>

            <!-- VRAM: the constraint that decides whether a job fits at all. -->
            <div class="mt-3">
              <div class="flex items-baseline justify-between gap-2 text-[11px]">
                <span class="font-medium text-fg-muted">VRAM</span>
                <span class="font-mono tabular-nums text-fg-subtle">
                  {gb(g.memUsedMb)} / {gb(g.memTotalMb)}{pct != null ? ` · ${pct}%` : ""}
                </span>
              </div>
              <div class="mt-1.5 h-2 w-full overflow-hidden rounded-full bg-surface-2">
                {#if pct != null}
                  <div class="h-full rounded-full {vramTone(pct)}" style="width: {Math.min(100, Math.max(0, pct))}%"></div>
                {/if}
              </div>
            </div>

            <!-- Secondary stats. -->
            <div class="mt-3 flex flex-wrap items-center gap-x-4 gap-y-1.5 text-[11.5px]">
              <span class="inline-flex items-center gap-1.5 {tempTone(g.tempC)}" title="Die temperature">
                <Icon name="thermometer" size={13} />
                <span class="font-mono tabular-nums">{g.tempC != null ? `${g.tempC}°C` : "—"}</span>
              </span>
              <span class="inline-flex items-center gap-1.5 text-fg-muted" title="Power draw / limit">
                <Icon name="zap" size={13} />
                <span class="font-mono tabular-nums">
                  {g.powerW != null ? `${Math.round(g.powerW)}` : "—"}{g.powerLimitW != null ? ` / ${Math.round(g.powerLimitW)}` : ""} W
                </span>
              </span>
              <span class="inline-flex items-center gap-1.5 text-fg-subtle" title="Compute processes">
                <Icon name="list" size={13} />
                <span class="tabular-nums">{g.procs.length} {g.procs.length === 1 ? "process" : "processes"}</span>
              </span>
              <button
                type="button"
                onclick={() => mlWatch.toggle({ connectionId, hostLabel: label, kind: "gpu", ref: String(g.index), name: g.name })}
                class="ml-auto inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-[11px] font-medium hover:bg-surface-2 {watched ? 'text-accent' : 'text-fg-subtle hover:text-fg'}"
                title={watched ? "Watching — you'll be notified when this GPU frees up" : "Notify me when this GPU is free"}
              >
                <Icon name="bell" size={12} />
                {watched ? "Watching" : "Notify when free"}
              </button>
            </div>

            <!-- Who owns the board — the answer to "why is GPU 3 full?" -->
            {#if g.procs.length}
              <ul class="mt-3 space-y-1 border-t border-border/40 pt-2.5">
                {#each g.procs as p (p.pid)}
                  <li class="flex items-baseline gap-2 text-[11.5px]">
                    <span class="font-mono text-fg-muted tabular-nums">{p.pid}</span>
                    {#if p.user}
                      <span class="shrink-0 rounded bg-surface-2 px-1 py-px text-[10.5px] text-fg-subtle">{p.user}</span>
                    {/if}
                    <span class="min-w-0 flex-1 truncate font-mono text-fg" title={p.name}>{p.name}</span>
                    <span class="shrink-0 font-mono tabular-nums text-fg-muted">{gb(p.memMb)}</span>
                  </li>
                {/each}
              </ul>
            {:else}
              <p class="mt-3 border-t border-border/40 pt-2.5 text-[11.5px] text-fg-subtle">
                No compute processes — GPU is idle.
              </p>
            {/if}
          </section>
        {/each}
      </div>
    {/if}
  </div>
</div>
