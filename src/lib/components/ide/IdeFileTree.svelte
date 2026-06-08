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

  import { clampToViewport } from "$lib/actions/clampToViewport";
  import Icon from "$lib/components/atoms/Icon.svelte";
  import type { IconName } from "$lib/components/atoms/Icon.svelte";
  import { browser } from "$app/environment";
  import { invokeQuiet, safeInvoke } from "$lib/ipc";
  import {
    EMPTY_SELECTION,
    plainSelect,
    rangeSelect,
    toggleSelect as selectionToggle,
    type Selection,
  } from "$lib/listSelection";
  import { connectWithPrompt } from "$lib/ssh/connectWithPrompt";
  import {
    sftpListDir,
    sftpMkdir,
    sftpRename,
    sftpRemoveFile,
    sftpRemoveDir,
    sftpChmod,
    sftpWriteText,
    posixJoin,
    posixParent,
  } from "$lib/sftp";
  import { isArchive, archiveStem, extractArchive } from "$lib/sftpArchive";
  import { posixBasename } from "$lib/posixPath";
  import { remoteExists, pushNameTaken } from "$lib/sftpGuards";
  import { startSftpSearch, SftpSearchCache, type RunningSearch } from "$lib/sftpSearch";
  import { confirmDialog } from "$lib/stores/confirm.svelte";
  import { uploadWithConfirm } from "$lib/sftpUploadFlow";
  import { sftpTransfers } from "$lib/stores/sftpTransfers.svelte";
  import type { SftpEntry } from "$lib/types/sshTunnels";

  interface Props {
    connectionId: string;
    label: string;
    onOpenFile: (path: string) => void;
    /** Path of the currently-active editor file, for row highlighting. */
    activePath?: string | null;
    /**
     * Open the right-hand agent panel pointed at a directory (its working dir),
     * so a chat can run inside the folder you right-clicked. Omitted hosts hide
     * the "Open agent here" action.
     */
    onOpenAgentHere?: (dir: string) => void;
    /** Open a folder in the editor area's Files tab (Finder-style browser). */
    onOpenFolder?: (path: string) => void;
  }
  let {
    connectionId,
    label,
    onOpenFile,
    activePath = null,
    onOpenAgentHere,
    onOpenFolder,
  }: Props = $props();

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
  let confirmDelete = $state<SftpEntry[] | null>(null);

  // Multi-select (VS Code Explorer model): plain click selects the row it
  // lands on (and still opens/expands it), ⇧-click extends from the anchor
  // across the rows as rendered, ⌘/Ctrl-click toggles. Delete / the context
  // menu then act on the whole selection.
  let selected = $state<Set<string>>(new Set());
  let anchorPath = $state<string | null>(null);

  // Selection semantics live in $lib/listSelection (pure + unit-tested);
  // these adapters apply its results to the tree's rune state.
  const currentSel = (): Selection => ({ paths: selected, anchor: anchorPath });
  function applySel(s: Selection) {
    selected = new Set(s.paths);
    anchorPath = s.anchor;
  }

  function clearSelection() {
    applySel(EMPTY_SELECTION);
  }

  // The tree flattened in exactly the order the rows render (expanded dirs
  // inlined, filter applied) — this is the list ⇧-click ranges over.
  const visibleRows = $derived.by(() => {
    const out: SftpEntry[] = [];
    const walk = (list: SftpEntry[], forced: boolean) => {
      for (const e of list) {
        if (!forced && !hasMatch(e)) continue;
        out.push(e);
        if (e.isDir && expanded[e.path]) walk(children[e.path] ?? [], forced || nameMatches(e));
      }
    };
    walk(children[root] ?? [], false);
    return out;
  });

  /** Row click: ⇧ extends, ⌘/Ctrl toggles (neither opens); a plain click
      selects the row and opens/expands it like before. */
  function rowClick(ev: MouseEvent, entry: SftpEntry) {
    if (ev.shiftKey) {
      ev.preventDefault();
      applySel(
        rangeSelect(
          currentSel(),
          visibleRows.map((e) => e.path),
          entry.path,
          ev.metaKey || ev.ctrlKey,
        ),
      );
      return;
    }
    if (ev.metaKey || ev.ctrlKey) {
      applySel(selectionToggle(currentSel(), entry.path));
      return;
    }
    applySel(plainSelect(entry.path));
    // A folder click also opens it in the editor area's Files tab (the
    // Finder-style browser), alongside expanding it here.
    if (entry.isDir) onOpenFolder?.(entry.path);
    clickEntry(entry);
  }

  /** Find a loaded entry by path (selection can span several directories). */
  function entryByPath(path: string): SftpEntry | undefined {
    for (const list of Object.values(children)) {
      const hit = list.find((e) => e.path === path);
      if (hit) return hit;
    }
    return undefined;
  }

  /** What a delete on `entry` targets: the whole selection when the row is
      part of a multi-selection, else just that row (VS Code semantics). */
  function deleteTargets(entry: SftpEntry): SftpEntry[] {
    if (selected.has(entry.path) && selected.size > 1) {
      return [...selected].map(entryByPath).filter((e): e is SftpEntry => !!e);
    }
    return [entry];
  }

  // Live filter over the loaded tree. A node shows when its name matches or
  // any already-loaded descendant matches. Filtering only changes *which rows
  // show* — it never expands or collapses anything: a folder keeps exactly the
  // state the user left it in, and the chevrons keep working normally during a
  // search. (The loaded-tree filter alone can only see what's been expanded —
  // that's why deep search below is on by default.)
  let filter = $state("");
  const fq = $derived(filter.trim().toLowerCase());
  const nameMatches = (e: SftpEntry) => e.name.toLowerCase().includes(fq);

  // Deep (recursive) server search — the backend walks every subfolder under
  // the tree root and streams matches in (substring or glob, e.g. `*.zip`).
  // ON by default so searching finds entries in folders that were never
  // expanded; the layers button toggles down to loaded-tree-only filtering.
  let deepSearch = $state(true);
  let deepResults = $state<SftpEntry[] | null>(null);
  let deepScanned = $state(0);
  let deepRunning = $state(false);
  let deepTruncated = $state(false);
  let deepHandle: RunningSearch | null = null;
  let deepTimer: ReturnType<typeof setTimeout> | null = null;
  // Bumped on every (re)start/reset so stale batch callbacks are ignored.
  let deepToken = 0;
  // Completed walks, so retyped/extended queries answer without re-walking.
  const searchCache = new SftpSearchCache();
  // Set by the Stop button — a stopped walk is partial, so it isn't cached.
  let deepStopped = false;

  function resetDeep() {
    if (deepTimer) clearTimeout(deepTimer);
    deepTimer = null;
    deepHandle?.cancel();
    deepHandle = null;
    deepToken += 1;
    deepResults = null;
    deepRunning = false;
    deepTruncated = false;
    deepScanned = 0;
  }

  /** (Re)schedule a deep search for the current query, debounced for typing. */
  function queueDeepSearch() {
    if (!deepSearch || !filter.trim()) {
      resetDeep();
      return;
    }
    if (deepTimer) clearTimeout(deepTimer);
    deepHandle?.cancel();
    deepToken += 1;
    deepRunning = true;
    deepTimer = setTimeout(() => void runDeepSearch(), 450);
  }

  async function runDeepSearch() {
    const token = ++deepToken;
    const q = filter.trim();
    // A cached (or cache-narrowable) result answers without touching the server.
    const cached = searchCache.resolve(root, q);
    if (cached) {
      deepResults = cached.hits;
      deepScanned = cached.scanned;
      deepTruncated = false;
      deepRunning = false;
      return;
    }
    deepResults = [];
    deepRunning = true;
    deepTruncated = false;
    deepScanned = 0;
    deepStopped = false;
    deepHandle = await startSftpSearch(connectionId, root, q, (u) => {
      if (token !== deepToken) return; // superseded by a newer search
      deepResults = [...u.hits];
      deepScanned = u.scanned;
      deepTruncated = u.truncated;
      if (u.done) {
        deepRunning = false;
        if (!deepStopped && !u.truncated) {
          searchCache.store(root, q, { hits: u.hits, scanned: u.scanned, truncated: u.truncated });
        }
      }
    });
  }

  function stopDeepSearch() {
    deepStopped = true;
    deepHandle?.cancel();
  }

  function toggleDeepSearch() {
    deepSearch = !deepSearch;
    queueDeepSearch();
  }

  /** Folder of a deep result, relative to the tree root (for the subtitle). */
  function relParent(e: SftpEntry): string {
    const parent = posixParent(e.path);
    if (parent === root) return "./";
    const base = root === "/" ? "/" : `${root}/`;
    return parent.startsWith(base) ? parent.slice(base.length) : parent;
  }

  /** Expand the tree down to `dir` (loading each level), so a deep-search hit
      can be shown in place. Clears the search to hand the tree back. */
  async function revealDir(dir: string) {
    if (dir !== root && !dir.startsWith(root === "/" ? "/" : `${root}/`)) return;
    const parts = dir.slice(root.length).split("/").filter(Boolean);
    let acc = root === "/" ? "" : root;
    for (const part of parts) {
      acc = `${acc}/${part}`;
      if (children[acc] === undefined) await loadDir(acc);
      expanded = { ...expanded, [acc]: true };
    }
    filter = "";
    resetDeep();
  }

  /** Open a deep-search hit: dirs reveal in the tree; files open (their parent
      revealed so the tree shows where they live). The revealed row is selected
      and scrolled into view, so leaving the results list never loses the hit. */
  async function openDeepHit(e: SftpEntry) {
    if (e.isDir) {
      await revealDir(e.path);
    } else {
      await revealDir(posixParent(e.path));
      if (isArchive(e.name)) extractTo(e);
      else onOpenFile(e.path);
    }
    selected = new Set([e.path]);
    anchorPath = e.path;
    if (e.isDir) onOpenFolder?.(e.path);
    requestAnimationFrame(() => {
      treeEl
        ?.querySelector(`[data-tree-path="${CSS.escape(e.path)}"]`)
        ?.scrollIntoView({ block: "center" });
    });
  }
  function hasMatch(e: SftpEntry): boolean {
    if (!fq) return true;
    if (nameMatches(e)) return true;
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

  // Collapse-all / expand-all toggle (the toolbar chevrons). Expand only spreads
  // across directories already loaded — it never bulk-loads the remote tree.
  const anyExpanded = $derived(Object.values(expanded).some(Boolean));
  function toggleCollapseAll() {
    // A deep search hides the tree behind its flat result list — leave the
    // search first so the collapse/expand visibly does something.
    if (filter) {
      filter = "";
      resetDeep();
    }
    if (anyExpanded) {
      expanded = {};
    } else {
      const next: Record<string, boolean> = {};
      for (const path of Object.keys(children)) if (path !== root) next[path] = true;
      expanded = next;
    }
  }

  // Interactive path: the tree is rooted at the home dir but `root` is movable,
  // so the breadcrumb segments let the user climb above home (up to `/`) the same
  // way the SFTP path does. Each crumb re-roots the tree at that directory.
  const crumbs = $derived.by(() => {
    if (!root || root === "/") return [{ name: "/", path: "/" }];
    const parts = root.split("/").filter(Boolean);
    let acc = "";
    const list = [{ name: "/", path: "/" }];
    for (const p of parts) {
      acc = `${acc}/${p}`;
      list.push({ name: p, path: acc });
    }
    return list;
  });
  async function reroot(path: string) {
    if (path === root) return;
    root = path;
    expanded = {};
    clearSelection();
    // An active deep search re-runs under the new root.
    queueDeepSearch();
    if (children[path] === undefined) await loadDir(path);
  }

  function clickEntry(entry: SftpEntry) {
    if (entry.isDir) void toggleDir(entry);
    // Archives can't open in the editor (binary) — clicking one offers the
    // extract dialog instead of a read error.
    else if (isArchive(entry.name)) extractTo(entry);
    else onOpenFile(entry.path);
  }

  // Remote archive extraction (ssh exec on the host). `extracting` drives the
  // per-row spinner; success/failure both toast from extractArchive.
  let extracting = $state<string | null>(null);

  /** Extract into the archive's own folder, no prompt. */
  async function extractHere(entry: SftpEntry) {
    if (extracting) return;
    const dir = posixParent(entry.path);
    extracting = entry.path;
    try {
      await extractArchive(connectionId, entry.path, dir);
      await refreshAndExpand(dir);
    } catch {
      /* toasted */
    } finally {
      extracting = null;
    }
  }

  /** Prompt for a destination path (created if missing), then extract. */
  function extractTo(entry: SftpEntry) {
    const dir = posixParent(entry.path);
    prompt = {
      title: `Extract "${entry.name}"`,
      value: posixJoin(dir, archiveStem(entry.name)),
      hint: "Runs on the server. The destination folder is created if missing.",
      confirmLabel: "Extract",
      onConfirm: async (dest) => {
        const target = dest.trim();
        if (!target) return;
        // Existing file at the destination → refuse; existing folder →
        // extracting would overwrite matching names inside it, so confirm.
        const ex = await remoteExists(connectionId, target);
        if (ex && !ex.isDir) {
          pushNameTaken(posixBasename(target), posixParent(target));
          return;
        }
        if (ex?.isDir) {
          const choice = await confirmDialog.open({
            title: "Extract into existing folder?",
            message: `“${target}” already exists — files inside it with matching names will be overwritten.`,
            destructive: true,
            icon: "circle-alert",
            actions: [
              { label: "Extract anyway", value: "extract", tone: "destructive", icon: "package" },
            ],
          });
          if (choice !== "extract") return;
        }
        extracting = entry.path;
        try {
          await extractArchive(connectionId, entry.path, target);
          await refreshAndExpand(posixParent(target));
        } finally {
          extracting = null;
        }
      },
    };
  }

  /** Refresh a directory's listing (and the whole visible tree when none
      given): the toolbar refresh re-lists the root plus every expanded
      directory — not just the top level — and drops the cached listings of
      collapsed ones so they re-fetch fresh on their next expand. */
  async function refresh(path = root) {
    searchCache.invalidate(); // a refresh implies the tree may have changed
    if (path === root) {
      const dirs = [root, ...Object.keys(children).filter((p) => p !== root && expanded[p])];
      const keep: Record<string, SftpEntry[]> = {};
      for (const d of dirs) if (children[d] !== undefined) keep[d] = children[d];
      children = keep;
      await Promise.all(dirs.map((d) => loadDir(d)));
    } else if (children[path] !== undefined) {
      await loadDir(path);
    }
  }

  function openMenu(e: MouseEvent, entry: SftpEntry | null) {
    e.preventDefault();
    e.stopPropagation();
    // Right-clicking outside the current selection moves it to that row, so
    // the menu's actions visibly target what's highlighted (VS Code does this).
    if (entry && !selected.has(entry.path)) {
      selected = new Set([entry.path]);
      anchorPath = entry.path;
    }
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
        // Writing an empty file over an existing one would TRUNCATE it.
        if (await remoteExists(connectionId, target)) {
          pushNameTaken(trimmed, dir);
          return;
        }
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
        const target = posixJoin(dir, trimmed);
        if (await remoteExists(connectionId, target)) {
          pushNameTaken(trimmed, dir);
          return;
        }
        await sftpMkdir(connectionId, target);
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
        const target = posixJoin(posixParent(entry.path), trimmed);
        // Renaming onto an existing entry would clobber it — refuse.
        if (await remoteExists(connectionId, target)) {
          pushNameTaken(trimmed, posixParent(entry.path));
          return;
        }
        await sftpRename(connectionId, entry.path, target);
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
    const targets = confirmDelete;
    confirmDelete = null;
    if (!targets || targets.length === 0) return;
    // Deepest paths first, so a selected folder whose selected children empty
    // it can still be removed in the same pass.
    const ordered = [...targets].sort(
      (a, b) => b.path.split("/").length - a.path.split("/").length,
    );
    const parents = new Set<string>();
    for (const e of ordered) {
      try {
        if (e.isDir) await sftpRemoveDir(connectionId, e.path);
        else await sftpRemoveFile(connectionId, e.path);
        // Keep an open deep-result list in sync with the deletion.
        if (deepResults) deepResults = deepResults.filter((x) => x.path !== e.path);
        parents.add(posixParent(e.path));
      } catch {
        /* toasted */
      }
    }
    clearSelection();
    for (const p of parents) await refresh(p);
  }

  async function download(entry: SftpEntry) {
    // Host-side save dialog: the backend runs it, canonicalizes the result,
    // and inserts the path into the approved set — a frontend dialog's path
    // would be rejected by `ensure_local_path_approved`.
    const dest = await safeInvoke<string | null>("sftp_pick_save_path", {
      defaultName: entry.name,
    });
    if (!dest) return;
    sftpTransfers.enqueueDownload(connectionId, entry.path, dest, entry.name);
  }

  async function uploadInto(entry: SftpEntry | null) {
    const dir = dirOf(entry);
    // Host-side multi-file picker — paths land in the approved set before the
    // webview ever sees them, so the transfers below pass the approval check.
    const paths = await safeInvoke<string[]>("sftp_pick_upload_files");
    if (paths.length === 0) return;
    await uploadPaths(paths, dir);
  }

  async function uploadFolderInto(entry: SftpEntry | null) {
    const dir = dirOf(entry);
    // Host-side folder picker; the chosen directory (and so its whole subtree)
    // is approved for reading before any transfer starts.
    const picked = await safeInvoke<string | null>("sftp_pick_upload_dir");
    if (!picked) return;
    await uploadPaths([picked], dir);
  }

  /** Upload files/folders into a remote dir: plan → replace-confirm → queue. */
  async function uploadPaths(localPaths: string[], dir: string) {
    try {
      await uploadWithConfirm(connectionId, localPaths, dir, () => void refreshAndExpand(dir));
    } catch {
      /* localStat / walk failures are toasted by safeInvoke */
    }
  }

  /** Copy a remote path to the clipboard — paste it into the agent's working-dir
      field, or anywhere a host path is wanted. Silent if clipboard is denied. */
  async function copyPath(path: string) {
    try {
      await navigator.clipboard.writeText(path);
    } catch {
      /* no clipboard permission — silently no-op */
    }
  }

  /** Reload a dir's listing and ensure it's expanded so new items show. */
  async function refreshAndExpand(dir: string) {
    searchCache.invalidate(); // only called after mutations (mkdir/upload/…)
    if (dir !== root) expanded = { ...expanded, [dir]: true };
    await loadDir(dir);
  }

  function iconFor(e: SftpEntry, open = false): IconName {
    if (e.isDir) return open ? "folder-open" : "folder";
    if (isArchive(e.name)) return "archive";
    if (/\.(png|jpe?g|gif|webp|bmp|ico|avif|svg)$/i.test(e.name)) return "image";
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

  // NOTE: the sidebar tree no longer accepts OS file drops — with the Files
  // tab open in the editor area, dropping into the folder you're LOOKING AT
  // (FileBrowserPane's drop zone) is the unambiguous target; a skinny tree
  // row under the cursor was too easy to mis-drop. Uploads from the tree stay
  // available via the context menu. `treeEl` remains for scroll-into-view.
  let treeEl = $state<HTMLDivElement | null>(null);

  // This connection's transfers, surfaced as a slim strip under the tree so
  // uploads/downloads started here show progress — and failures — without
  // having to open the SFTP tab's transfer popover.
  const myTransfers = $derived(sftpTransfers.value.filter((t) => t.connectionId === connectionId));
  const activeTransfers = $derived(
    myTransfers.filter((t) => t.status === "active" || t.status === "pending"),
  );
  const failedTransfers = $derived(myTransfers.filter((t) => t.status === "error"));
  const activePct = $derived.by(() => {
    const total = activeTransfers.reduce((n, t) => n + t.total, 0);
    if (total <= 0) return null;
    const moved = activeTransfers.reduce((n, t) => n + t.transferred, 0);
    return Math.min(100, Math.round((moved / total) * 100));
  });
  function dismissFailed() {
    for (const t of failedTransfers) sftpTransfers.remove(t.id);
  }
  function retryFailed() {
    for (const t of failedTransfers) sftpTransfers.retry(t.id);
  }

  // Refs for the two inline dialog elements so the Tab trap can query them.
  let promptDialogEl = $state<HTMLDivElement | null>(null);
  let deleteDialogEl = $state<HTMLDivElement | null>(null);

  /** Trap Tab focus within whichever inline dialog is open. */
  function trapTab(e: KeyboardEvent, el: HTMLDivElement | null) {
    if (e.key !== "Tab" || !el) return;
    const focusables = el.querySelectorAll<HTMLElement>(
      'button:not([disabled]), input, [tabindex]:not([tabindex="-1"])',
    );
    if (!focusables || focusables.length === 0) return;
    const first = focusables[0];
    const last = focusables[focusables.length - 1];
    const active = document.activeElement;
    if (e.shiftKey && active === first) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && active === last) {
      e.preventDefault();
      first.focus();
    }
  }
</script>

<svelte:window
  onclick={() => (menu = null)}
  onkeydown={(e) => {
    if (e.key === "Escape") {
      if (prompt) { prompt = null; return; }
      if (confirmDelete) { confirmDelete = null; return; }
      if (menu) { menu = null; return; }
      if (selected.size > 0) clearSelection();
      return;
    }
    if (prompt) trapTab(e, promptDialogEl);
    else if (confirmDelete) trapTab(e, deleteDialogEl);
    else if (
      // Delete (or ⌘/Ctrl+Backspace) deletes the selection — never while a
      // modal is up or focus sits in an editable (editor/terminal/inputs).
      (e.key === "Delete" || (e.key === "Backspace" && (e.metaKey || e.ctrlKey))) &&
      selected.size > 0 &&
      !menu
    ) {
      const t = e.target as HTMLElement | null;
      if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable)) return;
      e.preventDefault();
      const targets = [...selected].map(entryByPath).filter((x): x is SftpEntry => !!x);
      if (targets.length > 0) confirmDelete = targets;
    }
  }}
