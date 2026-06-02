<!--
  FileBrowserPane — the SFTP file-manager body: browse the remote host over the
  cached SFTP session, with upload/download (native dialogs), mkdir, rename,
  chmod, delete, and edit-and-push for text files.

  This is the presentational core shared by two hosts: the modal wrapper
  (FileBrowser.svelte, opened from the fileBrowser store) and the SSH host
  workspace's Files tab. It fills its container (`h-full`) and renders its own
  header; the optional `onClose` adds a close button (the modal passes it, the
  embedded tab does not). The pane owns the Escape cascade (close the open
  sub-modal first, then the pane) and its own connect-on-mount.
-->
<script lang="ts">
  import { onMount } from "svelte";

  import { browser } from "$app/environment";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import LocalFilePane from "$lib/components/connections/LocalFilePane.svelte";
  import { invokeQuiet } from "$lib/ipc";
  import { connectWithPrompt } from "$lib/ssh/connectWithPrompt";
  import { confirmDialog } from "$lib/stores/confirm.svelte";
  import { sftpTransfers, type Transfer } from "$lib/stores/sftpTransfers.svelte";
  import {
    sftpListDir,
    sftpMkdir,
    sftpRename,
    sftpRemoveFile,
    sftpRemoveDir,
    sftpChmod,
    sftpReadText,
    sftpReadPreview,
    sftpWriteText,
    posixJoin,
    posixParent,
    formatMode,
    formatSize,
    type SftpPreview,
  } from "$lib/sftp";
  import type { SftpEntry } from "$lib/types/sshTunnels";

  let {
    connectionId,
    label,
    onClose,
  }: { connectionId: string; label: string; onClose?: () => void } = $props();

  let cwd = $state<string>("");
  let entries = $state<SftpEntry[]>([]);
  let loading = $state(false);
  let navError = $state<string | null>(null);

  // Live name filter over the current directory's listing.
  let query = $state("");
  const visibleEntries = $derived.by(() => {
    const q = query.trim().toLowerCase();
    if (!q) return entries;
    return entries.filter((e) => e.name.toLowerCase().includes(q));
  });

  // A single text-prompt modal serves mkdir / rename.
  type Prompt = {
    title: string;
    value: string;
    hint?: string;
    confirmLabel: string;
    onConfirm: (value: string) => Promise<void> | void;
  };
  let prompt = $state<Prompt | null>(null);
  let confirmDelete = $state<SftpEntry | null>(null);

  // chmod editor: an rwx grid (owner/group/other) over the live mode bits.
  let chmod = $state<{ path: string; name: string; mode: number } | null>(null);
  // 0o400,0o200,0o100 / 0o040,0o020,0o010 / 0o004,0o002,0o001 — r/w/x per class.
  const CHMOD_CLASSES: { label: string; shift: number }[] = [
    { label: "Owner", shift: 6 },
    { label: "Group", shift: 3 },
    { label: "Others", shift: 0 },
  ];
  const CHMOD_BITS: { label: string; bit: number }[] = [
    { label: "r", bit: 4 },
    { label: "w", bit: 2 },
    { label: "x", bit: 1 },
  ];
  function chmodHas(shift: number, bit: number): boolean {
    return chmod !== null && (((chmod.mode >> shift) & 7) & bit) !== 0;
  }
  function chmodToggle(shift: number, bit: number) {
    if (!chmod) return;
    chmod = { ...chmod, mode: chmod.mode ^ (bit << shift) };
  }
  async function applyChmod() {
    if (!chmod) return;
    const c = chmod;
    chmod = null;
    try {
      await sftpChmod(connectionId, c.path, c.mode & 0o777);
      refresh();
    } catch {
      /* toasted */
    }
  }

  // Edit-and-push modal.
  let edit = $state<{ path: string; name: string; content: string } | null>(null);
  let editSaving = $state(false);

  // Preview modal (images / non-editable text / binary).
  let preview = $state<{ name: string; path: string; data: SftpPreview } | null>(null);
  let previewLoading = $state<string | null>(null);

  // Dual-pane: show a local-filesystem browser beside the remote listing.
  let dualPane = $state(false);

  // Transfer queue (global store) — progress popover in the footer.
  let showTransfers = $state(false);
  const transfers = $derived(sftpTransfers.value);
  function transferPct(t: Transfer): number {
    if (t.status === "done") return 100;
    if (t.total <= 0) return 0;
    return Math.min(100, Math.round((t.transferred / t.total) * 100));
  }

  // Multi-select for batch download / delete, keyed by remote path.
  let selected = $state<Set<string>>(new Set());
  let batchBusy = $state(false);
  const selectedCount = $derived(selected.size);
  const allVisibleSelected = $derived.by(
    () => visibleEntries.length > 0 && visibleEntries.every((e) => selected.has(e.path)),
  );
  function toggleSelect(path: string) {
    const next = new Set(selected);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    selected = next;
  }
  function toggleSelectAll() {
    if (allVisibleSelected) selected = new Set();
    else selected = new Set(visibleEntries.map((e) => e.path));
  }
  function clearSelection() {
    selected = new Set();
  }

  // OS drag-and-drop upload, scoped to this pane's bounding box.
  let paneEl = $state<HTMLDivElement | null>(null);
  let dragOver = $state(false);

  function uploadPaths(localPaths: string[]) {
    const dir = cwd;
    for (const local of localPaths) {
      const name = localBasename(local);
      sftpTransfers.enqueueUpload(connectionId, local, posixJoin(dir, name), name, () => {
        if (cwd === dir) refresh();
      });
    }
  }

  async function batchDownload() {
    const files = [...selected]
      .map((p) => entries.find((e) => e.path === p))
      .filter((e): e is SftpEntry => !!e && !e.isDir);
    if (files.length === 0) return;
    const { open } = await import("@tauri-apps/plugin-dialog");
    const dir = await open({ directory: true, multiple: false, title: "Download selected to…" });
    if (typeof dir !== "string") return;
    for (const f of files) {
      sftpTransfers.enqueueDownload(connectionId, f.path, `${dir}/${f.name}`, f.name);
    }
    clearSelection();
  }

  async function batchDelete() {
    const targets = [...selected]
      .map((p) => entries.find((e) => e.path === p))
      .filter((e): e is SftpEntry => !!e);
    if (targets.length === 0) return;
    const choice = await confirmDialog.open({
      title: `Delete ${targets.length} item${targets.length === 1 ? "" : "s"}?`,
      message: "Folders must be empty. This can't be undone.",
      destructive: true,
      icon: "trash-2",
      actions: [{ label: "Delete", value: "delete", tone: "destructive", icon: "trash-2" }],
    });
    if (choice !== "delete") return;
    batchBusy = true;
    try {
      for (const e of targets) {
        try {
          if (e.isDir) await sftpRemoveDir(connectionId, e.path);
          else await sftpRemoveFile(connectionId, e.path);
        } catch {
          /* toasted */
        }
      }
      clearSelection();
      refresh();
    } finally {
      batchBusy = false;
    }
  }

  const TEXT_EXT =
    /\.(txt|md|json|ya?ml|toml|ini|conf|cfg|env|sh|bash|zsh|js|ts|jsx|tsx|mjs|cjs|css|scss|html|xml|svg|rs|py|rb|php|go|java|kt|c|h|cpp|hpp|cs|swift|sql|log|gitignore|dockerfile)$/i;

  const isTextFile = (e: SftpEntry) =>
    !e.isDir && (TEXT_EXT.test(e.name) || !e.name.includes("."));

  const breadcrumbs = $derived.by(() => {
    if (!cwd || cwd === "/") return [{ name: "/", path: "/" }];
    const parts = cwd.split("/").filter(Boolean);
    let acc = "";
    const crumbs = [{ name: "/", path: "/" }];
    for (const p of parts) {
      acc = `${acc}/${p}`;
      crumbs.push({ name: p, path: acc });
    }
    return crumbs;
  });

  async function navigate(path: string) {
    loading = true;
    navError = null;
    try {
      entries = await sftpListDir(connectionId, path);
      cwd = path;
      query = "";
      selected = new Set();
    } catch (e) {
      navError = e instanceof Error ? e.message : "Couldn't open that folder.";
    } finally {
      loading = false;
    }
  }

  function refresh() {
    void navigate(cwd);
  }

  function openEntry(e: SftpEntry) {
    if (e.isDir) void navigate(e.path);
    else if (isTextFile(e)) void openEditor(e);
    else void openPreview(e);
  }

  async function openPreview(e: SftpEntry) {
    if (previewLoading) return;
    previewLoading = e.path;
    try {
      const data = await sftpReadPreview(connectionId, e.path);
      preview = { name: e.name, path: e.path, data };
    } catch {
      /* toasted (too large / unreadable) */
    } finally {
      previewLoading = null;
    }
  }

  function newFolder() {
    prompt = {
      title: "New folder",
      value: "",
      hint: "Created in the current directory.",
      confirmLabel: "Create",
      onConfirm: async (name) => {
        const trimmed = name.trim();
        if (!trimmed) return;
        await sftpMkdir(connectionId, posixJoin(cwd, trimmed));
        refresh();
      },
    };
  }

  function renameEntry(e: SftpEntry) {
    prompt = {
      title: `Rename "${e.name}"`,
      value: e.name,
      confirmLabel: "Rename",
      onConfirm: async (name) => {
        const trimmed = name.trim();
        if (!trimmed || trimmed === e.name) return;
        await sftpRename(connectionId, e.path, posixJoin(posixParent(e.path), trimmed));
        refresh();
      },
    };
  }

  function chmodEntry(e: SftpEntry) {
    chmod = {
      path: e.path,
      name: e.name,
      mode: (e.permissions ?? 0o644) & 0o777,
    };
  }

  async function submitPrompt() {
    if (!prompt) return;
    const p = prompt;
    prompt = null;
    try {
      await p.onConfirm(p.value);
    } catch {
      /* safeInvoke already toasted */
    }
  }

  async function doDelete() {
    const e = confirmDelete;
    confirmDelete = null;
    if (!e) return;
    try {
      if (e.isDir) await sftpRemoveDir(connectionId, e.path);
      else await sftpRemoveFile(connectionId, e.path);
      refresh();
    } catch {
      /* toasted */
    }
  }

  async function upload() {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const picked = await open({ multiple: true, directory: false });
    const paths = Array.isArray(picked) ? picked : typeof picked === "string" ? [picked] : [];
    if (paths.length) uploadPaths(paths);
  }

  async function download(e: SftpEntry) {
    const { save } = await import("@tauri-apps/plugin-dialog");
    const dest = await save({ defaultPath: e.name });
    if (typeof dest !== "string") return;
    sftpTransfers.enqueueDownload(connectionId, e.path, dest, e.name);
  }

  async function openEditor(e: SftpEntry) {
    try {
      const content = await sftpReadText(connectionId, e.path);
      edit = { path: e.path, name: e.name, content };
    } catch {
      /* toasted (e.g. non-UTF-8) */
    }
  }

  async function saveEditor() {
    if (!edit) return;
    editSaving = true;
    try {
      await sftpWriteText(connectionId, edit.path, edit.content);
      edit = null;
    } catch {
      /* toasted */
    } finally {
      editSaving = false;
    }
  }

  /** Local-path basename — handles both `/` and Windows `\` separators. */
  function localBasename(p: string): string {
    const seg = p.split(/[\\/]/).filter(Boolean);
    return seg.length ? seg[seg.length - 1] : p;
  }

  function iconFor(e: SftpEntry): "folder" | "file-code" | "file-text" {
    if (e.isDir) return "folder";
    return /\.(rs|ts|js|tsx|jsx|py|rb|php|go|java|c|h|cpp|json|sh|svelte|vue)$/i.test(e.name)
      ? "file-code"
      : "file-text";
  }

  function onKeydown(ev: KeyboardEvent) {
    if (ev.key === "Escape") {
      if (preview) preview = null;
      else if (edit) edit = null;
      else if (chmod) chmod = null;
      else if (prompt) prompt = null;
      else if (confirmDelete) confirmDelete = null;
      else onClose?.();
    }
  }

  onMount(() => {
    void (async () => {
      try {
        // Open + cache the SFTP session first, prompting once (VS Code-style)
        // for a one-shot password/passphrase if the host needs it. Subsequent
        // sftp_* calls reuse the cached session, so they never re-prompt or
        // flood toasts.
        const home = await connectWithPrompt(connectionId, label, (cred) =>
          invokeQuiet<string>("sftp_connect", {
            connectionId,
            password: cred?.kind === "password" ? cred.secret : undefined,
            passphrase: cred?.kind === "passphrase" ? cred.secret : undefined,
          }),
        );
        await navigate(home || "/");
      } catch {
        // connectWithPrompt already surfaced any real failure (or the user
        // cancelled). Leave the browser empty rather than firing another connect.
        navError = "Couldn't connect to this host.";
      }
    })();
  });

  function withinPane(pos?: { x: number; y: number }): boolean {
    if (!pos || !paneEl) return false;
    const r = paneEl.getBoundingClientRect();
    return pos.x >= r.left && pos.x <= r.right && pos.y >= r.top && pos.y <= r.bottom;
  }

  // OS file drop → upload into the current directory, scoped to this pane so it
  // doesn't collide with other drop targets (file tree, project drop).
  $effect(() => {
    if (!browser) return;
    let unlisten: (() => void) | null = null;
    void (async () => {
      const { getCurrentWebview } = await import("@tauri-apps/api/webview");
      unlisten = await getCurrentWebview().onDragDropEvent((event) => {
        const t = event.payload.type;
        const pos = (event.payload as { position?: { x: number; y: number } }).position;
        if (t === "drop") {
          dragOver = false;
          if (!withinPane(pos)) return;
          const paths = (event.payload as { paths?: string[] }).paths ?? [];
          if (paths.length) void uploadPaths(paths);
        } else if (t === "leave") {
          dragOver = false;
        } else if (t === "enter" || t === "over") {
          dragOver = withinPane(pos);
        }
      });
    })();
    return () => unlisten?.();
  });
