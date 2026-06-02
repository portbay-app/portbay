<!--
  LocalFilePane — the local half of the file manager's dual-pane mode. Browses
  *this machine's* filesystem (via `local_list_dir`) beside the remote SFTP pane,
  so files can be picked and uploaded into the remote working directory without
  going through a native dialog. Uploads run through the shared transfer queue.
-->
<script lang="ts">
  import { onMount } from "svelte";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import { localListDir, type LocalEntry } from "$lib/deploy";
  import { posixJoin } from "$lib/sftp";
  import { sftpTransfers } from "$lib/stores/sftpTransfers.svelte";

  let {
    connectionId,
    remoteCwd,
    onUploaded,
  }: { connectionId: string; remoteCwd: string; onUploaded: () => void } = $props();

  let cwd = $state("");
  let entries = $state<LocalEntry[]>([]);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let selected = $state<Set<string>>(new Set());

  const files = $derived(entries.filter((e) => !e.isDir));
  const selectedCount = $derived(selected.size);

  /** Parent of a local path (handles `/` and `\\`); stops at the root. */
  function localParent(p: string): string {
    const norm = p.replace(/[\\/]+$/, "");
    const idx = Math.max(norm.lastIndexOf("/"), norm.lastIndexOf("\\"));
    if (idx < 0) return p;
    return idx === 0 ? "/" : norm.slice(0, idx);
  }

  async function navigate(path: string) {
    loading = true;
    error = null;
    try {
      entries = await localListDir(path);
      cwd = path;
      selected = new Set();
    } catch (e) {
      error = e instanceof Error ? e.message : "Couldn't open that folder.";
    } finally {
      loading = false;
    }
  }

  function toggle(path: string) {
    const next = new Set(selected);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    selected = next;
  }

  function uploadOne(e: LocalEntry) {
    sftpTransfers.enqueueUpload(connectionId, e.path, posixJoin(remoteCwd, e.name), e.name, onUploaded);
  }

  function uploadSelected() {
    for (const e of files) {
      if (selected.has(e.path)) uploadOne(e);
    }
    selected = new Set();
  }

  onMount(() => {
    void (async () => {
      try {
        const { homeDir } = await import("@tauri-apps/api/path");
        await navigate(await homeDir());
      } catch {
        await navigate("/");
      }
    })();
  });
</script>

<div class="flex h-full min-h-0 flex-col bg-surface/40">
  <!-- Mini toolbar -->
  <div class="flex items-center gap-1 border-b border-border px-2 py-1.5">
    <Icon name="home" size={13} class="shrink-0 text-fg-subtle" />
    <button
      type="button"
      onclick={() => navigate(localParent(cwd))}
      class="inline-flex h-6 items-center rounded px-1.5 text-[11px] text-fg-muted hover:bg-surface-2 hover:text-fg"
      title="Up one folder"
    >
      <Icon name="arrow-up" size={12} />
    </button>
    <span class="min-w-0 flex-1 truncate font-mono text-[11px] text-fg-subtle" title={cwd}>{cwd || "…"}</span>
  </div>

  {#if selectedCount > 0}
    <div class="flex items-center gap-2 border-b border-border bg-accent/10 px-2 py-1 text-[11.5px]">
      <span class="text-fg">{selectedCount} selected</span>
      <button
        type="button"
        onclick={uploadSelected}
        class="ml-auto inline-flex h-6 items-center gap-1 rounded px-1.5 text-fg-muted hover:bg-surface hover:text-fg"
      >
        <Icon name="arrow-up" size={12} /> Upload →
      </button>
    </div>
  {/if}

  <div class="min-h-0 flex-1 overflow-y-auto">
    {#if error}
      <p class="m-3 rounded-md border border-status-crashed/40 bg-status-crashed/10 p-2 text-[11.5px] text-status-crashed">{error}</p>
    {:else if loading && entries.length === 0}
      <p class="p-4 text-center text-[11.5px] text-fg-subtle">Loading…</p>
    {:else if entries.length === 0}
      <p class="p-4 text-center text-[11.5px] text-fg-subtle">Empty folder.</p>
    {:else}
      <ul class="py-1">
        {#each entries as e (e.path)}
          <li class="group flex items-center gap-2 px-2 py-1 hover:bg-surface-2/60">
            {#if e.isDir}
              <Icon name="folder" size={13} class="shrink-0 text-accent" />
              <button type="button" onclick={() => navigate(e.path)} class="min-w-0 flex-1 truncate text-left text-[12px] text-fg hover:text-accent">
                {e.name}
              </button>
            {:else}
              <input
                type="checkbox"
                checked={selected.has(e.path)}
                onchange={() => toggle(e.path)}
                class="shrink-0 rounded border-border accent-accent"
                aria-label={`Select ${e.name}`}
              />
              <Icon name="file-text" size={13} class="shrink-0 text-fg-subtle" />
              <span class="min-w-0 flex-1 truncate text-[12px] text-fg-muted" title={e.name}>{e.name}</span>
              <button
                type="button"
                onclick={() => uploadOne(e)}
                class="shrink-0 rounded p-1 text-fg-subtle opacity-0 hover:bg-surface hover:text-fg group-hover:opacity-100"
                title="Upload to remote"
              >
                <Icon name="arrow-up" size={12} />
              </button>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </div>
</div>
