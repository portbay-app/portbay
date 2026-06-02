<!--
  SshConfigImport — preview-and-pick surface for importing hosts from
  ~/.ssh/config. Fetches parsed candidates, lets the user choose which to add,
  and saves the picks through the normal connection-save path (so each gets a
  fresh, collision-free id — import never overwrites an existing host).
  Wildcard `Host *` defaults blocks are shown but not selectable.
-->
<script lang="ts">
  import { onMount } from "svelte";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import { sshConnections } from "$lib/stores/sshConnections.svelte";
  import type {
    SaveSshConnectionInput,
    SshConfigCandidate,
  } from "$lib/types/sshConnections";

  interface Props {
    onclose: () => void;
    /** Called after a successful import with the first new host's id (or null). */
    ondone: (firstId: string | null) => void;
  }
  let { onclose, ondone }: Props = $props();

  let candidates = $state<SshConfigCandidate[]>([]);
  let selected = $state<Record<string, boolean>>({});
  let loaded = $state(false);
  let importing = $state(false);

  onMount(async () => {
    candidates = await sshConnections.importConfig();
    // Default-select importable (non-wildcard) hosts; key by file index so
    // duplicate aliases across the file don't collide.
    const sel: Record<string, boolean> = {};
    candidates.forEach((c, i) => {
      if (!c.wildcard) sel[String(i)] = true;
    });
    selected = sel;
    loaded = true;
  });

  const importable = $derived(
    candidates
      .map((c, i) => ({ c, i }))
      .filter(({ c }) => !c.wildcard),
  );
  const chosenCount = $derived(
    importable.filter(({ i }) => selected[String(i)]).length,
  );

  function toggle(index: number) {
    const key = String(index);
    selected = { ...selected, [key]: !selected[key] };
  }

  function destination(c: SshConfigCandidate): string {
    const base = c.sshUser.trim() ? `${c.sshUser}@${c.sshHost}` : c.sshHost;
    return c.sshPort !== 22 ? `${base}:${c.sshPort}` : base;
  }

  function candidateToInput(c: SshConfigCandidate): SaveSshConnectionInput {
    return {
      id: null,
      name: c.hostAlias,
      sshHost: c.sshHost,
      sshPort: c.sshPort,
      sshUser: c.sshUser,
      // Config hosts authenticate with a key / the agent (passwords aren't
      // expressible in ~/.ssh/config), so default to key auth.
      authKind: "key",
      keyPath: c.keyPath,
      proxyJump: c.proxyJump,
      identityId: null,
      tags: [],
      color: null,
      notes: null,
      password: null,
    };
  }

  async function runImport() {
    if (importing || chosenCount === 0) return;
    importing = true;
    let firstId: string | null = null;
    for (const { c, i } of importable) {
      if (!selected[String(i)]) continue;
      const saved = await sshConnections.save(candidateToInput(c));
      if (saved && !firstId) firstId = saved.id;
    }
    importing = false;
    ondone(firstId);
  }
</script>

<section class="h-full min-w-0 overflow-y-auto">
  <header class="px-8 pt-6 pb-4 border-b border-border/60">
    <button
      type="button"
      onclick={onclose}
      class="inline-flex items-center gap-1.5 text-[12px] text-fg-muted hover:text-fg transition-colors"
    >
      <Icon name="chevron-left" size={14} />
      Back to hosts
    </button>
    <h1 class="mt-3 text-[17px] font-semibold tracking-tight text-fg">
      Import from ~/.ssh/config
    </h1>
    <p class="mt-1 text-[12.5px] text-fg-muted leading-relaxed max-w-2xl">
      Pick the hosts to save as PortBay connections. Each becomes a new host —
      importing never overwrites or edits an existing one.
    </p>
  </header>

  <div class="px-8 py-6">
    {#if !loaded}
      <p class="text-[12.5px] text-fg-subtle">Reading ~/.ssh/config…</p>
    {:else if candidates.length === 0}
      <div class="rounded-xl border border-dashed border-border px-6 py-12 text-center">
        <span class="inline-grid place-items-center w-12 h-12 rounded-xl bg-surface-2 text-fg-subtle mx-auto">
          <Icon name="file-text" size={24} />
        </span>
        <p class="mt-3 text-[13px] font-medium text-fg">Nothing to import</p>
        <p class="mt-1.5 text-[12px] text-fg-subtle leading-relaxed max-w-md mx-auto">
          No host entries were found in <code class="font-mono">~/.ssh/config</code>.
        </p>
      </div>
    {:else}
      <div class="space-y-2">
        {#each candidates as c, i (i)}
          {@const disabled = c.wildcard}
          <label
            class="flex items-start gap-3 rounded-xl border px-4 py-3 transition-colors
                   {disabled
              ? 'border-border/50 bg-surface-2/40 cursor-not-allowed'
              : 'border-border/70 bg-surface hover:border-accent/40 cursor-pointer'}"
          >
            <input
              type="checkbox"
              checked={selected[String(i)] ?? false}
              {disabled}
              onchange={() => toggle(i)}
              class="mt-0.5 rounded border-border disabled:opacity-40"
            />
            <span class="min-w-0 flex-1">
              <span class="flex items-center gap-2">
                <span class="min-w-0 truncate text-[13px] font-semibold text-fg">
                  {c.hostAlias}
                </span>
                {#if c.wildcard}
                  <span class="shrink-0 rounded bg-surface-2 px-1.5 py-0.5 text-[10px] text-fg-muted">
                    Wildcard — skipped
                  </span>
                {:else if c.alreadyExists}
                  <span class="shrink-0 rounded bg-status-unhealthy/15 px-1.5 py-0.5 text-[10px] text-status-unhealthy">
                    Already saved — adds a copy
                  </span>
                {/if}
              </span>
              <span class="mt-0.5 block truncate font-mono text-[11px] text-fg-subtle">
                {destination(c)}
              </span>
              <span class="mt-1 flex flex-wrap items-center gap-1.5">
                {#if c.keyPath}
                  <span class="inline-flex items-center gap-1 rounded bg-surface-2 px-1.5 py-0.5 text-[10px] text-fg-muted">
                    <Icon name="key" size={10} />
                    <span class="font-mono">{c.keyPath}</span>
                  </span>
                {/if}
                {#if c.proxyJump}
                  <span class="inline-flex items-center gap-1 rounded bg-surface-2 px-1.5 py-0.5 text-[10px] text-fg-muted">
                    <Icon name="share" size={10} />
                    jump {c.proxyJump}
                  </span>
                {/if}
              </span>
            </span>
          </label>
        {/each}
      </div>

      <div class="mt-5 flex items-center gap-2">
        <button
          type="button"
          onclick={runImport}
          disabled={importing || chosenCount === 0}
          class="inline-flex items-center justify-center gap-1.5 h-9 px-4 rounded-md
                 text-[12px] font-medium bg-accent text-on-accent hover:brightness-110
                 disabled:opacity-50"
        >
          <Icon name={importing ? "refresh-cw" : "plus"} size={12}
            class={importing ? "animate-spin" : ""} />
          {importing
            ? "Importing…"
            : `Import ${chosenCount} host${chosenCount === 1 ? "" : "s"}`}
        </button>
        <button
          type="button"
          onclick={onclose}
          class="inline-flex items-center justify-center gap-1.5 h-9 px-4 rounded-md
                 text-[12px] font-medium border border-border text-fg-muted hover:text-fg hover:bg-surface-2"
        >
          Cancel
        </button>
      </div>
    {/if}
  </div>
</section>
