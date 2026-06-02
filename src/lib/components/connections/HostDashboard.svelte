<!--
  HostDashboard — the SSH page's host-first landing view. Renders saved
  connections as a searchable card grid (color dot, name, user@host, OS badge,
  tag chips, tunnel count, last-used). Selecting a host drills into it; "+" adds
  one. Shown once ≥1 connection exists; below that the page keeps its empty state.
-->
<script lang="ts">
  import HostMark from "$lib/components/atoms/HostMark.svelte";
  import Icon from "$lib/components/atoms/Icon.svelte";
  import type { SshConnectionView } from "$lib/types/sshConnections";

  interface Props {
    connections: SshConnectionView[];
    onselect: (id: string) => void;
    onadd: () => void;
    onimport: () => void;
    onmanageIdentities: () => void;
  }
  let { connections, onselect, onadd, onimport, onmanageIdentities }: Props = $props();

  let query = $state("");

  const filtered = $derived.by(() => {
    const q = query.trim().toLowerCase();
    if (!q) return connections;
    return connections.filter((c) => {
      const haystack = [
        c.name,
        c.sshHost,
        c.sshUser,
        c.detectedOs ?? "",
        ...(c.tags ?? []),
      ]
        .join(" ")
        .toLowerCase();
      return haystack.includes(q);
    });
  });

  function destination(c: SshConnectionView): string {
    return c.sshUser.trim() ? `${c.sshUser}@${c.sshHost}` : c.sshHost;
  }

  function lastUsedLabel(secs: number | null): string {
    if (!secs) return "Never used";
    const diff = Math.max(0, Math.floor(Date.now() / 1000) - secs);
    if (diff < 60) return "Used just now";
    if (diff < 3600) return `Used ${Math.floor(diff / 60)}m ago`;
    if (diff < 86_400) return `Used ${Math.floor(diff / 3600)}h ago`;
    return `Used ${Math.floor(diff / 86_400)}d ago`;
  }
</script>

<section class="h-full min-w-0 overflow-y-auto">
  <header class="px-8 pt-8 pb-5 border-b border-border/60">
    <div class="flex items-center gap-2.5">
      <Icon name="server" size={18} class="text-accent" />
      <h1 class="text-[17px] font-semibold tracking-tight text-fg">SSH Hosts</h1>
      <span
        class="ml-1 inline-flex items-center h-5 px-2 rounded-full text-[11px]
               font-medium tabular-nums bg-accent/10 text-accent"
      >
        {connections.length}
      </span>
      <button
        type="button"
        onclick={onmanageIdentities}
        class="ml-auto inline-flex items-center gap-1.5 h-8 px-3 rounded-md text-[12px]
               font-medium border border-border text-fg-muted hover:text-fg hover:bg-surface-2"
        title="Manage reusable SSH identities"
      >
        <Icon name="key" size={12} />
        Identities
      </button>
      <button
        type="button"
        onclick={onimport}
        class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md text-[12px]
               font-medium border border-border text-fg-muted hover:text-fg hover:bg-surface-2"
        title="Import hosts from ~/.ssh/config"
      >
        <Icon name="file-text" size={12} />
        Import config
      </button>
      <button
        type="button"
        onclick={onadd}
        class="inline-flex items-center gap-1.5 h-8 px-3.5 rounded-md text-[12px]
               font-medium text-on-accent bg-accent shadow-sm hover:brightness-110
               active:brightness-95 transition"
      >
        <Icon name="plus" size={13} />
        Add host
      </button>
    </div>
    <div class="mt-3 relative max-w-sm">
      <span class="absolute left-2.5 top-1/2 -translate-y-1/2 text-fg-subtle">
        <Icon name="search" size={13} />
      </span>
      <input
        bind:value={query}
        placeholder="Search hosts, tags, OS…"
        class="w-full h-8 rounded-md border border-border bg-surface pl-8 pr-2 text-[12px] text-fg"
      />
    </div>
  </header>

  <div class="px-8 py-6">
    {#if filtered.length === 0}
      <p class="text-[12.5px] text-fg-subtle">No hosts match “{query}”.</p>
    {:else}
      <div class="grid grid-cols-1 gap-2.5 @container sm:grid-cols-2 xl:grid-cols-3">
        {#each filtered as conn (conn.id)}
          <button
            type="button"
            onclick={() => onselect(conn.id)}
            class="group text-left rounded-2xl border border-border/70 bg-surface px-4 py-3.5
                   hover:border-accent/40 hover:bg-accent/[0.03] transition-colors"
          >
            <div class="flex items-center gap-2.5">
              <HostMark environment={conn.environment} size={22} class="shrink-0" />
              <span class="min-w-0 flex-1 truncate text-[13px] font-semibold text-fg">
                {conn.name}
              </span>
              {#if conn.color}
                <span
                  class="shrink-0 w-2.5 h-2.5 rounded-full"
                  style:background-color={conn.color}
                  title="Host colour"
                ></span>
              {/if}
              <Icon
                name="chevron-right"
                size={15}
                class="shrink-0 text-fg-subtle group-hover:text-fg transition-colors"
              />
            </div>
            <p class="mt-1 truncate font-mono text-[11px] text-fg-subtle">
              {destination(conn)}{conn.sshPort !== 22 ? `:${conn.sshPort}` : ""}
            </p>
            <div class="mt-2.5 flex flex-wrap items-center gap-1.5">
              {#if conn.detectedOs}
                <span class="inline-flex items-center gap-1 rounded bg-surface-2 px-1.5 py-0.5 text-[10px] text-fg-muted">
                  <Icon name="server" size={10} />
                  {conn.detectedOs}
                </span>
              {/if}
              {#if conn.tunnelCount > 0}
                <span class="inline-flex items-center gap-1 rounded bg-surface-2 px-1.5 py-0.5 text-[10px] text-fg-muted">
                  <Icon name="terminal" size={10} />
                  {conn.tunnelCount} tunnel{conn.tunnelCount === 1 ? "" : "s"}
                </span>
              {/if}
              {#each conn.tags ?? [] as tag (tag)}
                <span class="rounded bg-accent/10 px-1.5 py-0.5 text-[10px] text-accent">{tag}</span>
              {/each}
              <span class="ml-auto text-[10px] text-fg-subtle">{lastUsedLabel(conn.lastUsed)}</span>
            </div>
          </button>
        {/each}
      </div>
    {/if}
  </div>
</section>