</script>

<svelte:window onkeydown={onKeydown} />

<div bind:this={paneEl} class="relative flex h-full min-h-0 flex-col">
  {#if dragOver}
    <div class="pointer-events-none absolute inset-0 z-50 m-2 flex items-center justify-center rounded-lg border-2 border-dashed border-accent bg-accent/10">
      <span class="rounded-md bg-surface px-3 py-1.5 text-[12.5px] font-medium text-fg shadow">
        Drop to upload to {cwd || "/"}
      </span>
    </div>
  {/if}

  <!-- Header -->
  <header class="flex items-center gap-2 border-b border-border px-4 py-3">
    <Icon name="server" size={15} class="text-fg-muted" />
    <div class="min-w-0 flex-1">
      <h2 class="truncate text-[13px] font-semibold text-fg">Files · {label}</h2>
      <p class="truncate font-mono text-[11px] text-fg-subtle">{cwd || "…"}</p>
    </div>
    {#if onClose}
      <button
        type="button"
        onclick={onClose}
        class="rounded-md p-1.5 text-fg-muted hover:bg-surface-2 hover:text-fg"
        aria-label="Close"
      >
        <Icon name="x" size={16} />
      </button>
    {/if}
  </header>

  <!-- Toolbar -->
  <div class="flex items-center gap-1 border-b border-border bg-surface-2/40 px-3 py-2">
    <button
      type="button"
      onclick={() => navigate(posixParent(cwd))}
      disabled={cwd === "/" || cwd === ""}
      class="inline-flex h-7 items-center gap-1 rounded-md px-2 text-[12px] text-fg-muted hover:bg-surface hover:text-fg disabled:opacity-40"
      title="Up one folder"
    >
      <Icon name="arrow-up" size={13} />
    </button>
    <button
      type="button"
      onclick={refresh}
      class="inline-flex h-7 items-center gap-1 rounded-md px-2 text-[12px] text-fg-muted hover:bg-surface hover:text-fg"
      title="Refresh"
    >
      <Icon name="refresh-cw" size={13} class={loading ? "animate-spin" : ""} />
    </button>

    <!-- Breadcrumbs -->
    <nav class="mx-1 flex min-w-0 flex-1 items-center gap-0.5 overflow-x-auto text-[12px]">
      {#each breadcrumbs as crumb, i (crumb.path)}
        {#if i > 0}
          <Icon name="chevron-right" size={11} class="shrink-0 text-fg-subtle" />
        {/if}
        <button
          type="button"
          onclick={() => navigate(crumb.path)}
          class="shrink-0 rounded px-1.5 py-0.5 text-fg-muted hover:bg-surface hover:text-fg"
        >
          {crumb.name === "/" ? "" : crumb.name}{crumb.name === "/" ? "root" : ""}
        </button>
      {/each}
    </nav>

    <button
      type="button"
      onclick={newFolder}
      class="inline-flex h-7 items-center gap-1 rounded-md px-2 text-[12px] text-fg-muted hover:bg-surface hover:text-fg"
    >
      <Icon name="plus" size={13} /> Folder
    </button>
    <button
      type="button"
      onclick={() => (dualPane = !dualPane)}
      class="inline-flex h-7 items-center gap-1 rounded-md px-2 text-[12px] {dualPane ? 'bg-surface-2 text-fg' : 'text-fg-muted hover:bg-surface hover:text-fg'}"
      title="Toggle local file pane"
      aria-pressed={dualPane}
    >
      <Icon name="panel-left-open" size={13} /> Local
    </button>
    <button
      type="button"
      onclick={upload}
      class="inline-flex h-7 items-center gap-1 rounded-md bg-accent px-2.5 text-[12px] font-medium text-on-accent hover:brightness-110"
    >
      <Icon name="share" size={13} /> Upload
    </button>
  </div>

  <!-- Search: live filter over the current folder -->
  <div class="relative border-b border-border px-3 py-1.5">
    <span class="absolute left-5 top-1/2 -translate-y-1/2 text-fg-subtle">
      <Icon name="search" size={13} />
    </span>
    <input
      bind:value={query}
      placeholder="Search this folder…"
      spellcheck="false"
      class="w-full h-7 rounded-md border border-border bg-surface pl-7 pr-7 text-[12px] text-fg
             placeholder:text-fg-subtle focus:border-accent/60 focus:outline-none"
    />
    {#if query}
      <button
        type="button"
        onclick={() => (query = "")}
        class="absolute right-5 top-1/2 -translate-y-1/2 rounded p-0.5 text-fg-subtle hover:text-fg"
        aria-label="Clear search"
      >
        <Icon name="x" size={13} />
      </button>
    {/if}
  </div>

  <!-- Batch action bar (multi-select) -->
  {#if selectedCount > 0}
    <div class="flex items-center gap-2 border-b border-border bg-accent/10 px-3 py-1.5 text-[12px]">
      <span class="font-medium text-fg">{selectedCount} selected</span>
      <div class="ml-auto flex items-center gap-1">
        <button
          type="button"
          onclick={batchDownload}
          disabled={batchBusy}
          class="inline-flex h-7 items-center gap-1 rounded-md px-2 text-fg-muted hover:bg-surface hover:text-fg disabled:opacity-50"
        >
          <Icon name="arrow-up" size={13} class="rotate-180" /> Download
        </button>
        <button
          type="button"
          onclick={batchDelete}
          disabled={batchBusy}
          class="inline-flex h-7 items-center gap-1 rounded-md px-2 text-status-crashed hover:bg-status-crashed/10 disabled:opacity-50"
        >
          <Icon name="trash-2" size={13} /> Delete
        </button>
        <button
          type="button"
          onclick={clearSelection}
          class="inline-flex h-7 items-center gap-1 rounded-md px-2 text-fg-muted hover:bg-surface hover:text-fg"
        >
          <Icon name="x" size={13} /> Clear
        </button>
      </div>
    </div>
  {/if}

  <!-- Listing (optionally split with the local pane) -->
  <div class="flex min-h-0 flex-1">
    {#if dualPane}
      <div class="w-1/2 min-w-0 shrink-0 border-r border-border">
        <LocalFilePane {connectionId} remoteCwd={cwd} onUploaded={() => refresh()} />
      </div>
    {/if}
    <div class="min-h-0 flex-1 overflow-y-auto">
    {#if navError}
      <div class="m-4 rounded-md border border-status-crashed/40 bg-status-crashed/10 p-3 text-[12px] text-status-crashed">
        {navError}
      </div>
    {:else if loading && entries.length === 0}
      <p class="p-6 text-center text-[12px] text-fg-subtle">Loading…</p>
    {:else if entries.length === 0}
      <p class="p-6 text-center text-[12px] text-fg-subtle">This folder is empty.</p>
    {:else if visibleEntries.length === 0}
      <p class="p-6 text-center text-[12px] text-fg-subtle">No files match “{query}”.</p>
    {:else}
      <table class="w-full text-[12px]">
        <thead class="sticky top-0 bg-surface text-left text-[11px] uppercase text-fg-subtle">
          <tr class="border-b border-border">
            <th class="w-8 pl-4 py-1.5">
              <input
                type="checkbox"
                checked={allVisibleSelected}
                onchange={toggleSelectAll}
                class="rounded border-border accent-accent"
                aria-label="Select all"
              />
            </th>
            <th class="px-2 py-1.5 font-medium">Name</th>
            <th class="px-2 py-1.5 text-right font-medium">Size</th>
            <th class="px-2 py-1.5 font-medium">Perms</th>
            <th class="px-4 py-1.5 text-right font-medium">Actions</th>
          </tr>
        </thead>
        <tbody>
          {#each visibleEntries as e (e.path)}
            <tr class="group border-b border-border/40 hover:bg-surface-2/50 {selected.has(e.path) ? 'bg-accent/5' : ''}">
              <td class="w-8 pl-4 py-1.5">
                <input
                  type="checkbox"
                  checked={selected.has(e.path)}
                  onchange={() => toggleSelect(e.path)}
                  class="rounded border-border accent-accent"
                  aria-label={`Select ${e.name}`}
                />
              </td>
              <td class="px-2 py-1.5">
                <button
                  type="button"
                  onclick={() => openEntry(e)}
                  class="inline-flex items-center gap-2 text-left text-fg hover:text-accent"
                  title={e.isDir ? "Open" : isTextFile(e) ? "Edit" : e.name}
                >
                  <Icon
                    name={iconFor(e)}
                    size={14}
                    class={e.isDir ? "text-accent" : "text-fg-subtle"}
                  />
                  <span class="truncate">{e.name}{e.isSymlink ? " ↗" : ""}</span>
                </button>
              </td>
              <td class="px-2 py-1.5 text-right font-mono text-fg-muted">
                {e.isDir ? "—" : formatSize(e.size)}
              </td>
              <td class="px-2 py-1.5 font-mono text-fg-subtle">{formatMode(e.permissions)}</td>
              <td class="px-4 py-1.5">
                <div class="flex items-center justify-end gap-0.5 opacity-0 group-hover:opacity-100">
                  {#if !e.isDir}
                    <button
                      type="button"
                      onclick={() => download(e)}
                      class="rounded p-1 text-fg-muted hover:bg-surface hover:text-fg"
                      title="Download"
                    >
                      <Icon name="arrow-up" size={13} class="rotate-180" />
                    </button>
                  {/if}
                  <button
                    type="button"
                    onclick={() => renameEntry(e)}
                    class="rounded p-1 text-fg-muted hover:bg-surface hover:text-fg"
                    title="Rename"
                  >
                    <Icon name="pencil" size={13} />
                  </button>
                  <button
                    type="button"
                    onclick={() => chmodEntry(e)}
                    class="rounded p-1 text-fg-muted hover:bg-surface hover:text-fg"
                    title="Permissions"
                  >
                    <Icon name="lock" size={13} />
                  </button>
                  <button
                    type="button"
                    onclick={() => (confirmDelete = e)}
                    class="rounded p-1 text-fg-muted hover:bg-status-crashed/10 hover:text-status-crashed"
                    title="Delete"
                  >
                    <Icon name="trash-2" size={13} />
                  </button>
                </div>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
    </div>
  </div>

  <footer class="relative flex items-center gap-2 border-t border-border px-4 py-1.5 text-[11px] text-fg-subtle">
    <span>
      {#if query.trim() && entries.length}
        {visibleEntries.length} of {entries.length} item{entries.length === 1 ? "" : "s"}
      {:else}
        {entries.length} item{entries.length === 1 ? "" : "s"}
      {/if}
    </span>

    {#if transfers.length > 0}
      <button
        type="button"
        onclick={() => (showTransfers = !showTransfers)}
        class="ml-auto inline-flex items-center gap-1.5 rounded px-1.5 py-0.5 text-fg-muted hover:bg-surface-2 hover:text-fg"
      >
        <Icon name="refresh-cw" size={11} class={sftpTransfers.activeCount > 0 ? "animate-spin" : ""} />
        {#if sftpTransfers.activeCount > 0}
          {sftpTransfers.activeCount} transferring…
        {:else}
          Transfers
        {/if}
      </button>

      {#if showTransfers}
        <button type="button" class="fixed inset-0 z-40 cursor-default" aria-label="Close transfers" onclick={() => (showTransfers = false)}></button>
        <div class="absolute bottom-8 right-3 z-50 w-80 overflow-hidden rounded-lg border border-border bg-surface shadow-2xl">
          <div class="flex items-center justify-between border-b border-border px-3 py-2">
            <span class="text-[12px] font-semibold text-fg">Transfers</span>
            <button type="button" onclick={() => sftpTransfers.clearFinished()} class="text-[11px] text-fg-muted hover:text-fg">Clear finished</button>
          </div>
          <ul class="max-h-64 overflow-y-auto p-1.5">
            {#each transfers as t (t.id)}
              <li class="rounded-md px-2 py-1.5">
                <div class="flex items-center gap-2">
                  <Icon
                    name={t.direction === "upload" ? "arrow-up" : "arrow-up"}
                    size={11}
                    class={t.direction === "download" ? "rotate-180 text-fg-subtle" : "text-fg-subtle"}
                  />
                  <span class="min-w-0 flex-1 truncate text-[12px] text-fg" title={t.name}>{t.name}</span>
                  <span class="shrink-0 text-[10.5px] {t.status === 'error' ? 'text-status-crashed' : 'text-fg-subtle'}">
                    {#if t.status === "error"}failed{:else if t.status === "done"}done{:else if t.status === "pending"}queued{:else}{transferPct(t)}%{/if}
                  </span>
                </div>
                <div class="mt-1 h-1 overflow-hidden rounded-full bg-surface-2">
                  <div
                    class="h-full rounded-full transition-[width] {t.status === 'error' ? 'bg-status-crashed' : t.status === 'done' ? 'bg-status-running' : 'bg-accent'}"
                    style="width: {t.status === 'done' ? 100 : transferPct(t)}%"
                  ></div>
                </div>
                {#if t.error}<p class="mt-0.5 truncate text-[10.5px] text-status-crashed" title={t.error}>{t.error}</p>{/if}
              </li>
            {/each}
          </ul>
        </div>
      {/if}
    {/if}
  </footer>
</div>

<!-- Text-prompt modal (mkdir / rename / chmod) -->
{#if prompt}
  {@const p = prompt}
  <div
    class="fixed inset-0 z-[60] flex items-center justify-center bg-black/40 p-4"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) prompt = null;
    }}
  >
    <div class="w-full max-w-sm rounded-xl border border-border bg-surface p-4 shadow-2xl" role="dialog" aria-modal="true">
      <h3 class="mb-2 text-[13px] font-semibold text-fg">{p.title}</h3>
      <!-- svelte-ignore a11y_autofocus -->
      <input
        bind:value={p.value}
        autofocus
        onkeydown={(e) => e.key === "Enter" && submitPrompt()}
        class="w-full rounded-md border border-border bg-surface-2 px-2 py-1.5 text-[12px] text-fg outline-none focus:border-accent"
      />
      {#if p.hint}<p class="mt-1.5 text-[11px] text-fg-subtle">{p.hint}</p>{/if}
      <div class="mt-3 flex justify-end gap-2">
        <button
          type="button"
          onclick={() => (prompt = null)}
          class="h-8 rounded-md px-3 text-[12px] text-fg-muted hover:bg-surface-2"
        >
          Cancel
        </button>
        <button
          type="button"
          onclick={submitPrompt}
          class="h-8 rounded-md bg-accent px-3 text-[12px] font-medium text-on-accent hover:brightness-110"
        >
          {p.confirmLabel}
        </button>
      </div>
    </div>
  </div>
{/if}

<!-- chmod rwx grid -->
{#if chmod}
  {@const c = chmod}
  <div
    class="fixed inset-0 z-[60] flex items-center justify-center bg-black/40 p-4"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) chmod = null;
    }}
  >
    <div class="w-full max-w-sm rounded-xl border border-border bg-surface p-4 shadow-2xl" role="dialog" aria-modal="true">
      <h3 class="text-[13px] font-semibold text-fg">Permissions</h3>
      <p class="mt-0.5 truncate font-mono text-[11px] text-fg-subtle">{c.name}</p>

      <div class="mt-3 grid grid-cols-[auto_repeat(3,1fr)] gap-x-3 gap-y-1.5 text-[12px]">
        <span></span>
        {#each CHMOD_BITS as b (b.bit)}
          <span class="text-center text-[11px] uppercase text-fg-subtle">{b.label}</span>
        {/each}
        {#each CHMOD_CLASSES as cls (cls.shift)}
          <span class="text-fg-muted">{cls.label}</span>
          {#each CHMOD_BITS as b (b.bit)}
            <label class="flex justify-center">
              <input
                type="checkbox"
                checked={chmodHas(cls.shift, b.bit)}
                onchange={() => chmodToggle(cls.shift, b.bit)}
                class="rounded border-border accent-accent"
                aria-label={`${cls.label} ${b.label}`}
              />
            </label>
          {/each}
        {/each}
      </div>

      <div class="mt-3 flex items-center gap-2 text-[12px] text-fg-subtle">
        <span class="font-mono text-fg">{(c.mode & 0o777).toString(8).padStart(3, "0")}</span>
        <span class="font-mono">{formatMode(c.mode)}</span>
      </div>

      <div class="mt-3 flex justify-end gap-2">
        <button type="button" onclick={() => (chmod = null)} class="h-8 rounded-md px-3 text-[12px] text-fg-muted hover:bg-surface-2">Cancel</button>
        <button type="button" onclick={applyChmod} class="h-8 rounded-md bg-accent px-3 text-[12px] font-medium text-on-accent hover:brightness-110">Apply</button>
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
    onclick={(e) => {
      if (e.target === e.currentTarget) confirmDelete = null;
    }}
  >
    <div class="w-full max-w-sm rounded-xl border border-border bg-surface p-4 shadow-2xl" role="dialog" aria-modal="true">
      <h3 class="text-[13px] font-semibold text-fg">Delete "{d.name}"?</h3>
      <p class="mt-1.5 text-[12px] text-fg-muted">
        {d.isDir ? "The folder must be empty. " : ""}This can't be undone.
      </p>
      <div class="mt-3 flex justify-end gap-2">
        <button
          type="button"
          onclick={() => (confirmDelete = null)}
          class="h-8 rounded-md px-3 text-[12px] text-fg-muted hover:bg-surface-2"
        >
          Cancel
        </button>
        <button
          type="button"
          onclick={doDelete}
          class="h-8 rounded-md bg-status-crashed px-3 text-[12px] font-medium text-white hover:brightness-110"
        >
          Delete
        </button>
      </div>
    </div>
  </div>
{/if}

<!-- Edit-and-push -->
{#if edit}
  {@const ed = edit}
  <div
    class="fixed inset-0 z-[60] flex items-center justify-center bg-black/50 p-4"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) edit = null;
    }}
  >
    <div class="flex h-[80vh] w-full max-w-3xl flex-col overflow-hidden rounded-xl border border-border bg-surface shadow-2xl" role="dialog" aria-modal="true">
      <header class="flex items-center gap-2 border-b border-border px-4 py-2.5">
        <Icon name="pencil" size={14} class="text-fg-muted" />
        <span class="flex-1 truncate font-mono text-[12px] text-fg">{ed.name}</span>
        <button
          type="button"
          onclick={() => (edit = null)}
          class="rounded-md p-1 text-fg-muted hover:bg-surface-2 hover:text-fg"
          aria-label="Close editor"
        >
          <Icon name="x" size={15} />
        </button>
      </header>
      <textarea
        bind:value={ed.content}
        spellcheck="false"
        class="min-h-0 flex-1 resize-none bg-surface px-4 py-3 font-mono text-[12px] leading-relaxed text-fg outline-none"
      ></textarea>
      <footer class="flex justify-end gap-2 border-t border-border px-4 py-2.5">
        <button
          type="button"
          onclick={() => (edit = null)}
          class="h-8 rounded-md px-3 text-[12px] text-fg-muted hover:bg-surface-2"
        >
          Cancel
        </button>
        <button
          type="button"
          onclick={saveEditor}
          disabled={editSaving}
          class="h-8 rounded-md bg-accent px-3 text-[12px] font-medium text-on-accent hover:brightness-110 disabled:opacity-50"
        >
          {editSaving ? "Saving…" : "Save to server"}
        </button>
      </footer>
    </div>
  </div>
{/if}

<!-- Preview (image / text / binary) -->
{#if preview}
  {@const pv = preview}
  <div
    class="fixed inset-0 z-[60] flex items-center justify-center bg-black/50 p-4"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) preview = null;
    }}
  >
    <div class="flex max-h-[85vh] w-full max-w-3xl flex-col overflow-hidden rounded-xl border border-border bg-surface shadow-2xl" role="dialog" aria-modal="true">
      <header class="flex items-center gap-2 border-b border-border px-4 py-2.5">
        <Icon name={pv.data.kind === "image" ? "image" : "file-text"} size={14} class="text-fg-muted" />
        <span class="flex-1 truncate font-mono text-[12px] text-fg">{pv.name}</span>
        <span class="shrink-0 text-[11px] text-fg-subtle">{formatSize(pv.data.size)}</span>
        <button type="button" onclick={() => (preview = null)} class="rounded-md p-1 text-fg-muted hover:bg-surface-2 hover:text-fg" aria-label="Close preview">
          <Icon name="x" size={15} />
        </button>
      </header>
      <div class="min-h-0 flex-1 overflow-auto bg-surface-2/30 p-4">
        {#if pv.data.kind === "image" && pv.data.base64}
          <img
            src={`data:${pv.data.mime};base64,${pv.data.base64}`}
            alt={pv.name}
            class="mx-auto max-h-full max-w-full rounded-md object-contain"
            style="image-rendering: auto"
          />
        {:else if pv.data.kind === "text"}
          <pre class="whitespace-pre-wrap break-words font-mono text-[12px] leading-relaxed text-fg">{pv.data.text}</pre>
        {:else}
          <div class="flex flex-col items-center justify-center gap-3 py-10 text-center">
            <Icon name="file-text" size={28} class="text-fg-subtle" />
            <p class="text-[12.5px] text-fg-muted">This file isn't text or a known image type.</p>
            <button
              type="button"
              onclick={() => { const e = { name: pv.name, path: pv.path, isDir: false } as SftpEntry; preview = null; void download(e); }}
              class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md text-[12px] font-medium bg-surface-2 text-fg hover:bg-surface-2/70"
            >
              <Icon name="arrow-up" size={12} class="rotate-180" /> Download
            </button>
          </div>
        {/if}
      </div>
    </div>
  </div>
{/if}
