<!--
  LocalFilePane — the local half of the file manager's dual-pane mode. Browses
  *this machine's* filesystem (via `local_list_dir`) beside the remote SFTP pane,
  so files and folders can be picked and uploaded into the remote destination
  without going through a native file dialog.

  Because these paths are renderer-named (no OS picker), the first upload from a
  location asks the user once via a host-rendered confirm (`ensureLocalAccess`)
  before the backend will read anything. Uploads run through the shared transfer
  queue; rows can also be dragged onto the remote pane to drop into an exact
  remote folder.
-->
<script lang="ts">
  import { onMount } from "svelte";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import { localListDir, localSearch, type LocalEntry } from "$lib/deploy";
  import { posixBasename } from "$lib/sftp";
  import { ensureLocalAccess } from "$lib/sftpLocalAccess";
  import { uploadWithConfirm, LOCAL_DRAG_MIME } from "$lib/sftpUploadFlow";

  let {
    connectionId,
    hostLabel,
    uploadDest,
    onUploaded,
  }: {
    connectionId: string;
    hostLabel: string;
    /** Remote directory uploads land in (the SFTP pane's cwd, or the single
        selected remote folder — the parent decides and labels it here). */
    uploadDest: string;
    onUploaded: () => void;
  } = $props();

  let cwd = $state("");
  let entries = $state<LocalEntry[]>([]);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let selected = $state<Set<string>>(new Set());

  // Live name filter over this folder's listing (mirrors the remote column).
  let query = $state("");
  const visibleEntries = $derived.by(() => {
    const q = query.trim().toLowerCase();
    if (!q) return entries;
    return entries.filter((e) => e.name.toLowerCase().includes(q));
  });

  // Deep (recursive) local search — same layers toggle as the server column.
  // ON by default: finds files anywhere under the current folder by name or
  // glob (`*.zip`) without having to drill into subfolders first.
  let deepSearch = $state(true);
  let deepResults = $state<LocalEntry[] | null>(null);
  let deepRunning = $state(false);
  let deepTruncated = $state(false);
  let deepTimer: ReturnType<typeof setTimeout> | null = null;
  // Bumped on every (re)start/reset so stale results are ignored.
  let deepToken = 0;

  function resetDeep() {
    if (deepTimer) clearTimeout(deepTimer);
    deepTimer = null;
    deepToken += 1;
    deepResults = null;
    deepRunning = false;
    deepTruncated = false;
  }

  function queueDeepSearch() {
    if (!deepSearch || !query.trim()) {
      resetDeep();
      return;
    }
    if (deepTimer) clearTimeout(deepTimer);
    deepToken += 1;
    deepRunning = true;
    deepTimer = setTimeout(() => void runDeepSearch(), 350);
  }

  async function runDeepSearch() {
    const token = ++deepToken;
    deepRunning = true;
    try {
      const res = await localSearch(cwd, query.trim());
      if (token !== deepToken) return; // superseded
      deepResults = res.entries;
      deepTruncated = res.truncated;
    } catch {
      if (token === deepToken) deepResults = [];
    } finally {
      if (token === deepToken) deepRunning = false;
    }
  }

  function toggleDeepSearch() {
    deepSearch = !deepSearch;
    queueDeepSearch();
  }

  const deepActive = $derived(deepSearch && query.trim() !== "" && deepResults !== null);
  /** Rows on display: deep results when searching subfolders, else the folder. */
  const shownEntries = $derived(deepActive ? (deepResults ?? []) : visibleEntries);

  /** Folder of a deep result, relative to the browsed folder (subtitle). */
  function relParent(e: LocalEntry): string {
    const parent = localParent(e.path);
    if (parent === cwd) return "./";
    const base = cwd.endsWith("/") ? cwd : `${cwd}/`;
    return parent.startsWith(base) ? parent.slice(base.length) : parent;
  }

  const selectedCount = $derived(selected.size);
  /** Compact destination for the Upload button ("/" stays "/"). */
  const destLabel = $derived(
    uploadDest === "/" || uploadDest === "" ? "/" : posixBasename(uploadDest),
  );

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
      query = "";
      selected = new Set();
      resetDeep();
    } catch (e) {
      error = e instanceof Error ? e.message : "Couldn't open that folder.";
    } finally {
      loading = false;
    }
  }

  /** Re-list the current folder in place — the filter, selection (pruned to
      paths that still exist), and any deep search survive, unlike navigate(). */
  async function refreshNow() {
    if (!cwd) return;
    loading = true;
    error = null;
    try {
      const list = await localListDir(cwd);
      entries = list;
      if (selected.size > 0) {
        const alive = new Set(list.map((e) => e.path));
        selected = new Set([...selected].filter((p) => alive.has(p)));
      }
      if (deepSearch && query.trim()) void runDeepSearch();
    } catch (e) {
      error = e instanceof Error ? e.message : "Couldn't refresh this folder.";
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

  /** Grant-check, then the shared pipeline: plan (folders walk recursively),
      live replace-confirm against the remote listing, enqueue. */
  async function upload(paths: string[]) {
    if (paths.length === 0) return;
    try {
      if (!(await ensureLocalAccess(paths, hostLabel))) return;
      await uploadWithConfirm(connectionId, paths, uploadDest, onUploaded);
    } catch {
      /* safeInvoke toasted the failure */
    }
  }

  function uploadOne(e: LocalEntry) {
    void upload([e.path]);
  }

  function uploadSelected() {
    // Selection is a set of absolute paths — works across both the folder
    // listing and deep-search results (the plan stats each path itself).
    const paths = [...selected];
    selected = new Set();
    void upload(paths);
  }

  // Drag source for in-app drag-and-drop: dragging a selected row carries the
  // whole selection; an unselected row drags alone. The remote pane reads the
  // payload and drops it into the hovered remote folder.
  function onDragStart(ev: DragEvent, e: LocalEntry) {
    const paths = selected.has(e.path) ? [...selected] : [e.path];
    ev.dataTransfer?.setData(LOCAL_DRAG_MIME, JSON.stringify(paths));
    if (ev.dataTransfer) ev.dataTransfer.effectAllowed = "copy";
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
    <button
      type="button"
      onclick={() => void refreshNow()}
      class="inline-flex h-6 items-center rounded px-1.5 text-[11px] text-fg-muted hover:bg-surface-2 hover:text-fg"
      title="Refresh"
      aria-label="Refresh"
    >
      <Icon name="refresh-cw" size={12} class={loading ? "animate-spin" : ""} />
    </button>
  </div>

  <!-- Search: live filter over this folder, or — with the layers toggle — a
       recursive search of every subfolder (by name or glob, e.g. *.zip). -->
  <div class="flex items-center gap-1 border-b border-border px-2 py-1.5">
    <div class="relative min-w-0 flex-1">
      <span class="absolute left-1.5 top-1/2 -translate-y-1/2 text-fg-subtle">
        <Icon name="search" size={12} />
      </span>
      <input
        bind:value={query}
        oninput={queueDeepSearch}
        placeholder={deepSearch ? "Search subfolders… (*.zip works)" : "Search local…"}
        spellcheck="false"
        class="w-full rounded border border-border bg-surface pl-7 pr-6 text-[11.5px] text-fg
               placeholder:text-fg-subtle focus:border-accent/60 focus:outline-none"
        style="height: 26px"
      />
      {#if query}
        <button
          type="button"
          onclick={() => { query = ""; resetDeep(); }}
          class="absolute right-1.5 top-1/2 -translate-y-1/2 rounded p-0.5 text-fg-subtle hover:text-fg"
          aria-label="Clear search"
        >
          <Icon name="x" size={12} />
        </button>
      {/if}
    </div>
    <button
      type="button"
      onclick={toggleDeepSearch}
      aria-pressed={deepSearch}
      class="grid h-[26px] w-7 shrink-0 place-items-center rounded {deepSearch ? 'bg-accent/15 text-accent' : 'text-fg-muted hover:bg-surface-2 hover:text-fg'}"
      title={deepSearch ? "Searching subfolders too — click for this folder only" : "Search subfolders too (deep search)"}
    >
      <Icon name="layers" size={12} />
    </button>
  </div>

  {#if selectedCount > 0}
    <div class="flex items-center gap-2 border-b border-border bg-accent/10 px-2 py-1 text-[11.5px]">
      <span class="text-fg">{selectedCount} selected</span>
      <button
        type="button"
        onclick={uploadSelected}
        class="ml-auto inline-flex h-6 min-w-0 items-center gap-1 rounded px-1.5 text-fg-muted hover:bg-surface hover:text-fg"
        title={`Upload to ${uploadDest || "/"}`}
      >
        <Icon name="arrow-up" size={12} />
        <span class="truncate">Upload → {destLabel}</span>
      </button>
    </div>
  {/if}

  <div class="min-h-0 flex-1 overflow-y-auto">
    {#if error}
      <p class="m-3 rounded-md border border-status-crashed/40 bg-status-crashed/10 p-2 text-[11.5px] text-status-crashed">{error}</p>
    {:else if loading && entries.length === 0}
      <p class="p-4 text-center text-[11.5px] text-fg-subtle">Loading…</p>
    {:else if !deepActive && entries.length === 0}
      <p class="p-4 text-center text-[11.5px] text-fg-subtle">Empty folder.</p>
    {:else}
      {#if deepActive}
        <div class="sticky top-0 z-10 flex items-center gap-2 border-b border-border/60 bg-surface px-2 py-1 text-[10.5px] text-fg-subtle">
          {#if deepRunning}<Icon name="refresh-cw" size={10} class="shrink-0 animate-spin" />{/if}
          <span class="truncate">
            {shownEntries.length} result{shownEntries.length === 1 ? "" : "s"} in subfolders
            {#if deepTruncated}· stopped at limit{/if}
          </span>
        </div>
      {/if}
      {#if shownEntries.length === 0}
        <p class="p-4 text-center text-[11.5px] text-fg-subtle">
          {deepActive && deepRunning ? "Searching…" : `No files match “${query}”.`}
        </p>
      {:else}
        <ul class="py-1">
          {#each shownEntries as e (e.path)}
            <li
              class="group flex items-center gap-2 px-2 py-1 hover:bg-surface-2/60"
              draggable="true"
              ondragstart={(ev) => onDragStart(ev, e)}
            >
              <button
                type="button"
                role="checkbox"
                aria-checked={selected.has(e.path)}
                onclick={() => toggle(e.path)}
                aria-label={`Select ${e.name}`}
                class="grid h-[15px] w-[15px] shrink-0 place-items-center rounded border {selected.has(e.path)
                  ? 'border-accent bg-accent text-on-accent'
                  : 'border-border bg-surface hover:border-fg-subtle'}"
              >
                {#if selected.has(e.path)}
                  <Icon name="check" size={11} />
                {/if}
              </button>
              {#if e.isDir}
                <Icon name="folder" size={13} class="shrink-0 text-accent" />
                <button type="button" onclick={() => navigate(e.path)} class="min-w-0 flex-1 truncate text-left text-[12px] text-fg hover:text-accent" title={e.path}>
                  {e.name}{#if deepActive}<span class="ml-1.5 font-mono text-[10px] text-fg-subtle">{relParent(e)}</span>{/if}
                </button>
              {:else}
                <Icon name="file-text" size={13} class="shrink-0 text-fg-subtle" />
                <span class="min-w-0 flex-1 truncate text-[12px] text-fg-muted" title={e.path}>
                  {e.name}{#if deepActive}<span class="ml-1.5 font-mono text-[10px] text-fg-subtle">{relParent(e)}</span>{/if}
                </span>
              {/if}
              <button
                type="button"
                onclick={() => uploadOne(e)}
                class="shrink-0 rounded p-1 text-fg-subtle opacity-0 hover:bg-surface hover:text-fg group-hover:opacity-100"
                title={`Upload ${e.isDir ? "folder" : "file"} to ${uploadDest || "/"}`}
              >
                <Icon name="arrow-up" size={12} />
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    {/if}
  </div>
</div>
