<!--
  IdeWelcome — the editor area's pinned "Welcome" tab: the host snapshot meters,
  host facts, and quick actions. Lifted out of SshWorkspace so the overview is
  one editor tab among the open files. Snapshot data + the refresh action are
  owned by SshWorkspace (which warms the connection on mount and drives the
  status bar's `connected` flag); this component is presentational.
-->
<script lang="ts">
  import HostMark from "$lib/components/atoms/HostMark.svelte";
  import Icon from "$lib/components/atoms/Icon.svelte";
  import { ideLayout } from "$lib/stores/ideLayout.svelte";
  import { providerLabel } from "$lib/ssh/providers";
  import {
    absoluteTime,
    authSummary,
    dateLabel,
    relativeTime,
    stageMeta,
  } from "$lib/ssh/hostFormat";
  import type { HostSnapshot } from "$lib/ssh/hostSnapshot";
  import { errorBus } from "$lib/stores/errors.svelte";
  import type { ProbeResult, SshConnectionView } from "$lib/types/sshConnections";

  interface Props {
    host: SshConnectionView;
    dest: string;
    snapshot: HostSnapshot | null;
    snapshotAt: number | null;
    loadingSnapshot: boolean;
    connected: boolean;
    probe: ProbeResult | null;
    onRefresh: () => void;
    onAddTunnel: () => void;
  }
  let { host, dest, snapshot, snapshotAt, loadingSnapshot, connected, probe, onRefresh, onAddTunnel }: Props =
    $props();

  let copied = $state<string | null>(null);

  const stage = $derived(stageMeta(host.stage));
  const auth = $derived(authSummary(host));
  const prov = $derived(providerLabel(host.provider));

  const memPercent = $derived(
    snapshot?.memTotalMb && snapshot.memUsedMb != null
      ? Math.round((snapshot.memUsedMb / snapshot.memTotalMb) * 100)
      : null,
  );

  function gb(mb: number | null): string {
    if (mb == null) return "—";
    return mb >= 1024 ? `${(mb / 1024).toFixed(1)} GB` : `${mb} MB`;
  }

  async function copy(key: string, value: string, what: string) {
    try {
      await navigator.clipboard.writeText(value);
      copied = key;
      setTimeout(() => {
        if (copied === key) copied = null;
      }, 1500);
      errorBus.push({
        code: "COPIED",
        category: "infrastructure",
        whatHappened: what,
        whyItMatters: "Copied to your clipboard.",
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });
    } catch {
      /* no clipboard permission */
    }
  }
</script>

<div class="@container w-full px-8 py-7">
  <div class="flex items-center gap-2.5">
    <HostMark environment={host.environment} size={32} class="shrink-0" />
    <div class="min-w-0">
      <h2 class="truncate text-[16px] font-semibold tracking-tight text-fg">{host.name}</h2>
      <p class="truncate font-mono text-[12px] text-fg-subtle">
        {host.sshUser ? `${host.sshUser}@` : ""}{dest}:{host.sshPort}
      </p>
    </div>
    {#if connected}
      <span class="ml-auto inline-flex items-center gap-1.5 text-[12px] text-status-running">
        <span class="h-1.5 w-1.5 rounded-full bg-status-running"></span> Connected
      </span>
    {/if}
  </div>

  <!-- Snapshot + host facts: side-by-side once the pane is wide enough, so the
       overview fills the available width instead of a single narrow column. -->
  <div class="mt-5 grid items-start gap-4 @4xl:grid-cols-2">
  <!-- Snapshot hero -->
  <section class="rounded-xl border border-border/70 bg-surface px-5 py-4">
    <div class="flex items-center gap-2">
      <h3 class="text-[13px] font-semibold text-fg">Host snapshot</h3>
      {#if snapshotAt}
        <span class="text-[11px] text-fg-subtle">· as of {relativeTime(snapshotAt)}</span>
      {/if}
      <button
        type="button"
        onclick={onRefresh}
        disabled={loadingSnapshot}
        class="ml-auto inline-flex items-center gap-1.5 h-7 px-2.5 rounded-md text-[11.5px] font-medium border border-border text-fg-muted hover:text-fg hover:bg-surface-2 disabled:opacity-50"
      >
        <Icon name="refresh-cw" size={12} class={loadingSnapshot ? "animate-spin" : ""} />
        {snapshot ? "Refresh" : "Run"}
      </button>
    </div>

    {#if !snapshot}
      <p class="mt-2 text-[12px] text-fg-subtle leading-relaxed">
        Point-in-time only — runs <code class="font-mono text-fg-muted">uptime</code>,
        <code class="font-mono text-fg-muted">free</code> and
        <code class="font-mono text-fg-muted">df</code> once over SSH.
      </p>
    {:else}
      <div class="mt-3 grid grid-cols-2 gap-3">
        {@render meter("Memory", memPercent, `${gb(snapshot.memUsedMb)} / ${gb(snapshot.memTotalMb)}`)}
        {@render meter("Disk (/)", snapshot.diskPercent, snapshot.diskUsed && snapshot.diskTotal ? `${snapshot.diskUsed} / ${snapshot.diskTotal}` : "—")}
      </div>
      <dl class="mt-4 grid grid-cols-3 gap-x-6 gap-y-2 text-[12px]">
        {@render fact("Logged in as", snapshot.user ?? "—")}
        {@render fact("OS", snapshot.os ?? host.detectedOs ?? "—")}
        {@render fact("Load (1m)", snapshot.load1 ?? "—")}
      </dl>
    {/if}
  </section>

  <!-- Host facts -->
  <section class="rounded-xl border border-border/70 bg-surface px-5 py-4">
    <h3 class="text-[13px] font-semibold text-fg">Host overview</h3>
    <dl class="mt-3 grid grid-cols-2 gap-x-8 gap-y-2.5 text-[12.5px]">
      {@render row("Host", host.sshHost, true, "host")}
      {@render row("Port", String(host.sshPort), false)}
      {@render row("Username", host.sshUser || "—", false, host.sshUser ? "user" : undefined)}
      {@render row("Authentication", auth.detail ? `${auth.label} (${auth.detail})` : auth.label, false)}
      {@render row("Fingerprint", probe?.fingerprint ?? "Not probed yet", true, probe?.fingerprint ? "fp" : undefined)}
      {@render row("Provider / Region", prov ? (host.region ? `${prov} / ${host.region}` : prov) : "—", false)}
      {@render row("OS", host.detectedOs ?? "Not detected", false)}
      {@render row("Created", dateLabel(host.createdAt), false)}
      {@render row("Last used", relativeTime(host.lastUsed) + (host.lastUsed ? ` (${absoluteTime(host.lastUsed)})` : ""), false)}
    </dl>
    {#if stage || (host?.tags ?? []).length}
      <div class="mt-3 flex flex-wrap items-center gap-1.5">
        {#if stage}
          <span class="inline-flex items-center rounded-md px-1.5 py-0.5 text-[10.5px] font-medium {stage.chipClass}">{stage.label}</span>
        {/if}
        {#each host?.tags ?? [] as tag (tag)}
          <span class="rounded bg-surface-2 px-1.5 py-0.5 text-[10.5px] text-fg-muted">{tag}</span>
        {/each}
      </div>
    {/if}
  </section>
  </div>

  <!-- Quick actions -->
  <div class="mt-4 flex flex-wrap gap-2">
    <button type="button" onclick={() => ideLayout.selectView("explorer")} class="inline-flex items-center gap-2 rounded-lg border border-border px-3 py-2 text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg">
      <Icon name="folder" size={14} /> Browse files
    </button>
    <button type="button" onclick={() => ideLayout.showPanelTab("terminal")} class="inline-flex items-center gap-2 rounded-lg border border-border px-3 py-2 text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg">
      <Icon name="terminal" size={14} /> Open terminal
    </button>
    <button type="button" onclick={() => ideLayout.selectView("deploy")} class="inline-flex items-center gap-2 rounded-lg border border-border px-3 py-2 text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg">
      <Icon name="rocket" size={14} /> Deploy
    </button>
    <button type="button" onclick={onAddTunnel} class="inline-flex items-center gap-2 rounded-lg border border-border px-3 py-2 text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg">
      <Icon name="plus" size={14} /> Add port forward
    </button>
  </div>
</div>

<!-- Snapshot fact (label over value). -->
{#snippet fact(label: string, value: string)}
  <div>
    <dt class="text-fg-subtle">{label}</dt>
    <dd class="mt-0.5 truncate font-mono text-fg">{value}</dd>
  </div>
{/snippet}

<!-- Snapshot meter: a labelled bar, or a dash when the percentage is unknown. -->
{#snippet meter(label: string, pct: number | null, detail: string)}
  <div class="rounded-lg border border-border/60 bg-surface-2/40 px-3 py-2.5">
    <div class="flex items-baseline justify-between gap-2">
      <span class="text-[11.5px] font-medium text-fg">{label}</span>
      <span class="text-[11px] tabular-nums text-fg-muted">{pct != null ? `${pct}%` : "—"}</span>
    </div>
    <div class="mt-1.5 h-1.5 w-full overflow-hidden rounded-full bg-surface-2">
      {#if pct != null}
        <div class="h-full rounded-full bg-accent" style="width: {Math.min(100, Math.max(0, pct))}%"></div>
      {/if}
    </div>
    <p class="mt-1.5 truncate font-mono text-[10.5px] text-fg-subtle">{detail}</p>
  </div>
{/snippet}

<!-- Fact row (label beside value) with an optional copy button. -->
{#snippet row(label: string, value: string, mono: boolean, copyKey?: string)}
  <div class="flex items-baseline gap-3">
    <dt class="w-28 shrink-0 text-fg-subtle">{label}</dt>
    <dd class="flex min-w-0 flex-1 items-center gap-1.5 text-fg">
      <span class="min-w-0 truncate {mono ? 'font-mono' : ''}">{value}</span>
      {#if copyKey}
        <button type="button" onclick={() => copy(copyKey, value, `${label} copied.`)} class="shrink-0 rounded p-0.5 text-fg-subtle hover:text-fg" aria-label={`Copy ${label}`}>
          <Icon name={copied === copyKey ? "check" : "copy"} size={12} />
        </button>
      {/if}
    </dd>
  </div>
{/snippet}
