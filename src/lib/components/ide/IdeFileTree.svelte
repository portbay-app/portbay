<!--
  IdeFileTree — the Explorer's remote file tree (VS Code Remote-SSH model). Roots
  at the connection's home dir, lazy-loads each directory's children on expand
  via `sftp_list_dir`, and opens files into the editor area on click. A
  right-click context menu reuses the existing SFTP commands (new file/folder,
  rename, delete, chmod, download, upload-into); files dragged from the OS upload
  into the hovered directory.

  The cached SFTP session is opened once on mount (prompting a single time for a
  credential if needed); every later op reuses it without re-authenticating.
-->
<script lang="ts">
  import { onMount } from "svelte";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import type { IconName } from "$lib/components/atoms/Icon.svelte";
  import { browser } from "$app/environment";
  import { invokeQuiet } from "$lib/ipc";
  import { connectWithPrompt } from "$lib/ssh/connectWithPrompt";
  import {
    sftpListDir,
    sftpMkdir,
    sftpRename,
    sftpRemoveFile,
    sftpRemoveDir,
    sftpChmod,
    sftpWriteText,
    sftpUpload,
    sftpDownload,
    posixJoin,
    posixParent,
  } from "$lib/sftp";
  import type { SftpEntry } from "$lib/types/sshTunnels";

  interface Props {
    connectionId: string;
    label: string;
    onOpenFile: (path: string) => void;
    /** Path of the currently-active editor file, for row highlighting. */
    activePath?: string | null;
  }
  let { connectionId, label, onOpenFile, activePath = null }: Props = $props();

  let root = $state<string>("");
  let connecting = $state(true);
  let connectError = $state<string | null>(null);

  // Per-path caches. Plain records reassigned on write so runes track them.
  let children = $state<Record<string, SftpEntry[]>>({});
  let expanded = $state<Record<string, boolean>>({});
  let loadingDir = $state<Record<string, boolean>>({});

  // Context menu + text-prompt modal (mkdir / new file / rename / chmod).
  let menu = $state<{ x: number; y: number; entry: SftpEntry | null } | null>(null);
  type Prompt = {
    title: string;
    value: string;
    hint?: string;
    confirmLabel: string;
    onConfirm: (value: string) => Promise<void> | void;
  };
  let prompt = $state<Prompt | null>(null);
  let confirmDelete = $state<SftpEntry | null>(null);
  let dragOverPath = $state<string | null>(null);

  // Live filter over the loaded tree. A node shows when its name matches or any
  // already-loaded descendant matches; matching directories force-expand so the
  // hits are visible. (Unexpanded folders aren't loaded yet, so deep search is
  // bounded to what's been opened — a remote recursive find is a later add.)
  let filter = $state("");
  const fq = $derived(filter.trim().toLowerCase());
  function hasMatch(e: SftpEntry): boolean {
    if (!fq) return true;
    if (e.name.toLowerCase().includes(fq)) return true;
    if (e.isDir) {
      const kids = children[e.path];
      if (kids) return kids.some(hasMatch);
    }
    return false;
  }

  // The directory a context action / drop applies to: the entry if it's a dir,
  // else its parent; falls back to root.
  function dirOf(entry: SftpEntry | null): string {
    if (!entry) return root;
    return entry.isDir ? entry.path : posixParent(entry.path);
  }

  async function loadDir(path: string) {
    loadingDir = { ...loadingDir, [path]: true };
    try {
      const entries = await sftpListDir(connectionId, path);
      children = { ...children, [path]: entries };
    } catch {
      /* sftp wrapper toasted */
    } finally {
      loadingDir = { ...loadingDir, [path]: false };
    }
  }

  async function toggleDir(entry: SftpEntry) {
    const open = !expanded[entry.path];
    expanded = { ...expanded, [entry.path]: open };
    if (open && children[entry.path] === undefined) {
      await loadDir(entry.path);
    }
  }

  function clickEntry(entry: SftpEntry) {
    if (entry.isDir) void toggleDir(entry);
    else onOpenFile(entry.path);
  }

  /** Refresh a directory's listing (and the root when none given). */
  async function refresh(path = root) {
    if (children[path] !== undefined || path === root) await loadDir(path);
  }

  function openMenu(e: MouseEvent, entry: SftpEntry | null) {
    e.preventDefault();
    e.stopPropagation();
    menu = { x: e.clientX, y: e.clientY, entry };
  }

  function newFile(entry: SftpEntry | null) {
    const dir = dirOf(entry);
    prompt = {
      title: "New file",
      value: "",
      hint: `Created in ${dir}`,
      confirmLabel: "Create",
      onConfirm: async (name) => {
        const trimmed = name.trim();
        if (!trimmed) return;
        const target = posixJoin(dir, trimmed);
        await sftpWriteText(connectionId, target, "");
        await refreshAndExpand(dir);
        onOpenFile(target);
      },
    };
  }

  function newFolder(entry: SftpEntry | null) {
    const dir = dirOf(entry);
    prompt = {
      title: "New folder",
      value: "",
      hint: `Created in ${dir}`,
      confirmLabel: "Create",
      onConfirm: async (name) => {
        const trimmed = name.trim();
        if (!trimmed) return;
        await sftpMkdir(connectionId, posixJoin(dir, trimmed));
        await refreshAndExpand(dir);
      },
    };
  }

  function renameEntry(entry: SftpEntry) {
    prompt = {
      title: `Rename "${entry.name}"`,
      value: entry.name,
      confirmLabel: "Rename",
      onConfirm: async (name) => {
        const trimmed = name.trim();
        if (!trimmed || trimmed === entry.name) return;
        await sftpRename(connectionId, entry.path, posixJoin(posixParent(entry.path), trimmed));
        await refresh(posixParent(entry.path));
      },
    };
  }

  function chmodEntry(entry: SftpEntry) {
    const current = entry.permissions !== null ? (entry.permissions & 0o777).toString(8) : "644";
    prompt = {
      title: `Permissions for "${entry.name}"`,
      value: current,
      hint: "Octal mode, e.g. 644 or 755.",
      confirmLabel: "Apply",
      onConfirm: async (text) => {
        const mode = parseInt(text.trim(), 8);
        if (Number.isNaN(mode)) return;
        await sftpChmod(connectionId, entry.path, mode);
        await refresh(posixParent(entry.path));
      },
    };
  }

  async function submitPrompt() {
    if (!prompt) return;
    const p = prompt;
    prompt = null;
    try {
      await p.onConfirm(p.value);
    } catch {
      /* toasted */
    }
  }

  async function doDelete() {
    const e = confirmDelete;
    confirmDelete = null;
    if (!e) return;
    try {
      if (e.isDir) await sftpRemoveDir(connectionId, e.path);
      else await sftpRemoveFile(connectionId, e.path);
      await refresh(posixParent(e.path));
    } catch {
      /* toasted */
    }
  }

  async function download(entry: SftpEntry) {
    const { save } = await import("@tauri-apps/plugin-dialog");
    const dest = await save({ defaultPath: entry.name });
    if (typeof dest !== "string") return;
    try {
      await sftpDownload(connectionId, entry.path, dest);
    } catch {
      /* toasted */
    }
  }

  async function uploadInto(entry: SftpEntry | null) {
    const dir = dirOf(entry);
    const { open } = await import("@tauri-apps/plugin-dialog");
    const picked = await open({ multiple: true, directory: false });
    const paths = Array.isArray(picked) ? picked : typeof picked === "string" ? [picked] : [];
    if (paths.length === 0) return;
    await uploadPaths(paths, dir);
  }

  async function uploadPaths(localPaths: string[], dir: string) {
    let ok = false;
    for (const local of localPaths) {
      try {
        await sftpUpload(connectionId, local, posixJoin(dir, localBasename(local)));
        ok = true;
      } catch {
        /* toasted */
      }
    }
    if (ok) await refreshAndExpand(dir);
  }

  function localBasename(p: string): string {
    const seg = p.split(/[\\/]/).filter(Boolean);
    return seg.length ? seg[seg.length - 1] : p;
  }

  /** Reload a dir's listing and ensure it's expanded so new items show. */
  async function refreshAndExpand(dir: string) {
    if (dir !== root) expanded = { ...expanded, [dir]: true };
    await loadDir(dir);
  }

  function iconFor(e: SftpEntry): IconName {
    if (e.isDir) return expanded[e.path] ? "folder-open" : "folder";
    return /\.(rs|ts|js|tsx|jsx|py|rb|php|go|java|c|h|cpp|json|sh|svelte|vue|sql)$/i.test(e.name)
      ? "file-code"
      : "file-text";
  }

  onMount(() => {
    void (async () => {
      try {
        const home = await connectWithPrompt(connectionId, label, (cred) =>
          invokeQuiet<string>("sftp_connect", {
            connectionId,
            password: cred?.kind === "password" ? cred.secret : undefined,
            passphrase: cred?.kind === "passphrase" ? cred.secret : undefined,
          }),
        );
        root = home || "/";
        await loadDir(root);
      } catch {
        connectError = "Couldn't connect to this host.";
      } finally {
        connecting = false;
      }
    })();
  });

  // OS drag-and-drop upload, scoped to the tree's bounding box so it doesn't
  // collide with the app's project-drop handler elsewhere.
  let treeEl = $state<HTMLDivElement | null>(null);
  $effect(() => {
    if (!browser) return;
    let unlisten: (() => void) | null = null;
    void (async () => {
      const { getCurrentWebview } = await import("@tauri-apps/api/webview");
      unlisten = await getCurrentWebview().onDragDropEvent((event) => {
        const t = event.payload.type;
        if (t === "drop") {
          dragOverPath = null;
          const pos = (event.payload as { position?: { x: number; y: number } }).position;
          if (!withinTree(pos)) return;
          const paths = (event.payload as { paths?: string[] }).paths ?? [];
          if (paths.length) void uploadPaths(paths, root);
        } else if (t === "leave") {
          dragOverPath = null;
        } else if ((t === "enter" || t === "over") && root) {
          const pos = (event.payload as { position?: { x: number; y: number } }).position;
          dragOverPath = withinTree(pos) ? root : null;
        }
      });
    })();
    return () => unlisten?.();
  });

  function withinTree(pos?: { x: number; y: number }): boolean {
    if (!pos || !treeEl) return false;
    const r = treeEl.getBoundingClientRect();
    return pos.x >= r.left && pos.x <= r.right && pos.y >= r.top && pos.y <= r.bottom;
  }