/>

<div class="flex h-full min-h-0 flex-col">
  <!-- Toolbar -->
  <div class="flex items-center gap-1 border-b border-border/50 px-2 py-1.5">
    <!-- Interactive path: click any segment to re-root the tree there. -->
    <nav class="flex min-w-0 flex-1 items-center gap-0.5 overflow-x-auto font-mono text-[11px]">
      {#if !root}
        <span class="text-fg-subtle">…</span>
      {/if}
      {#each crumbs as crumb, i (crumb.path)}
        {#if i > 0}
          <Icon name="chevron-right" size={10} class="shrink-0 text-fg-subtle" />
        {/if}
        <button
          type="button"
          onclick={() => reroot(crumb.path)}
          disabled={crumb.path === root}
          class="shrink-0 rounded px-1 py-0.5 text-fg-subtle hover:bg-surface-2 hover:text-fg disabled:text-fg disabled:hover:bg-transparent"
          title={crumb.path}
        >
          {crumb.name === "/" ? "root" : crumb.name}
        </button>
      {/each}
    </nav>
    <button
      type="button"
      onclick={toggleCollapseAll}
      disabled={!root}
      class="grid h-6 w-6 place-items-center rounded text-fg-muted hover:bg-surface-2 hover:text-fg disabled:opacity-40"
      title={anyExpanded ? "Collapse all folders" : "Expand loaded folders"}
      aria-label={anyExpanded ? "Collapse all folders" : "Expand loaded folders"}
    >
      <Icon name={anyExpanded ? "chevrons-down-up" : "chevrons-up-down"} size={13} />
    </button>
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

  <!-- Filter: live match over the loaded tree, or — with the layers toggle —
       a recursive server-side search of every subfolder under the root. -->
  <div class="flex items-center gap-1 border-b border-border/50 px-2 py-1.5">
    <div class="relative min-w-0 flex-1">
      <span class="absolute left-1.5 top-1/2 -translate-y-1/2 text-fg-subtle">
        <Icon name="search" size={12} />
      </span>
      <input
        bind:value={filter}
        oninput={queueDeepSearch}
        placeholder={deepSearch ? "Search subfolders… (*.zip works)" : "Filter files…"}
        spellcheck="false"
        class="w-full rounded border border-border bg-surface pl-7 pr-6 text-[11.5px] text-fg
               placeholder:text-fg-subtle focus:border-accent/60 focus:outline-none"
        style="height: 26px"
      />
      {#if filter}
        <button
          type="button"
          onclick={() => { filter = ""; resetDeep(); }}
          class="absolute right-1.5 top-1/2 -translate-y-1/2 rounded p-0.5 text-fg-subtle hover:text-fg"
          aria-label="Clear filter"
        >
          <Icon name="x" size={12} />
        </button>
      {/if}
    </div>
    <button
      type="button"
      onclick={toggleDeepSearch}
      disabled={!root}
      aria-pressed={deepSearch}
      class="grid h-[26px] w-7 shrink-0 place-items-center rounded {deepSearch ? 'bg-accent/15 text-accent' : 'text-fg-muted hover:bg-surface-2 hover:text-fg'} disabled:opacity-40"
      title={deepSearch ? "Searching subfolders too — click for loaded tree only" : "Search subfolders too (deep search)"}
    >
      <Icon name="layers" size={13} />
    </button>
  </div>

  <!-- Tree -->
  <div
    bind:this={treeEl}
    class="min-h-0 flex-1 overflow-auto py-1"
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
    {:else if deepSearch && fq && deepResults !== null}
      <!-- Deep search results: every match under the root, streamed in as the
           server walk progresses. Clicking reveals the hit in the tree. -->
      <div class="sticky top-0 z-10 flex items-center gap-2 border-b border-border/60 bg-surface px-2 py-1 text-[11px] text-fg-subtle">
        {#if deepRunning}<Icon name="refresh-cw" size={11} class="shrink-0 animate-spin" />{/if}
        <span class="truncate">
          {deepResults.length} result{deepResults.length === 1 ? "" : "s"} · {deepScanned.toLocaleString()} scanned
          {#if deepTruncated}· stopped at limit{/if}
        </span>
        {#if deepRunning}
          <button
            type="button"
            onclick={stopDeepSearch}
            class="ml-auto shrink-0 rounded px-1.5 py-0.5 text-fg-muted hover:bg-surface-2 hover:text-fg"
          >
            Stop
          </button>
        {/if}
      </div>
      {#if deepResults.length === 0 && !deepRunning}
        <p class="p-4 text-center text-[12px] text-fg-subtle">No matches for “{filter}”.</p>
      {:else}
        {#each deepResults as e (e.path)}
          <button
            type="button"
            onclick={() => void openDeepHit(e)}
            oncontextmenu={(ev) => openMenu(ev, e)}
            class="flex w-full items-center gap-1.5 px-2 py-1 text-left hover:bg-surface-2/60"
            title={e.path}
          >
            <Icon name={iconFor(e)} size={14} class={e.isDir ? "shrink-0 text-accent" : "shrink-0 text-fg-subtle"} />
            <span class="min-w-0 flex-1 truncate">
              <span class="text-[12.5px] text-fg-muted">{e.name}</span>
              <span class="ml-1.5 font-mono text-[10.5px] text-fg-subtle">{relParent(e)}</span>
            </span>
          </button>
        {/each}
      {/if}
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

  <!-- Transfer status: progress for queue jobs started from this tree. -->
  {#if activeTransfers.length > 0 || failedTransfers.length > 0}
    <div class="border-t border-border/50 px-2 py-1 text-[11px]">
      {#if activeTransfers.length > 0}
        <div class="flex items-center gap-1.5 text-fg-muted">
          <Icon name="refresh-cw" size={11} class="shrink-0 animate-spin" />
          <span class="min-w-0 flex-1 truncate">
            {activeTransfers.length === 1
              ? `Transferring ${activeTransfers[0].name}…`
              : `${activeTransfers.length} transfers…`}
          </span>
          {#if activePct !== null}
            <span class="shrink-0 tabular-nums text-fg-subtle">{activePct}%</span>
          {/if}
        </div>
      {/if}
      {#if failedTransfers.length > 0}
        <div class="flex items-center gap-1.5 text-status-crashed">
          <span class="min-w-0 flex-1 truncate" title={failedTransfers[0].error}>
            {failedTransfers.length} transfer{failedTransfers.length === 1 ? "" : "s"} failed
            {#if failedTransfers[0].error}— {failedTransfers[0].error}{/if}
          </span>
          <button type="button" onclick={retryFailed} class="shrink-0 rounded px-1 hover:bg-status-crashed/10" title="Retry failed transfers">
            Retry
          </button>
          <button type="button" onclick={dismissFailed} class="shrink-0 rounded px-1 text-fg-subtle hover:bg-surface-2 hover:text-fg" title="Dismiss">
            <Icon name="x" size={11} />
          </button>
        </div>
      {/if}
    </div>
  {/if}
</div>

<!-- A tree row, recursing into expanded directories. Hidden when a filter is
     active and neither it nor any loaded descendant matches. Filtering never
     changes expand state — folders stay exactly as the user left them.
     `forced` means an ancestor matched by name, so this whole subtree is shown
     regardless of the query (when its folders are expanded). -->
{#snippet node(entry: SftpEntry, depth: number, forced = false)}
  {#if forced || hasMatch(entry)}
    {@const open = !!expanded[entry.path]}
    <div>
      <button
        type="button"
        data-tree-path={entry.path}
        data-tree-dir={entry.isDir}
        onclick={(e) => rowClick(e, entry)}
        oncontextmenu={(e) => openMenu(e, entry)}
        aria-current={activePath === entry.path ? "true" : undefined}
        class="group flex w-full items-center gap-1 py-0.5 pr-2 text-left text-[12.5px] hover:bg-surface-2/60
          {selected.has(entry.path) || activePath === entry.path ? 'bg-accent/10 text-fg' : 'text-fg-muted'}"
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
        {#if extracting === entry.path}
          <Icon name="refresh-cw" size={14} class="shrink-0 animate-spin text-fg-subtle" />
        {:else}
          <Icon name={iconFor(entry, open)} size={14} class={entry.isDir ? "shrink-0 text-accent" : "shrink-0 text-fg-subtle"} />
        {/if}
        <span class="truncate">{entry.name}{entry.isSymlink ? " ↗" : ""}</span>
      </button>
      {#if entry.isDir && open}
        {#if loadingDir[entry.path] && children[entry.path] === undefined}
          <p class="py-0.5 text-[11px] text-fg-subtle" style="padding-left: {(depth + 1) * 12 + 24}px">Loading…</p>
        {:else}
          {#each children[entry.path] ?? [] as child (child.path)}
            {@render node(child, depth + 1, forced || nameMatches(entry))}
          {/each}
          {#if (!fq || forced || nameMatches(entry)) && (children[entry.path] ?? []).length === 0}
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
  {#key menu}
  <div
    use:clampToViewport
    class="fixed z-50 w-44 rounded-lg border border-border bg-surface p-1 shadow-xl"
    style="left: {x}px; top: {y}px"
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
      <Icon name="share" size={13} /> Upload files here
    </button>
    <button type="button" role="menuitem" onclick={() => { const en = entry; menu = null; uploadFolderInto(en); }} class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg">
      <Icon name="folder-plus" size={13} /> Upload folder here
    </button>
    <div class="my-1 border-t border-border/60"></div>
    {#if onOpenAgentHere}
      <button type="button" role="menuitem" onclick={() => { const dir = dirOf(entry); menu = null; onOpenAgentHere?.(dir); }} class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg">
        <Icon name="bot" size={13} /> Open agent here
      </button>
    {/if}
    <button type="button" role="menuitem" onclick={() => { const p = entry?.path ?? root; menu = null; void copyPath(p); }} class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg">
      <Icon name="copy" size={13} /> Copy path
    </button>
    {#if entry}
      <div class="my-1 border-t border-border/60"></div>
      {#if !entry.isDir}
        <button type="button" role="menuitem" onclick={() => { const en = entry; if (!en) return; menu = null; void download(en); }} class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg">
          <Icon name="arrow-up" size={13} class="rotate-180" /> Download
        </button>
      {/if}
      {#if !entry.isDir && isArchive(entry.name)}
        <button type="button" role="menuitem" onclick={() => { const en = entry; if (!en) return; menu = null; void extractHere(en); }} class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg">
          <Icon name="package" size={13} /> Extract here
        </button>
        <button type="button" role="menuitem" onclick={() => { const en = entry; if (!en) return; menu = null; extractTo(en); }} class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg">
          <Icon name="archive" size={13} /> Extract to…
        </button>
      {/if}
      <button type="button" role="menuitem" onclick={() => { const en = entry; if (!en) return; menu = null; renameEntry(en); }} class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg">
        <Icon name="pencil" size={13} /> Rename
      </button>
      <button type="button" role="menuitem" onclick={() => { const en = entry; if (!en) return; menu = null; chmodEntry(en); }} class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg">
        <Icon name="lock" size={13} /> Permissions
      </button>
      <button type="button" role="menuitem" onclick={() => { const en = entry; if (!en) return; menu = null; confirmDelete = deleteTargets(en); }} class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-status-crashed hover:bg-status-crashed/10">
        <Icon name="trash-2" size={13} />
        Delete{selected.has(entry.path) && selected.size > 1 ? ` ${selected.size} items` : ""}
      </button>
    {/if}
  </div>
  {/key}
{/if}

<!-- Text-prompt modal (new file/folder / rename / chmod) -->
{#if prompt}
  {@const p = prompt}
  <div
    class="fixed inset-0 z-[60] flex items-center justify-center bg-black/40 p-4"
    role="presentation"
    onclick={(e) => { if (e.target === e.currentTarget) prompt = null; }}
  >
    <div bind:this={promptDialogEl} class="w-full max-w-sm rounded-xl border border-border bg-surface p-4 shadow-2xl" role="dialog" aria-modal="true">
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
    <div bind:this={deleteDialogEl} class="w-full max-w-sm rounded-xl border border-border bg-surface p-4 shadow-2xl" role="dialog" aria-modal="true">
      <h3 class="text-[13px] font-semibold text-fg">
        Delete {d.length === 1 ? `"${d[0].name}"` : `${d.length} items`}?
      </h3>
      <p class="mt-1.5 text-[12px] text-fg-muted">
        {d.some((x) => x.isDir) ? "Folders must be empty. " : ""}This can't be undone.
      </p>
      {#if d.length > 1}
        <ul class="mt-2 max-h-32 overflow-y-auto rounded-md border border-border/60 bg-surface-2/40 px-2.5 py-1.5 font-mono text-[11px] text-fg-subtle">
          {#each d as t (t.path)}
            <li class="truncate" title={t.path}>{t.name}</li>
          {/each}
        </ul>
      {/if}
      <div class="mt-3 flex justify-end gap-2">
        <button type="button" onclick={() => (confirmDelete = null)} class="h-8 rounded-md px-3 text-[12px] text-fg-muted hover:bg-surface-2">Cancel</button>
        <button type="button" onclick={doDelete} class="h-8 rounded-md bg-status-crashed px-3 text-[12px] font-medium text-white hover:brightness-110">Delete</button>
      </div>
    </div>
  </div>
{/if}