</script>

<svelte:window
  onclick={() => (menu = null)}
  onkeydown={(e) => {
    if (e.key === "Escape") {
      if (prompt) prompt = null;
      else if (confirmDelete) confirmDelete = null;
      else menu = null;
    }
  }}
/>

<div class="flex h-full min-h-0 flex-col">
  <!-- Toolbar -->
  <div class="flex items-center gap-1 border-b border-border/50 px-2 py-1.5">
    <span class="min-w-0 flex-1 truncate font-mono text-[11px] text-fg-subtle" title={root}>
      {root || "…"}
    </span>
    <button
      type="button"
      onclick={() => newFile(null)}
      disabled={!root}
      class="grid h-6 w-6 place-items-center rounded text-fg-muted hover:bg-surface-2 hover:text-fg disabled:opacity-40"
      title="New file in root"
    >
      <Icon name="file-plus" size={13} />
    </button>
    <button
      type="button"
      onclick={() => newFolder(null)}
      disabled={!root}
      class="grid h-6 w-6 place-items-center rounded text-fg-muted hover:bg-surface-2 hover:text-fg disabled:opacity-40"
      title="New folder in root"
    >
      <Icon name="folder-plus" size={13} />
    </button>
    <button
      type="button"
      onclick={() => refresh(root)}
      disabled={!root}
      class="grid h-6 w-6 place-items-center rounded text-fg-muted hover:bg-surface-2 hover:text-fg disabled:opacity-40"
      title="Refresh"
    >
      <Icon name="refresh-cw" size={13} class={loadingDir[root] ? "animate-spin" : ""} />
    </button>
  </div>

  <!-- Filter: live match over the loaded tree -->
  <div class="relative border-b border-border/50 px-2 py-1.5">
    <span class="absolute left-3.5 top-1/2 -translate-y-1/2 text-fg-subtle">
      <Icon name="search" size={12} />
    </span>
    <input
      bind:value={filter}
      placeholder="Filter files…"
      spellcheck="false"
      class="w-full rounded border border-border bg-surface pl-7 pr-6 text-[11.5px] text-fg
             placeholder:text-fg-subtle focus:border-accent/60 focus:outline-none"
      style="height: 26px"
    />
    {#if filter}
      <button
        type="button"
        onclick={() => (filter = "")}
        class="absolute right-3.5 top-1/2 -translate-y-1/2 rounded p-0.5 text-fg-subtle hover:text-fg"
        aria-label="Clear filter"
      >
        <Icon name="x" size={12} />
      </button>
    {/if}
  </div>

  <!-- Tree -->
  <div
    bind:this={treeEl}
    class="min-h-0 flex-1 overflow-auto py-1 {dragOverPath === root ? 'ring-1 ring-inset ring-accent/60 bg-accent/5' : ''}"
    role="tree"
    tabindex="-1"
    oncontextmenu={(e) => openMenu(e, null)}
  >
    {#if connecting}
      <p class="p-4 text-center text-[12px] text-fg-subtle">Connecting…</p>
    {:else if connectError}
      <p class="m-3 rounded-md border border-status-crashed/40 bg-status-crashed/10 p-2.5 text-[12px] text-status-crashed">
        {connectError}
      </p>
    {:else if (children[root] ?? []).length === 0}
      <p class="p-4 text-center text-[12px] text-fg-subtle">This folder is empty.</p>
    {:else}
      {#each children[root] ?? [] as entry (entry.path)}
        {@render node(entry, 0)}
      {/each}
      {#if fq && !(children[root] ?? []).some(hasMatch)}
        <p class="p-4 text-center text-[12px] text-fg-subtle">No loaded files match “{filter}”.</p>
      {/if}
    {/if}
  </div>
</div>

<!-- A tree row, recursing into expanded directories. Hidden when a filter is
     active and neither it nor any loaded descendant matches; a filter
     force-expands matching directories so the hits are visible. -->
{#snippet node(entry: SftpEntry, depth: number)}
  {#if hasMatch(entry)}
    {@const open = fq ? true : expanded[entry.path]}
    <div>
      <button
        type="button"
        onclick={() => clickEntry(entry)}
        oncontextmenu={(e) => openMenu(e, entry)}
        aria-current={activePath === entry.path ? "true" : undefined}
        class="group flex w-full items-center gap-1 py-0.5 pr-2 text-left text-[12.5px] hover:bg-surface-2/60
          {activePath === entry.path ? 'bg-accent/10 text-fg' : 'text-fg-muted'}
          {dragOverPath === entry.path ? 'ring-1 ring-inset ring-accent/60' : ''}"
        style="padding-left: {depth * 12 + 8}px"
      >
        {#if entry.isDir}
          <Icon
            name={open ? "chevron-down" : "chevron-right"}
            size={12}
            class="shrink-0 text-fg-subtle"
          />
        {:else}
          <span class="inline-block w-3 shrink-0"></span>
        {/if}
        <Icon name={iconFor(entry)} size={14} class={entry.isDir ? "shrink-0 text-accent" : "shrink-0 text-fg-subtle"} />
        <span class="truncate">{entry.name}{entry.isSymlink ? " ↗" : ""}</span>
      </button>
      {#if entry.isDir && open}
        {#if loadingDir[entry.path] && children[entry.path] === undefined}
          <p class="py-0.5 text-[11px] text-fg-subtle" style="padding-left: {(depth + 1) * 12 + 24}px">Loading…</p>
        {:else}
          {#each children[entry.path] ?? [] as child (child.path)}
            {@render node(child, depth + 1)}
          {/each}
          {#if !fq && (children[entry.path] ?? []).length === 0}
            <p class="py-0.5 text-[11px] italic text-fg-subtle" style="padding-left: {(depth + 1) * 12 + 24}px">empty</p>
          {/if}
        {/if}
      {/if}
    </div>
  {/if}
{/snippet}

<!-- Context menu.
     Every value read here is null-safe (`menu?.…`): an action sets `menu = null`,
     which re-evaluates these reactive reads to null *before* the `{#if menu}`
     unmounts the block — reading a property off the bare value would then throw
     (`$.get(m).entry`). Each handler also snapshots the entry into a plain local
     before nulling `menu`, since the reactive `entry` collapses to null too. -->
{#if menu}
  {@const x = menu?.x ?? 0}
  {@const y = menu?.y ?? 0}
  {@const entry = menu?.entry ?? null}
  <div
    class="fixed z-50 w-44 rounded-lg border border-border bg-surface p-1 shadow-xl"
    style="left: {Math.min(x, (typeof window !== 'undefined' ? window.innerWidth : 9999) - 190)}px; top: {y}px"
    role="menu"
    tabindex="-1"
  >
    <button type="button" role="menuitem" onclick={() => { const en = entry; menu = null; newFile(en); }} class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg">
      <Icon name="file-plus" size={13} /> New file
    </button>
    <button type="button" role="menuitem" onclick={() => { const en = entry; menu = null; newFolder(en); }} class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg">
      <Icon name="folder-plus" size={13} /> New folder
    </button>
    <button type="button" role="menuitem" onclick={() => { const en = entry; menu = null; uploadInto(en); }} class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg">
      <Icon name="share" size={13} /> Upload here
    </button>
    {#if entry}
      <div class="my-1 border-t border-border/60"></div>
      {#if !entry.isDir}
        <button type="button" role="menuitem" onclick={() => { const en = entry; if (!en) return; menu = null; void download(en); }} class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg">
          <Icon name="arrow-up" size={13} class="rotate-180" /> Download
        </button>
      {/if}
      <button type="button" role="menuitem" onclick={() => { const en = entry; if (!en) return; menu = null; renameEntry(en); }} class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg">
        <Icon name="pencil" size={13} /> Rename
      </button>
      <button type="button" role="menuitem" onclick={() => { const en = entry; if (!en) return; menu = null; chmodEntry(en); }} class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg">
        <Icon name="lock" size={13} /> Permissions
      </button>
      <button type="button" role="menuitem" onclick={() => { const en = entry; if (!en) return; menu = null; confirmDelete = en; }} class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-status-crashed hover:bg-status-crashed/10">
        <Icon name="trash-2" size={13} /> Delete
      </button>
    {/if}
  </div>
{/if}

<!-- Text-prompt modal (new file/folder / rename / chmod) -->
{#if prompt}
  {@const p = prompt}
  <div
    class="fixed inset-0 z-[60] flex items-center justify-center bg-black/40 p-4"
    role="presentation"
    onclick={(e) => { if (e.target === e.currentTarget) prompt = null; }}
  >
    <div class="w-full max-w-sm rounded-xl border border-border bg-surface p-4 shadow-2xl" role="dialog" aria-modal="true">
      <h3 class="mb-2 text-[13px] font-semibold text-fg">{p?.title}</h3>
      <!-- svelte-ignore a11y_autofocus -->
      <input
        bind:value={p.value}
        autofocus
        onkeydown={(e) => e.key === "Enter" && submitPrompt()}
        class="w-full rounded-md border border-border bg-surface-2 px-2 py-1.5 text-[12px] text-fg outline-none focus:border-accent"
      />
      {#if p?.hint}<p class="mt-1.5 truncate text-[11px] text-fg-subtle" title={p.hint}>{p.hint}</p>{/if}
      <div class="mt-3 flex justify-end gap-2">
        <button type="button" onclick={() => (prompt = null)} class="h-8 rounded-md px-3 text-[12px] text-fg-muted hover:bg-surface-2">Cancel</button>
        <button type="button" onclick={submitPrompt} class="h-8 rounded-md bg-accent px-3 text-[12px] font-medium text-on-accent hover:brightness-110">{p?.confirmLabel}</button>
      </div>
    </div>
  </div>
{/if}

<!-- Delete confirm -->
{#if confirmDelete}
  {@const d = confirmDelete}
  <div
    class="fixed inset-0 z-[60] flex items-center justify-center bg-black/40 p-4"
    role="presentation"
    onclick={(e) => { if (e.target === e.currentTarget) confirmDelete = null; }}
  >
    <div class="w-full max-w-sm rounded-xl border border-border bg-surface p-4 shadow-2xl" role="dialog" aria-modal="true">
      <h3 class="text-[13px] font-semibold text-fg">Delete "{d?.name}"?</h3>
      <p class="mt-1.5 text-[12px] text-fg-muted">
        {d?.isDir ? "The folder must be empty. " : ""}This can't be undone.
      </p>
      <div class="mt-3 flex justify-end gap-2">
        <button type="button" onclick={() => (confirmDelete = null)} class="h-8 rounded-md px-3 text-[12px] text-fg-muted hover:bg-surface-2">Cancel</button>
        <button type="button" onclick={doDelete} class="h-8 rounded-md bg-status-crashed px-3 text-[12px] font-medium text-white hover:brightness-110">Delete</button>
      </div>
    </div>
  </div>
{/if}
