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

  import { clampToViewport } from "$lib/actions/clampToViewport";
  import Icon from "$lib/components/atoms/Icon.svelte";
  import { sortEntries, type SortDir, type SortKey } from "$lib/entrySort";
  import LocalFilePane from "$lib/components/connections/LocalFilePane.svelte";
  import SftpFolderIcon from "$lib/components/connections/SftpFolderIcon.svelte";
  import SftpInspectorPane from "$lib/components/connections/SftpInspectorPane.svelte";
  import { invokeQuiet, safeInvoke } from "$lib/ipc";
  import {
    EMPTY_SELECTION,
    plainSelect,
    pruneSelection,
    rangeSelect,
    toggleSelect as selectionToggle,
    toggleSelectAll as selectionToggleAll,
    type Selection,
  } from "$lib/listSelection";
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
    posixBasename,
    formatMode,
    formatMtime,
    formatSize,
    type SftpPreview,
  } from "$lib/sftp";
  import { isArchive, archiveStem, compressEntries, extractArchive } from "$lib/sftpArchive";
  import { ensureLocalAccess } from "$lib/sftpLocalAccess";
  import { remoteExists, pushNameTaken } from "$lib/sftpGuards";
  import { planMoves, splitMoveConflicts, type MovePlan } from "$lib/sftpMove";
  import { startSftpSearch, SftpSearchCache, type RunningSearch } from "$lib/sftpSearch";
  import { uploadWithConfirm, LOCAL_DRAG_MIME } from "$lib/sftpUploadFlow";
  import type { SftpEntry } from "$lib/types/sshTunnels";

  let {
    connectionId,
    label,
    onClose,
    onOpenFile,
    onOpenFolder,
    navigateRequest = null,
    variant = "full",
  }: {
    connectionId: string;
    label: string;
    onClose?: () => void;
    /**
     * When provided (the IDE workspace), text files open in the host's shared
     * editor area (right-hand CodeMirror) instead of this pane's inline modal —
     * matching the Explorer. Omitted by the standalone modal, which keeps its
     * own edit-and-push dialog.
     */
    onOpenFile?: (path: string) => void;
    /**
     * When provided (the workspace sidebar), a plain click on a folder opens
     * it in the editor area's Files tab — the Finder-style browser on the
     * right — instead of only selecting it here.
     */
    onOpenFolder?: (path: string) => void;
    /** External "navigate here" request (the Files tab being pointed at a
        folder). The nonce makes the same path re-applyable. */
    navigateRequest?: { path: string; nonce: number } | null;
    /**
     * `sidebar` is the skinny workspace-rail embedding: list view only (no
     * icon grid) and no toolbar — new folder, the view switcher, the Local
     * split and both upload buttons all live in the right-hand Files tab
     * (the `full` variant), which is where folders open anyway. Context-menu
     * and drag-drop uploads still work in the sidebar.
     */
    variant?: "full" | "sidebar";
  } = $props();

  let cwd = $state<string>("");
  let entries = $state<SftpEntry[]>([]);
  let loading = $state(false);
  let navError = $state<string | null>(null);

  // Finder-style view prefs — list vs icon grid, plus the sort column. A
  // per-machine preference (like ideLayout), so the browser opens the way
  // you left it.
  const VIEW_PREF_KEY = "portbay.sftp.view";
  type ViewMode = "list" | "icons";
  function loadViewPref(): { mode: ViewMode; sortKey: SortKey; sortDir: SortDir } {
    const fallback = { mode: "list" as ViewMode, sortKey: "name" as SortKey, sortDir: "asc" as SortDir };
    if (typeof localStorage === "undefined") return fallback;
    try {
      const raw = localStorage.getItem(VIEW_PREF_KEY);
      if (!raw) return fallback;
      const p = JSON.parse(raw) as Partial<{ mode: string; sortKey: string; sortDir: string }>;
      return {
        mode: p.mode === "icons" ? "icons" : "list",
        sortKey: p.sortKey === "size" || p.sortKey === "mtime" ? p.sortKey : "name",
        sortDir: p.sortDir === "desc" ? "desc" : "asc",
      };
    } catch {
      return fallback;
    }
  }
  const initialView = loadViewPref();
  let viewMode = $state<ViewMode>(initialView.mode);
  let sortKey = $state<SortKey>(initialView.sortKey);
  let sortDir = $state<SortDir>(initialView.sortDir);
  function saveViewPref() {
    if (typeof localStorage === "undefined") return;
    try {
      localStorage.setItem(VIEW_PREF_KEY, JSON.stringify({ mode: viewMode, sortKey, sortDir }));
    } catch {
      /* storage full / disabled — the pref just won't stick */
    }
  }
  function setViewMode(mode: ViewMode) {
    viewMode = mode;
    saveViewPref();
  }
  /** Header click: the same key flips direction, a new key starts ascending. */
  function setSort(key: SortKey) {
    if (sortKey === key) sortDir = sortDir === "asc" ? "desc" : "asc";
    else {
      sortKey = key;
      sortDir = "asc";
    }
    saveViewPref();
  }

  // Live name filter over the current directory's listing, in display order.
  let query = $state("");
  const visibleEntries = $derived.by(() => {
    const q = query.trim().toLowerCase();
    const base = q ? entries.filter((e) => e.name.toLowerCase().includes(q)) : entries;
    return sortEntries(base, sortKey, sortDir);
  });

  // Deep (recursive) server search — the "find files" mode of desktop SFTP
  // clients. ON by default so a search reaches into subfolders without opening
  // them first; the layers toggle drops back to this-folder-only filtering.
  // The backend walks subfolders and streams hits in, supporting substring or
  // glob (`*.zip`) queries.
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
    if (!deepSearch || !query.trim()) {
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
    const q = query.trim();
    // A cached (or cache-narrowable) result answers without touching the server.
    const cached = searchCache.resolve(cwd, q);
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
    const root = cwd;
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

  /** Folder of a deep result, relative to the search root (for the subtitle). */
  function relParent(e: SftpEntry): string {
    const parent = posixParent(e.path);
    if (parent === cwd) return "./";
    const base = cwd === "/" ? "/" : `${cwd}/`;
    return parent.startsWith(base) ? parent.slice(base.length) : parent;
  }

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
  /** "4m 12s" / "48s" — compact ETA for the active-transfer readout. */
  function formatEta(secs: number | null): string {
    if (secs == null || !isFinite(secs) || secs <= 0) return "";
    const s = Math.round(secs);
    if (s < 60) return `${s}s`;
    const m = Math.floor(s / 60);
    if (m < 60) return `${m}m ${s % 60}s`;
    const h = Math.floor(m / 60);
    return `${h}h ${m % 60}m`;
  }

  // Multi-select for batch download / delete, keyed by remote path.
  let selected = $state<Set<string>>(new Set());
  let batchBusy = $state(false);
  const selectedCount = $derived(selected.size);
  // The single highlighted entry, when exactly one is selected — the target for
  // the footer's Rename/Copy-path actions (which act on one item).
  const selectedEntry = $derived.by<SftpEntry | null>(() => {
    if (selected.size !== 1) return null;
    const [path] = [...selected];
    return entries.find((e) => e.path === path) ?? null;
  });
  const allVisibleSelected = $derived.by(
    () => visibleEntries.length > 0 && visibleEntries.every((e) => selected.has(e.path)),
  );
  // Where local-pane uploads land: checking exactly one remote *folder* targets
  // it; otherwise the current directory. Surfaced on the local pane's Upload
  // button so the destination is never ambiguous.
  const uploadDest = $derived.by(() => {
    if (selected.size === 1) {
      const [path] = [...selected];
      const e = entries.find((x) => x.path === path);
      if (e?.isDir) return e.path;
    }
    return cwd;
  });
  // Selection semantics live in $lib/listSelection (pure + unit-tested);
  // these adapters apply its results to the pane's rune state.
  const currentSel = (): Selection => ({ paths: selected, anchor: anchorPath });
  function applySel(s: Selection) {
    selected = new Set(s.paths);
    anchorPath = s.anchor;
  }
  const visiblePaths = () => visibleEntries.map((e) => e.path);

  function toggleSelectAll() {
    applySel(selectionToggleAll(currentSel(), visiblePaths()));
  }
  function clearSelection() {
    applySel(EMPTY_SELECTION);
  }

  // Right-side inspector (Finder-style): a plain click selects a row and shows
  // it here — folder contents you can drill into, or a file preview, plus a
  // Get-Info block. ✕ hides the panel until the next click. Driven by the
  // single selection, so checkbox-selecting exactly one row inspects it too.
  let inspectorOpen = $state(true);
  // The last plain-clicked entry, kept separately so deep-search hits (whose
  // paths live outside `entries`) can be inspected as well.
  let inspected = $state<SftpEntry | null>(null);
  const inspectorEntry = $derived.by<SftpEntry | null>(() => {
    if (!inspectorOpen || selected.size !== 1) return null;
    const [path] = [...selected];
    return entries.find((e) => e.path === path) ?? (inspected?.path === path ? inspected : null);
  });

  // Finder-style click selection over the listing: plain click selects the row,
  // ⇧-click extends from the last-clicked row (the anchor), ⌘/Ctrl-click
  // toggles. The checkbox column shares the same anchor so ⇧-checking ranges too.
  let anchorPath = $state<string | null>(null);

  /** Row click: ⇧ extends, ⌘/Ctrl toggles, plain click selects just this row
      (and opens it in the inspector — Finder-style; double-click opens). */
  function rowClick(ev: MouseEvent, e: SftpEntry) {
    if (ev.shiftKey) {
      applySel(rangeSelect(currentSel(), visiblePaths(), e.path, ev.metaKey || ev.ctrlKey));
    } else if (ev.metaKey || ev.ctrlKey) {
      applySel(selectionToggle(currentSel(), e.path));
      inspected = e;
    } else {
      applySel(plainSelect(e.path));
      inspected = e;
      inspectorOpen = true;
      if (e.isDir) onOpenFolder?.(e.path);
    }
  }

  /** Deep-search hit click: select + inspect, like a listing row. */
  function deepClick(e: SftpEntry) {
    applySel(plainSelect(e.path));
    inspected = e;
    inspectorOpen = true;
    if (e.isDir) onOpenFolder?.(e.path);
  }

  // Apply external navigate requests once connected (`cwd` set). The effect
  // also tracks `cwd`, so a request that arrives during connect-on-mount is
  // applied right after the initial home navigation lands instead of racing it.
  let lastNavNonce = -1;
  $effect(() => {
    const req = navigateRequest;
    if (!req || req.nonce === lastNavNonce || !cwd) return;
    lastNavNonce = req.nonce;
    if (req.path !== cwd) void navigate(req.path);
  });

  /** Checkbox click: ⇧ extends additively, otherwise toggle + move the anchor.
      The checkboxes are state-driven buttons (no native checked state), so the
      rendered tick always matches `selected` — mixing row clicks and checkbox
      clicks can't desync them. */
  function checkboxClick(ev: MouseEvent, e: SftpEntry) {
    ev.stopPropagation();
    if (ev.shiftKey) {
      applySel(rangeSelect(currentSel(), visiblePaths(), e.path, true));
    } else {
      applySel(selectionToggle(currentSel(), e.path));
    }
  }

  // OS drag-and-drop upload, scoped to this pane's bounding box.
  let paneEl = $state<HTMLDivElement | null>(null);
  let dragOver = $state(false);

  async function uploadPaths(localPaths: string[], intoDir = cwd) {
    const dir = intoDir;
    if (localPaths.length === 0) return;
    // The shared pipeline plans (folders walk recursively, remote dirs are
    // recreated), checks the live remote listing for same-name clashes and asks
    // Replace / Skip existing / Cancel, then runs everything on the queue.
    try {
      await uploadWithConfirm(connectionId, localPaths, dir, () => {
        if (cwd === dir || dir.startsWith(`${cwd}/`)) refresh();
      });
    } catch {
      /* localStat / walk failures are toasted by safeInvoke */
    }
  }

  // In-app drag-and-drop from the local pane: remote folder rows (and the
  // listing background, meaning the cwd) accept drops of local paths. The
  // hovered target highlights so it's unambiguous where files will land.
  let dropTarget = $state<string | null>(null);
  function hasLocalDrag(ev: DragEvent): boolean {
    return !!ev.dataTransfer && Array.from(ev.dataTransfer.types).includes(LOCAL_DRAG_MIME);
  }

  // NOTE: the listing's own rows/cards are deliberately NOT drag sources —
  // dragging files around a live server is too easy to fat-finger. Moving is
  // the explicit, guarded "Move…" menu action below (runMoves).
  async function runMoves(plan: MovePlan[]) {
    if (plan.length === 0) return;
    // Probe every destination first — landing on an existing name must be an
    // explicit Replace / Skip decision, never a silent overwrite.
    const existing = new Map<string, SftpEntry>();
    for (const m of plan) {
      const hit = await remoteExists(connectionId, m.to);
      if (hit) existing.set(m.to, hit);
    }
    let toRun = plan;
    if (existing.size > 0) {
      const { clean, conflicted } = splitMoveConflicts(plan, new Set(existing.keys()));
      const choice = await confirmDialog.open({
        title:
          conflicted.length === 1
            ? `“${posixBasename(conflicted[0].to)}” already exists at the destination`
            : `${conflicted.length} of ${plan.length} items already exist at the destination`,
        message:
          "Replace deletes the existing file(s) first, then moves. Existing folders are never replaced — those moves are skipped.",
        destructive: true,
        icon: "circle-alert",
        actions: [
          { label: "Replace", value: "replace", tone: "destructive", icon: "trash-2" },
          { label: "Skip existing", value: "skip" },
        ],
      });
      if (choice === "replace") {
        const runnable = [...clean];
        for (const m of conflicted) {
          const ex = existing.get(m.to);
          // Never delete a folder to make room — that's a recursive delete
          // the user didn't ask for. Those moves are skipped.
          if (!ex || ex.isDir) continue;
          try {
            await sftpRemoveFile(connectionId, m.to);
            runnable.push(m);
          } catch {
            /* toasted — that move is skipped */
          }
        }
        toRun = runnable;
      } else if (choice === "skip") {
        toRun = clean;
      } else {
        return; // cancelled — nothing moved
      }
      if (toRun.length === 0) {
        refresh();
        return;
      }
    }
    for (const m of toRun) {
      try {
        await sftpRename(connectionId, m.from, m.to);
      } catch {
        /* toasted (permission denied etc.) */
      }
    }
    clearSelection();
    refresh();
  }

  function rowDragOver(ev: DragEvent, e: SftpEntry) {
    if (!e.isDir || !hasLocalDrag(ev)) return;
    ev.preventDefault();
    ev.stopPropagation();
    if (ev.dataTransfer) ev.dataTransfer.dropEffect = "copy";
    dropTarget = e.path;
  }
  function listDragOver(ev: DragEvent) {
    if (!hasLocalDrag(ev)) return;
    ev.preventDefault();
    if (ev.dataTransfer) ev.dataTransfer.dropEffect = "copy";
    dropTarget = cwd;
  }
  function localDrop(ev: DragEvent, dir: string) {
    if (!hasLocalDrag(ev)) return;
    ev.preventDefault();
    ev.stopPropagation();
    dropTarget = null;
    const raw = ev.dataTransfer?.getData(LOCAL_DRAG_MIME);
    if (!raw) return;
    try {
      const paths = JSON.parse(raw) as string[];
      if (Array.isArray(paths) && paths.length > 0) void droppedUpload(paths, dir);
    } catch {
      /* malformed payload — ignore */
    }
  }
  async function droppedUpload(paths: string[], dir: string) {
    // Dragged paths are renderer-named (no OS picker), so they need the
    // host-confirmed access grant before the backend will read them.
    try {
      if (!(await ensureLocalAccess(paths, label))) return;
    } catch {
      return; /* toasted */
    }
    await uploadPaths(paths, dir);
  }

  async function batchDownload() {
    const files = [...selected]
      .map((p) => entries.find((e) => e.path === p))
      .filter((e): e is SftpEntry => !!e && !e.isDir);
    if (files.length === 0) return;
    // Host-side picker: the backend runs the dialog, canonicalizes the result,
    // and inserts it into the approved set before returning it to the renderer.
    const dir = await safeInvoke<string | null>("sftp_pick_download_dir");
    if (!dir) return;
    for (const f of files) {
      sftpTransfers.enqueueDownload(connectionId, f.path, `${dir}/${f.name}`, f.name);
    }
    clearSelection();
  }

  async function batchDelete() {
    if (batchBusy) return;
    const targets = [...selected]
      .map((p) => entries.find((e) => e.path === p))
      .filter((e): e is SftpEntry => !!e);
    if (targets.length === 0) return;
    // Busy from the confirm onward so a repeated trigger (footer button, Delete
    // key) can't stack a second confirm dialog.
    batchBusy = true;
    try {
      const choice = await confirmDialog.open({
        title: `Delete ${targets.length} item${targets.length === 1 ? "" : "s"}?`,
        message: "Folders must be empty. This can't be undone.",
        destructive: true,
        icon: "trash-2",
        actions: [{ label: "Delete", value: "delete", tone: "destructive", icon: "trash-2" }],
      });
      if (choice !== "delete") return;
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

  // Keep in step with languageFor() in $lib/ide/codemirror.ts — anything the
  // editor can highlight should open as text here rather than fall to preview.
  const TEXT_EXT =
    /\.(txt|text|md|markdown|json|jsonc|json5|jsonl|ndjson|webmanifest|ya?ml|toml|ini|conf|cfg|env|properties|htaccess|service|socket|timer|target|desktop|git(ignore|attributes|modules|config)|npmrc|yarnrc|editorconfig|sh|bash|zsh|ksh|fish|ps1|psm1|psd1|js|mjs|cjs|ts|mts|cts|jsx|tsx|css|scss|sass|less|styl|html?|xhtml|svelte|vue|astro|ejs|erb|hbs|mustache|pug|jade|j2|jinja2?|njk|twig|xml|xsl(t)?|xsd|svg|plist|rs|py|pyw|rb|php|go|java|kt|kts|scala|sbt|gradle|groovy|cs|csx|dart|swift|m|mm|c|h|cc|cxx|cpp|hpp|hh|pl|pm|r|jl|hs|erl|hrl|ex|exs|clj|cljs|cljc|edn|lisp|cl|el|scm|ss|rkt|ml|mli|fs|fsi|fsx|proto|cmake|mk|coffee|elm|cr|d|tcl|lua|pp|hcl|tf|tfvars|http|rest|vb|sql|tex|sty|bib|log|diff|patch|dockerfile)$/i;

  const isTextFile = (e: SftpEntry) => {
    if (e.isDir) return false;
    if (TEXT_EXT.test(e.name)) return true;
    // Treat extensionless files (Makefile, LICENSE) and dotfiles (.htaccess,
    // .bashrc, .env) as text/config: strip a single leading dot, then anything
    // with no remaining extension is conventionally an editable text file.
    const base = e.name.startsWith(".") ? e.name.slice(1) : e.name;
    return !base.includes(".");
  };

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
      clearSelection();
      inspected = null;
      resetDeep();
    } catch (e) {
      navError = e instanceof Error ? e.message : "Couldn't open that folder.";
    } finally {
      loading = false;
    }
  }

  function refresh() {
    void refreshNow();
  }

  /** Re-list the current directory in place. Unlike navigate(), the filter,
      selection (pruned to paths that still exist), and any active deep search
      all survive — so the reload button reloads, nothing else. */
  async function refreshNow() {
    if (!cwd) return;
    searchCache.invalidate(); // a refresh implies the tree may have changed
    loading = true;
    navError = null;
    try {
      const list = await sftpListDir(connectionId, cwd);
      entries = list;
      if (selected.size > 0) {
        applySel(pruneSelection(currentSel(), new Set(list.map((e) => e.path))));
      }
      if (deepSearch && query.trim()) void runDeepSearch();
    } catch (e) {
      navError = e instanceof Error ? e.message : "Couldn't refresh this folder.";
    } finally {
      loading = false;
    }
  }

  function openEntry(e: SftpEntry) {
    if (e.isDir) void navigate(e.path);
    // Archives aren't previewable text — clicking one offers the extract
    // dialog (Download stays available from the row / context menu).
    else if (isArchive(e.name)) extractTo(e);
    else if (isTextFile(e)) {
      // In the IDE, open text in the shared editor area like the Explorer;
      // the standalone modal falls back to its own edit-and-push dialog.
      if (onOpenFile) onOpenFile(e.path);
      else void openEditor(e);
    } else void openPreview(e);
  }

  // Remote archive extraction (ssh exec on the host). `extracting` drives the
  // per-row spinner; success/failure both toast from extractArchive.
  let extracting = $state<string | null>(null);

  /** Extract into the current directory, no prompt. */
  async function extractHere(e: SftpEntry) {
    if (extracting) return;
    const dir = posixParent(e.path);
    extracting = e.path;
    try {
      await extractArchive(connectionId, e.path, dir);
      refresh();
    } catch {
      /* toasted */
    } finally {
      extracting = null;
    }
  }

  /** Prompt for a destination path (created if missing), then extract. */
  function extractTo(e: SftpEntry) {
    prompt = {
      title: `Extract "${e.name}"`,
      value: posixJoin(posixParent(e.path), archiveStem(e.name)),
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
        extracting = e.path;
        try {
          await extractArchive(connectionId, e.path, target);
          refresh();
        } finally {
          extracting = null;
        }
      },
    };
  }

  // Right-click context menu (rows + listing background).
  let menu = $state<{ x: number; y: number; entry: SftpEntry | null } | null>(null);
  function openMenu(ev: MouseEvent, entry: SftpEntry | null) {
    ev.preventDefault();
    ev.stopPropagation();
    menu = { x: ev.clientX, y: ev.clientY, entry };
  }

  /** Copy one remote path to the clipboard (menu action). */
  async function copyPathOf(path: string) {
    try {
      await navigator.clipboard.writeText(path);
    } catch {
      /* no clipboard permission — silently no-op */
    }
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
        const path = posixJoin(cwd, trimmed);
        if (await remoteExists(connectionId, path)) {
          pushNameTaken(trimmed, cwd);
          return;
        }
        await sftpMkdir(connectionId, path);
        refresh();
      },
    };
  }

  function newFile() {
    prompt = {
      title: "New file",
      value: "",
      hint: "Created empty in the current directory.",
      confirmLabel: "Create",
      onConfirm: async (name) => {
        const trimmed = name.trim();
        if (!trimmed) return;
        const path = posixJoin(cwd, trimmed);
        // Writing an empty file over an existing one would TRUNCATE it.
        if (await remoteExists(connectionId, path)) {
          pushNameTaken(trimmed, cwd);
          return;
        }
        await sftpWriteText(connectionId, path, "");
        refresh();
      },
    };
  }

  /** Copy the selected entries' remote paths (or the current directory when
      nothing is selected) to the clipboard, one per line. */
  async function copyPaths() {
    const paths = selected.size > 0 ? [...selected] : [cwd];
    try {
      await navigator.clipboard.writeText(paths.join("\n"));
    } catch {
      /* no clipboard permission — silently no-op, matching IdeWelcome */
    }
  }

  /** What Move/Compress operate on: the whole selection when the menu's
      entry is part of a multi-selection, else just that entry. */
  function actionTargets(e: SftpEntry): string[] {
    return selected.has(e.path) && selected.size > 1 ? [...selected] : [e.path];
  }

  /** Prompt for a destination folder, then move (server-side rename). */
  function moveEntry(e: SftpEntry) {
    const targets = actionTargets(e);
    prompt = {
      title: targets.length === 1 ? `Move "${e.name}" to…` : `Move ${targets.length} items to…`,
      value: posixParent(e.path),
      hint: "Absolute destination folder on the server (it must already exist).",
      confirmLabel: "Move",
      onConfirm: async (dest) => {
        const dir = dest.trim().replace(/\/+$/, "") || "/";
        if (!dir.startsWith("/")) return;
        await runMoves(planMoves(targets, dir));
      },
    };
  }

  /** Prompt for an archive name, then zip the entries on the server. */
  function compressEntry(e: SftpEntry) {
    const base = posixParent(e.path);
    // Everything zipped must sit in one folder (the archive holds relative
    // names) — a multi-selection is filtered to the menu entry's folder.
    const targets = actionTargets(e).filter((p) => posixParent(p) === base);
    prompt = {
      title: targets.length === 1 ? `Compress "${e.name}"` : `Compress ${targets.length} items`,
      value: targets.length === 1 ? `${e.name}.zip` : "Archive.zip",
      hint: "Created next to the originals. Runs zip on the server.",
      confirmLabel: "Compress",
      onConfirm: async (zipName) => {
        const trimmed = zipName.trim();
        if (!trimmed) return;
        const name = trimmed.endsWith(".zip") ? trimmed : `${trimmed}.zip`;
        // zip silently MERGES into an existing archive — replacing one must
        // be an explicit, destructive choice (delete first, then compress).
        const dest = posixJoin(base, name);
        const ex = await remoteExists(connectionId, dest);
        if (ex) {
          if (ex.isDir) {
            pushNameTaken(name, base);
            return;
          }
          const choice = await confirmDialog.open({
            title: `Replace “${name}”?`,
            message:
              "An archive with this name already exists — compressing again would merge into it. Replace deletes it first.",
            destructive: true,
            icon: "circle-alert",
            actions: [
              { label: "Replace", value: "replace", tone: "destructive", icon: "trash-2" },
            ],
          });
          if (choice !== "replace") return;
          await sftpRemoveFile(connectionId, dest);
        }
        await compressEntries(connectionId, base, targets.map(posixBasename), name);
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
        const target = posixJoin(posixParent(e.path), trimmed);
        // Renaming onto an existing entry would clobber it — refuse.
        if (await remoteExists(connectionId, target)) {
          pushNameTaken(trimmed, posixParent(e.path));
          return;
        }
        await sftpRename(connectionId, e.path, target);
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
      // Keep an open deep-result list in sync with the deletion.
      if (deepResults) deepResults = deepResults.filter((x) => x.path !== e.path);
      refresh();
    } catch {
      /* toasted */
    }
  }

  async function upload() {
    // Capture the destination before the dialog (selection could change while
    // it's open). Like every upload entry point, this targets `uploadDest` —
    // the single selected remote folder, or the current directory.
    const dest = uploadDest;
    // Host-side multi-file picker: the backend runs the dialog, canonicalizes
    // each path, and inserts them into the approved set before returning.
    const paths = await safeInvoke<string[]>("sftp_pick_upload_files");
    if (paths.length) void uploadPaths(paths, dest);
  }

  async function uploadFolder() {
    const dest = uploadDest;
    // Host-side folder picker — the directory (and so its whole subtree) is
    // approved before the recursive walk + transfers start.
    const picked = await safeInvoke<string | null>("sftp_pick_upload_dir");
    if (picked) void uploadPaths([picked], dest);
  }

  async function download(e: SftpEntry) {
    // Host-side save dialog: the backend runs the dialog, canonicalizes the
    // result (parent dir + file name for a not-yet-existing destination), and
    // inserts the path into the approved set before returning.
    const dest = await safeInvoke<string | null>("sftp_pick_save_path", {
      defaultName: e.name,
    });
    if (!dest) return;
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

  function iconFor(e: SftpEntry): "folder" | "file-code" | "file-text" | "archive" | "image" {
    if (e.isDir) return "folder";
    if (isArchive(e.name)) return "archive";
    if (/\.(png|jpe?g|gif|webp|bmp|ico|avif|svg)$/i.test(e.name)) return "image";
    return /\.(rs|ts|js|tsx|jsx|py|rb|php|go|java|c|h|cpp|json|sh|svelte|vue)$/i.test(e.name)
      ? "file-code"
      : "file-text";
  }

  function onKeydown(ev: KeyboardEvent) {
    if (ev.key === "Escape") {
      if (menu) menu = null;
      else if (preview) preview = null;
      else if (edit) edit = null;
      else if (chmod) chmod = null;
      else if (prompt) prompt = null;
      else if (confirmDelete) confirmDelete = null;
      else if (selected.size > 0) clearSelection();
      else onClose?.();
    } else if (
      // Delete (or ⌘/Ctrl+Backspace, the macOS Finder chord) deletes the
      // selection — but never while a modal is open or focus is in a field.
      (ev.key === "Delete" || (ev.key === "Backspace" && (ev.metaKey || ev.ctrlKey))) &&
      selected.size > 0 &&
      !menu && !preview && !edit && !chmod && !prompt && !confirmDelete
    ) {
      const t = ev.target as HTMLElement | null;
      if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable)) return;
      ev.preventDefault();
      void batchDelete();
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

<svelte:window onkeydown={onKeydown} onclick={() => (menu = null)} />

<div bind:this={paneEl} class="@container relative flex h-full min-h-0 flex-col">
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
      <!-- Interactive path: click any segment to jump back up the tree. -->
      <nav class="flex min-w-0 items-center gap-0.5 overflow-x-auto font-mono text-[11px]">
        {#each breadcrumbs as crumb, i (crumb.path)}
          {#if i > 0}
            <Icon name="chevron-right" size={10} class="shrink-0 text-fg-subtle" />
          {/if}
          <button
            type="button"
            onclick={() => navigate(crumb.path)}
            disabled={crumb.path === cwd}
            class="shrink-0 rounded px-1 py-0.5 text-fg-subtle hover:bg-surface-2 hover:text-fg disabled:text-fg disabled:hover:bg-transparent"
            title={crumb.path}
          >
            {crumb.name === "/" ? "root" : crumb.name}
          </button>
        {/each}
      </nav>
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

  <!-- Toolbar (full variant only — the sidebar embedding is a plain list and
       defers all of this to the right-hand Files tab). Labels collapse to
       icons (with tooltips) when the pane is narrow. Navigation (up/refresh)
       lives in the path bar above the listing, mirroring the local pane. -->
  {#if variant === "full"}
  <div class="flex items-center gap-1 whitespace-nowrap border-b border-border bg-surface-2/40 px-3 py-2">
    <button
      type="button"
      onclick={newFolder}
      class="inline-flex h-7 items-center gap-1 rounded-md px-2 text-[12px] text-fg-muted hover:bg-surface hover:text-fg"
      title="New folder"
    >
      <Icon name="plus" size={13} /><span class="@max-md:hidden">Folder</span>
    </button>

    <div class="flex-1"></div>

    <!-- Finder-style view switcher: icon grid / list. -->
    <div class="flex items-center rounded-md border border-border/70 p-0.5" role="group" aria-label="View mode">
      <button
        type="button"
        onclick={() => setViewMode("icons")}
        aria-pressed={viewMode === "icons"}
        class="grid h-6 w-6 place-items-center rounded {viewMode === 'icons' ? 'bg-surface-2 text-fg' : 'text-fg-muted hover:text-fg'}"
        title="Icon view"
      >
        <Icon name="grid-2x2" size={13} />
      </button>
      <button
        type="button"
        onclick={() => setViewMode("list")}
        aria-pressed={viewMode === "list"}
        class="grid h-6 w-6 place-items-center rounded {viewMode === 'list' ? 'bg-surface-2 text-fg' : 'text-fg-muted hover:text-fg'}"
        title="List view"
      >
        <Icon name="list" size={13} />
      </button>
    </div>

    <button
      type="button"
      onclick={() => (dualPane = !dualPane)}
      class="inline-flex h-7 items-center gap-1 rounded-md px-2 text-[12px] {dualPane ? 'bg-surface-2 text-fg' : 'text-fg-muted hover:bg-surface hover:text-fg'}"
      title="Toggle local file pane"
      aria-pressed={dualPane}
    >
      <Icon name="panel-left-open" size={13} /><span class="@max-md:hidden">Local</span>
    </button>
    <button
      type="button"
      onclick={uploadFolder}
      class="inline-flex h-7 items-center gap-1 rounded-md px-2 text-[12px] text-fg-muted hover:bg-surface hover:text-fg"
      title={`Upload a folder (and everything in it) to ${uploadDest || "/"}`}
    >
      <Icon name="folder-open" size={13} /><span class="@max-md:hidden">Upload folder</span>
    </button>
    <button
      type="button"
      onclick={upload}
      class="inline-flex h-7 items-center gap-1 rounded-md bg-accent px-2.5 text-[12px] font-medium text-on-accent hover:brightness-110"
      title={`Upload files to ${uploadDest || "/"}`}
    >
      <Icon name="share" size={13} /> Upload
    </button>
  </div>
  {/if}

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

  <!-- Listing (optionally split with the local pane); each column carries its
       own path bar + search so server and local read the same way. The split
       stacks vertically when the pane is too narrow for two usable columns. -->
  <div class="flex min-h-0 flex-1 @max-xl:flex-col">
    {#if dualPane}
      <div class="w-1/2 min-w-0 shrink-0 border-r border-border @max-xl:h-2/5 @max-xl:w-full @max-xl:border-b @max-xl:border-r-0">
        <LocalFilePane
          {connectionId}
          hostLabel={label}
          {uploadDest}
          onUploaded={() => refresh()}
        />
      </div>
    {/if}
    <div class="@container flex min-h-0 flex-1 flex-col">
    <!-- Path bar: up + current dir + refresh — mirrors the local pane. -->
    <div class="flex items-center gap-1 border-b border-border px-2 py-1.5">
      <Icon name="server" size={13} class="shrink-0 text-fg-subtle" />
      <button
        type="button"
        onclick={() => navigate(posixParent(cwd))}
        disabled={cwd === "/" || cwd === ""}
        class="inline-flex h-6 items-center rounded px-1.5 text-[11px] text-fg-muted hover:bg-surface-2 hover:text-fg disabled:opacity-40"
        title="Up one folder"
      >
        <Icon name="arrow-up" size={12} />
      </button>
      <span class="min-w-0 flex-1 truncate font-mono text-[11px] text-fg-subtle" title={cwd}>{cwd || "…"}</span>
      <button
        type="button"
        onclick={refresh}
        class="inline-flex h-6 items-center rounded px-1.5 text-[11px] text-fg-muted hover:bg-surface-2 hover:text-fg"
        title="Refresh"
      >
        <Icon name="refresh-cw" size={12} class={loading ? "animate-spin" : ""} />
      </button>
    </div>
    <!-- Search: live filter over the current folder, or — with the layers
         toggle — a recursive server-side search of all subfolders. -->
    <div class="flex items-center gap-1 border-b border-border px-3 py-1.5">
      <div class="relative min-w-0 flex-1">
        <span class="absolute left-2 top-1/2 -translate-y-1/2 text-fg-subtle">
          <Icon name="search" size={13} />
        </span>
        <input
          bind:value={query}
          oninput={queueDeepSearch}
          placeholder={deepSearch ? "Search subfolders… (*.zip works)" : dualPane ? "Search server…" : "Search this folder…"}
          spellcheck="false"
          class="w-full h-7 rounded-md border border-border bg-surface pl-7 pr-7 text-[12px] text-fg
                 placeholder:text-fg-subtle focus:border-accent/60 focus:outline-none"
        />
        {#if query}
          <button
            type="button"
            onclick={() => { query = ""; resetDeep(); }}
            class="absolute right-2 top-1/2 -translate-y-1/2 rounded p-0.5 text-fg-subtle hover:text-fg"
            aria-label="Clear search"
          >
            <Icon name="x" size={13} />
          </button>
        {/if}
      </div>
      <button
        type="button"
        onclick={toggleDeepSearch}
        aria-pressed={deepSearch}
        class="grid h-7 w-7 shrink-0 place-items-center rounded-md {deepSearch ? 'bg-accent/15 text-accent' : 'text-fg-muted hover:bg-surface hover:text-fg'}"
        title={deepSearch ? "Searching subfolders too — click for this folder only" : "Search subfolders too (deep search)"}
      >
        <Icon name="layers" size={13} />
      </button>
    </div>
    <div
      class="min-h-0 flex-1 overflow-y-auto {dropTarget === cwd ? 'ring-1 ring-inset ring-accent/60 bg-accent/5' : ''}"
      role="presentation"
      onclick={(ev) => { if (ev.target === ev.currentTarget) clearSelection(); }}
      ondragover={listDragOver}
      ondragleave={() => (dropTarget = null)}
      ondrop={(ev) => localDrop(ev, cwd)}
      oncontextmenu={(ev) => openMenu(ev, null)}
    >
    {#if navError}
      <div class="m-4 rounded-md border border-status-crashed/40 bg-status-crashed/10 p-3 text-[12px] text-status-crashed">
        {navError}
      </div>
    {:else if deepSearch && query.trim() && deepResults !== null}
      <!-- Deep search results: every match under the current folder, streamed
           in as the server walk progresses. -->
      <div class="sticky top-0 z-10 flex items-center gap-2 border-b border-border/60 bg-surface px-3 py-1.5 text-[11px] text-fg-subtle">
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
        <p class="p-6 text-center text-[12px] text-fg-subtle">No matches for “{query}” under {cwd || "/"}.</p>
      {:else}
        <ul class="py-1">
          {#each deepResults as e (e.path)}
            <li>
              <button
                type="button"
                onclick={() => deepClick(e)}
                ondblclick={() => openEntry(e)}
                onmousedown={(ev) => { if (ev.detail > 1) ev.preventDefault(); }}
                oncontextmenu={(ev) => openMenu(ev, e)}
                class="flex w-full items-center gap-2 px-3 py-1.5 text-left hover:bg-surface-2/50 {selected.has(e.path) ? 'bg-accent/10' : ''}"
                title={e.path}
              >
                <Icon name={iconFor(e)} size={14} class={e.isDir ? "shrink-0 text-accent" : "shrink-0 text-fg-subtle"} />
                <span class="min-w-0 flex-1 truncate">
                  <span class="text-[12px] text-fg">{e.name}</span>
                  <span class="ml-2 font-mono text-[10.5px] text-fg-subtle">{relParent(e)}</span>
                </span>
                <span class="shrink-0 font-mono text-[11px] text-fg-muted">{e.isDir ? "—" : formatSize(e.size)}</span>
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    {:else if loading && entries.length === 0}
      <p class="p-6 text-center text-[12px] text-fg-subtle">Loading…</p>
    {:else if entries.length === 0}
      <p class="p-6 text-center text-[12px] text-fg-subtle">This folder is empty.</p>
    {:else if visibleEntries.length === 0}
      <p class="p-6 text-center text-[12px] text-fg-subtle">No files match “{query}”.</p>
    {:else if viewMode === "icons" && variant === "full"}
      <!-- Browse view (BookSlash treatment): folder cards with the 3D folder
           icon, then file cards. Plain click opens a folder; modifier clicks
           select. Cards are drag sources (drop on a folder = move there) and
           folder cards are drop targets for both moves and local uploads. -->
      {@const folders = visibleEntries.filter((e) => e.isDir)}
      {@const files = visibleEntries.filter((e) => !e.isDir)}
      <!-- Cards stop click propagation, so any click that reaches this
           wrapper is empty space — it deselects (closing the info panel). -->
      <div
        class="flex min-h-full flex-col gap-4 p-3"
        role="presentation"
        onclick={() => clearSelection()}
      >
        {#if folders.length > 0}
          <section>
            <h3 class="mb-1.5 px-0.5 text-[10.5px] font-semibold uppercase tracking-wide text-fg-subtle">
              Folders
            </h3>
            <div class="grid grid-cols-[repeat(auto-fill,minmax(170px,1fr))] gap-2">
              {#each folders as e, i (e.path)}
                <button
                  type="button"
                  style="animation-delay: {Math.min(i, 12) * 35}ms"
                  onclick={(ev) => {
                    ev.stopPropagation();
                    rowClick(ev, e);
                  }}
                  ondblclick={() => openEntry(e)}
                  onmousedown={(ev) => {
                    if (ev.shiftKey || ev.detail > 1) ev.preventDefault();
                  }}
                  oncontextmenu={(ev) => openMenu(ev, e)}
                  ondragover={(ev) => rowDragOver(ev, e)}
                  ondrop={(ev) => localDrop(ev, e.path)}
                  class="pb-fade-up group flex items-center gap-2.5 rounded-xl border p-3 text-left
                    transition-[border-color,box-shadow,transform] duration-200 active:scale-[0.99]
                    {dropTarget === e.path
                      ? 'border-accent bg-accent/10 ring-2 ring-accent/30'
                      : selected.has(e.path)
                        ? 'border-accent/50 bg-accent/10'
                        : 'border-border bg-surface hover:-translate-y-[1px] hover:border-fg-subtle/40 hover:shadow-[0_4px_14px_-8px_rgba(0,0,0,0.55)]'}"
                  title={e.path}
                >
                  <SftpFolderIcon />
                  <span class="min-w-0 flex-1">
                    <span class="block truncate text-[12.5px] font-semibold tracking-[-0.01em] text-fg">
                      {e.name}{e.isSymlink ? " ↗" : ""}
                    </span>
                    <span class="mt-0.5 block truncate text-[10.5px] tabular-nums text-fg-subtle">
                      {formatMtime(e.mtimeSecs)}
                    </span>
                  </span>
                </button>
              {/each}
            </div>
          </section>
        {/if}

        {#if files.length > 0}
          <section>
            <h3 class="mb-1.5 px-0.5 text-[10.5px] font-semibold uppercase tracking-wide text-fg-subtle">
              Files
            </h3>
            <div class="grid grid-cols-[repeat(auto-fill,minmax(190px,1fr))] gap-2">
              {#each files as e, i (e.path)}
                <div
                  role="button"
                  tabindex="0"
                  style="animation-delay: {Math.min(i, 12) * 35}ms"
                  onclick={(ev) => {
                    ev.stopPropagation();
                    rowClick(ev, e);
                  }}
                  ondblclick={() => openEntry(e)}
                  onkeydown={(ev) => {
                    if (ev.key === "Enter") openEntry(e);
                  }}
                  onmousedown={(ev) => {
                    if (ev.shiftKey || ev.detail > 1) ev.preventDefault();
                  }}
                  oncontextmenu={(ev) => openMenu(ev, e)}
                  class="pb-fade-up group relative flex cursor-default items-center gap-2.5 rounded-xl border p-3
                    transition-[border-color,box-shadow,transform] duration-200 active:scale-[0.99]
                    {selected.has(e.path)
                      ? 'border-accent/50 bg-accent/10'
                      : 'border-border bg-surface hover:-translate-y-[1px] hover:border-fg-subtle/40 hover:shadow-[0_4px_14px_-8px_rgba(0,0,0,0.55)]'}"
                  title={e.path}
                >
                  <span class="grid h-9 w-9 shrink-0 place-items-center rounded-lg bg-surface-2/60">
                    {#if extracting === e.path}
                      <Icon name="refresh-cw" size={16} class="animate-spin text-fg-subtle" />
                    {:else}
                      <Icon name={iconFor(e)} size={16} class="text-fg-muted" />
                    {/if}
                  </span>
                  <span class="min-w-0 flex-1">
                    <span class="block truncate text-[12.5px] font-semibold tracking-[-0.01em] text-fg">
                      {e.name}{e.isSymlink ? " ↗" : ""}
                    </span>
                    <span class="mt-0.5 block truncate text-[10.5px] tabular-nums text-fg-subtle">
                      {formatSize(e.size)} · {formatMtime(e.mtimeSecs)}
                    </span>
                  </span>
                  <!-- Hover actions, top-right reveal (BookSlash treatment). -->
                  <span class="absolute right-2 top-2 z-10 flex gap-1 opacity-0 transition-opacity duration-150 group-hover:opacity-100">
                    <button
                      type="button"
                      onclick={(ev) => { ev.stopPropagation(); void download(e); }}
                      ondblclick={(ev) => ev.stopPropagation()}
                      class="grid h-6 w-6 place-items-center rounded-md border border-border bg-surface text-fg-muted shadow-sm hover:text-fg"
                      title="Download"
                    >
                      <Icon name="arrow-up" size={12} class="rotate-180" />
                    </button>
                    <button
                      type="button"
                      onclick={(ev) => { ev.stopPropagation(); openMenu(ev, e); }}
                      ondblclick={(ev) => ev.stopPropagation()}
                      class="grid h-6 w-6 place-items-center rounded-md border border-border bg-surface text-fg-muted shadow-sm hover:text-fg"
                      title="More"
                      aria-label="More actions"
                    >
                      <Icon name="more-horizontal" size={12} />
                    </button>
                  </span>
                </div>
              {/each}
            </div>
          </section>
        {/if}
      </div>
    {:else}
      <table class="w-full table-fixed text-[12px]">
        <thead class="sticky top-0 bg-surface text-left text-[11px] uppercase text-fg-subtle">
          <tr class="border-b border-border">
            <th class="w-8 pl-4 py-1.5">
              <button
                type="button"
                role="checkbox"
                aria-checked={allVisibleSelected ? "true" : selectedCount > 0 ? "mixed" : "false"}
                onclick={toggleSelectAll}
                aria-label="Select all"
                class="grid h-[15px] w-[15px] place-items-center rounded border {allVisibleSelected || selectedCount > 0
                  ? 'border-accent bg-accent text-on-accent'
                  : 'border-border bg-surface hover:border-fg-subtle'}"
              >
                {#if allVisibleSelected}
                  <Icon name="check" size={11} />
                {:else if selectedCount > 0}
                  <Icon name="minus" size={11} />
                {/if}
              </button>
            </th>
            <!-- Finder-style sortable columns: click toggles asc/desc. -->
            <th class="px-2 py-1.5 font-medium">
              <button
                type="button"
                onclick={() => setSort("name")}
                class="inline-flex items-center gap-0.5 uppercase hover:text-fg"
              >
                Name
                {#if sortKey === "name"}<Icon name={sortDir === "asc" ? "chevron-up" : "chevron-down"} size={10} />{/if}
              </button>
            </th>
            <th class="w-16 px-2 py-1.5 text-right font-medium">
              <button
                type="button"
                onclick={() => setSort("size")}
                class="inline-flex w-full items-center justify-end gap-0.5 uppercase hover:text-fg"
              >
                Size
                {#if sortKey === "size"}<Icon name={sortDir === "asc" ? "chevron-up" : "chevron-down"} size={10} />{/if}
              </button>
            </th>
            <th class="w-36 px-2 py-1.5 font-medium @max-lg:hidden">
              <button
                type="button"
                onclick={() => setSort("mtime")}
                class="inline-flex items-center gap-0.5 uppercase hover:text-fg"
              >
                Modified
                {#if sortKey === "mtime"}<Icon name={sortDir === "asc" ? "chevron-up" : "chevron-down"} size={10} />{/if}
              </button>
            </th>
            <th class="w-24 px-2 py-1.5 font-medium @max-md:hidden">Perms</th>
            <th class="w-28 px-4 py-1.5 text-right font-medium @max-lg:hidden">Actions</th>
          </tr>
        </thead>
        <tbody>
          {#each visibleEntries as e (e.path)}
            <tr
              class="group cursor-default border-b border-border/40 hover:bg-surface-2/50 {selected.has(e.path) ? 'bg-accent/10' : ''} {dropTarget === e.path ? 'bg-accent/10 ring-1 ring-inset ring-accent/60' : ''}"
              aria-selected={selected.has(e.path)}
              onclick={(ev) => rowClick(ev, e)}
              ondblclick={() => openEntry(e)}
              onmousedown={(ev) => { if (ev.shiftKey || ev.detail > 1) ev.preventDefault(); }}
              ondragover={(ev) => rowDragOver(ev, e)}
              ondrop={(ev) => localDrop(ev, e.isDir ? e.path : cwd)}
              oncontextmenu={(ev) => openMenu(ev, e)}
            >
              <td class="w-8 pl-4 py-1.5">
                <button
                  type="button"
                  role="checkbox"
                  aria-checked={selected.has(e.path)}
                  onclick={(ev) => checkboxClick(ev, e)}
                  ondblclick={(ev) => ev.stopPropagation()}
                  aria-label={`Select ${e.name}`}
                  class="grid h-[15px] w-[15px] place-items-center rounded border {selected.has(e.path)
                    ? 'border-accent bg-accent text-on-accent'
                    : 'border-border bg-surface hover:border-fg-subtle'}"
                >
                  {#if selected.has(e.path)}
                    <Icon name="check" size={11} />
                  {/if}
                </button>
              </td>
              <td class="overflow-hidden px-2 py-1.5">
                <!-- Finder-style: clicks (select + inspect) and double-clicks
                     (open) are the row's — the name is plain content. -->
                <span
                  class="flex w-full min-w-0 items-center gap-2 text-left text-fg"
                  title={e.isDir ? "Double-click to open" : isArchive(e.name) ? "Double-click to extract" : isTextFile(e) ? "Double-click to edit" : "Double-click to preview"}
                >
                  {#if extracting === e.path}
                    <Icon name="refresh-cw" size={14} class="shrink-0 animate-spin text-fg-subtle" />
                  {:else}
                    <Icon
                      name={iconFor(e)}
                      size={14}
                      class={e.isDir ? "shrink-0 text-accent" : "shrink-0 text-fg-subtle"}
                    />
                  {/if}
                  <span class="truncate">{e.name}{e.isSymlink ? " ↗" : ""}</span>
                </span>
              </td>
              <td class="whitespace-nowrap px-2 py-1.5 text-right font-mono text-fg-muted">
                {e.isDir ? "—" : formatSize(e.size)}
              </td>
              <td class="whitespace-nowrap px-2 py-1.5 text-fg-muted @max-lg:hidden">{formatMtime(e.mtimeSecs)}</td>
              <td class="whitespace-nowrap px-2 py-1.5 font-mono text-fg-subtle @max-md:hidden">{formatMode(e.permissions)}</td>
              <td class="px-4 py-1.5 @max-lg:hidden">
                <!-- stopPropagation (click + dblclick): hover actions must not
                     retarget the row selection or open the entry underneath. -->
                <div
                  class="flex items-center justify-end gap-0.5 opacity-0 group-hover:opacity-100"
                  role="presentation"
                  ondblclick={(ev) => ev.stopPropagation()}
                >
                  {#if !e.isDir}
                    <button
                      type="button"
                      onclick={(ev) => { ev.stopPropagation(); void download(e); }}
                      class="rounded p-1 text-fg-muted hover:bg-surface hover:text-fg"
                      title="Download"
                    >
                      <Icon name="arrow-up" size={13} class="rotate-180" />
                    </button>
                  {/if}
                  <button
                    type="button"
                    onclick={(ev) => { ev.stopPropagation(); renameEntry(e); }}
                    class="rounded p-1 text-fg-muted hover:bg-surface hover:text-fg"
                    title="Rename"
                  >
                    <Icon name="pencil" size={13} />
                  </button>
                  <button
                    type="button"
                    onclick={(ev) => { ev.stopPropagation(); chmodEntry(e); }}
                    class="rounded p-1 text-fg-muted hover:bg-surface hover:text-fg"
                    title="Permissions"
                  >
                    <Icon name="lock" size={13} />
                  </button>
                  <button
                    type="button"
                    onclick={(ev) => { ev.stopPropagation(); confirmDelete = e; }}
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
    {#if inspectorEntry}
      <!-- Finder-style inspector column. Hidden when the pane is too narrow
           for a third column (the workspace-sidebar embed) — there, folder
           clicks open the editor area's Files tab via `onOpenFolder`. -->
      <div class="w-72 shrink-0 @max-xl:hidden">
        <SftpInspectorPane
          {connectionId}
          entry={inspectorEntry}
          onClose={() => (inspectorOpen = false)}
          onReveal={(path) => void navigate(path)}
          onOpen={(e) => openEntry(e)}
          onDownload={(e) => void download(e)}
          onExtract={(e) => extractTo(e)}
        />
      </div>
    {/if}
  </div>

  <footer class="relative flex items-center gap-2 border-t border-border px-4 py-1.5 text-[11px] text-fg-subtle">
    <!-- Action toolbar (Lapce's file-explorer context-menu actions surfaced as a
         footer row). New file/folder act on the current directory; Rename and
         Copy path act on the selection. -->
    <div class="flex items-center gap-0.5">
      <button
        type="button"
        onclick={newFile}
        class="grid h-6 w-6 place-items-center rounded text-fg-muted hover:bg-surface-2 hover:text-fg"
        title="New file"
        aria-label="New file"
      >
        <Icon name="file-plus" size={13} />
      </button>
      <button
        type="button"
        onclick={newFolder}
        class="grid h-6 w-6 place-items-center rounded text-fg-muted hover:bg-surface-2 hover:text-fg"
        title="New folder"
        aria-label="New folder"
      >
        <Icon name="folder-plus" size={13} />
      </button>
      <button
        type="button"
        onclick={refresh}
        class="grid h-6 w-6 place-items-center rounded text-fg-muted hover:bg-surface-2 hover:text-fg"
        title="Refresh"
        aria-label="Refresh"
      >
        <Icon name="refresh-cw" size={13} class={loading ? "animate-spin" : ""} />
      </button>
      <span class="mx-0.5 h-3.5 w-px bg-border/70"></span>
      <button
        type="button"
        onclick={() => selectedEntry && renameEntry(selectedEntry)}
        disabled={!selectedEntry}
        class="grid h-6 w-6 place-items-center rounded text-fg-muted hover:bg-surface-2 hover:text-fg disabled:opacity-40 disabled:hover:bg-transparent"
        title={selectedEntry ? `Rename "${selectedEntry.name}"` : "Rename (select one item)"}
        aria-label="Rename selected item"
      >
        <Icon name="pencil" size={13} />
      </button>
      <button
        type="button"
        onclick={copyPaths}
        class="grid h-6 w-6 place-items-center rounded text-fg-muted hover:bg-surface-2 hover:text-fg"
        title={selectedCount > 0 ? `Copy path${selectedCount === 1 ? "" : "s"} of selection` : "Copy current folder path"}
        aria-label="Copy path"
      >
        <Icon name="copy" size={13} />
      </button>
      <button
        type="button"
        onclick={batchDelete}
        disabled={selectedCount === 0 || batchBusy}
        class="grid h-6 w-6 place-items-center rounded text-fg-muted hover:bg-status-crashed/10 hover:text-status-crashed disabled:opacity-40 disabled:hover:bg-transparent disabled:hover:text-fg-muted"
        title={selectedCount > 0 ? `Delete ${selectedCount} selected` : "Delete (select items)"}
        aria-label="Delete selected items"
      >
        <Icon name="trash-2" size={13} />
      </button>
    </div>
    <span class="mx-1 h-3.5 w-px bg-border/70"></span>
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
          <ul class="max-h-72 overflow-y-auto p-1.5">
            {#each transfers as t (t.id)}
              <li class="group/transfer rounded-md px-2 py-1.5 hover:bg-surface-2/40">
                <div class="flex items-center gap-2">
                  <Icon
                    name="arrow-up"
                    size={11}
                    class={t.direction === "download" ? "rotate-180 text-fg-subtle" : "text-fg-subtle"}
                  />
                  <span class="min-w-0 flex-1 truncate text-[12px] text-fg" title={t.name}>{t.name}</span>
                  <span class="shrink-0 text-[10.5px] tabular-nums {t.status === 'error' ? 'text-status-crashed' : t.status === 'paused' ? 'text-status-unhealthy' : 'text-fg-subtle'}">
                    {#if t.status === "error"}failed{:else if t.status === "done"}done{:else if t.status === "paused"}paused{:else if t.status === "pending"}queued{:else}{transferPct(t)}%{/if}
                  </span>
                  <!-- Per-status actions: cancel an active one, resume/restart a
                       stopped one, remove a finished one. -->
                  <div class="flex shrink-0 items-center gap-0.5">
                    {#if t.status === "active" || t.status === "pending"}
                      <button type="button" onclick={() => sftpTransfers.cancel(t.id)} class="rounded p-0.5 text-fg-subtle hover:bg-surface-2 hover:text-fg" title="Cancel (keep partial to resume)" aria-label="Cancel transfer">
                        <Icon name="circle-stop" size={12} />
                      </button>
                    {:else if t.status === "paused"}
                      <button type="button" onclick={() => sftpTransfers.resume(t.id)} class="rounded p-0.5 text-accent hover:bg-surface-2" title="Resume from {formatSize(t.transferred)}" aria-label="Resume transfer">
                        <Icon name="play-circle" size={12} />
                      </button>
                      <button type="button" onclick={() => sftpTransfers.remove(t.id)} class="rounded p-0.5 text-fg-subtle opacity-0 hover:bg-surface-2 hover:text-fg group-hover/transfer:opacity-100" title="Remove" aria-label="Remove transfer">
                        <Icon name="x" size={12} />
                      </button>
                    {:else if t.status === "error"}
                      <button type="button" onclick={() => sftpTransfers.resume(t.id)} class="rounded p-0.5 text-accent hover:bg-surface-2" title="Resume from {formatSize(t.transferred)}" aria-label="Resume transfer">
                        <Icon name="play-circle" size={12} />
                      </button>
                      <button type="button" onclick={() => sftpTransfers.retry(t.id)} class="rounded p-0.5 text-fg-subtle hover:bg-surface-2 hover:text-fg" title="Restart from the beginning" aria-label="Retry transfer">
                        <Icon name="rotate-cw" size={12} />
                      </button>
                    {:else if t.status === "done"}
                      <button type="button" onclick={() => sftpTransfers.remove(t.id)} class="rounded p-0.5 text-fg-subtle opacity-0 hover:bg-surface-2 hover:text-fg group-hover/transfer:opacity-100" title="Remove" aria-label="Remove transfer">
                        <Icon name="x" size={12} />
                      </button>
                    {/if}
                  </div>
                </div>
                <div class="mt-1 h-1 overflow-hidden rounded-full bg-surface-2">
                  <div
                    class="h-full rounded-full transition-[width] {t.status === 'error' ? 'bg-status-crashed' : t.status === 'paused' ? 'bg-status-unhealthy' : t.status === 'done' ? 'bg-status-running' : 'bg-accent'}"
                    style="width: {t.status === 'done' ? 100 : transferPct(t)}%"
                  ></div>
                </div>
                <!-- Byte counter + live throughput/ETA. -->
                <div class="mt-0.5 flex items-center justify-between text-[10px] tabular-nums text-fg-subtle">
                  <span>
                    {#if t.total > 0}{formatSize(t.transferred)} / {formatSize(t.total)}{:else if t.transferred > 0}{formatSize(t.transferred)}{/if}
                  </span>
                  {#if t.status === "active" && t.speedBps > 0}
                    <span>{formatSize(t.speedBps)}/s{#if formatEta(t.etaSecs)} · {formatEta(t.etaSecs)} left{/if}</span>
                  {/if}
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

<!-- Context menu. Same null-safe pattern as the Explorer tree: every read is
     `menu?.…` and each handler snapshots the entry before nulling `menu`. -->
{#if menu}
  {@const mx = menu?.x ?? 0}
  {@const my = menu?.y ?? 0}
  {@const entry = menu?.entry ?? null}
  {#key menu}
  <div
    use:clampToViewport
    class="fixed z-50 w-48 rounded-lg border border-border bg-surface p-1 shadow-xl"
    style="left: {mx}px; top: {my}px"
    role="menu"
    tabindex="-1"
  >
    {#if entry}
      <button type="button" role="menuitem" onclick={() => { const en = entry; if (!en) return; menu = null; openEntry(en); }} class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg">
        <Icon name={entry.isDir ? "folder-open" : isArchive(entry.name) ? "package" : "file-text"} size={13} />
        {entry.isDir ? "Open" : isArchive(entry.name) ? "Extract to…" : isTextFile(entry) ? "Edit" : "Preview"}
      </button>
      {#if !entry.isDir && isArchive(entry.name)}
        <button type="button" role="menuitem" onclick={() => { const en = entry; if (!en) return; menu = null; void extractHere(en); }} class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg">
          <Icon name="archive" size={13} /> Extract here
        </button>
      {/if}
      {#if !entry.isDir}
        <button type="button" role="menuitem" onclick={() => { const en = entry; if (!en) return; menu = null; void download(en); }} class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg">
          <Icon name="arrow-up" size={13} class="rotate-180" /> Download
        </button>
      {/if}
      <div class="my-1 border-t border-border/60"></div>
      <button type="button" role="menuitem" onclick={() => { const en = entry; if (!en) return; menu = null; renameEntry(en); }} class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg">
        <Icon name="pencil" size={13} /> Rename
      </button>
      <button type="button" role="menuitem" onclick={() => { const en = entry; if (!en) return; menu = null; moveEntry(en); }} class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg">
        <Icon name="arrow-right" size={13} />
        {selected.has(entry.path) && selectedCount > 1 ? `Move ${selectedCount} items…` : "Move…"}
      </button>
      <button type="button" role="menuitem" onclick={() => { const en = entry; if (!en) return; menu = null; chmodEntry(en); }} class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg">
        <Icon name="lock" size={13} /> Change permissions
      </button>
      <button type="button" role="menuitem" onclick={() => { const en = entry; if (!en) return; menu = null; compressEntry(en); }} class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg">
        <Icon name="archive" size={13} />
        {selected.has(entry.path) && selectedCount > 1 ? `Compress ${selectedCount} items` : "Compress"}
      </button>
      <button type="button" role="menuitem" onclick={() => { const p = entry?.path; menu = null; if (p) void copyPathOf(p); }} class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg">
        <Icon name="copy" size={13} /> Copy path
      </button>
      <div class="my-1 border-t border-border/60"></div>
      <button type="button" role="menuitem" onclick={() => { const en = entry; if (!en) return; menu = null; confirmDelete = en; }} class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-status-crashed hover:bg-status-crashed/10">
        <Icon name="trash-2" size={13} /> Delete
      </button>
    {:else}
      <button type="button" role="menuitem" onclick={() => { menu = null; newFile(); }} class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg">
        <Icon name="file-plus" size={13} /> New file
      </button>
      <button type="button" role="menuitem" onclick={() => { menu = null; newFolder(); }} class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg">
        <Icon name="folder-plus" size={13} /> New folder
      </button>
      <div class="my-1 border-t border-border/60"></div>
      <button type="button" role="menuitem" onclick={() => { menu = null; void upload(); }} class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg">
        <Icon name="share" size={13} /> Upload files here
      </button>
      <button type="button" role="menuitem" onclick={() => { menu = null; void uploadFolder(); }} class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg">
        <Icon name="folder-open" size={13} /> Upload folder here
      </button>
      <div class="my-1 border-t border-border/60"></div>
      <button type="button" role="menuitem" onclick={() => { menu = null; void copyPathOf(cwd); }} class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg">
        <Icon name="copy" size={13} /> Copy folder path
      </button>
      <button type="button" role="menuitem" onclick={() => { menu = null; refresh(); }} class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg">
        <Icon name="refresh-cw" size={13} /> Refresh
      </button>
    {/if}
  </div>
  {/key}
{/if}

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

<style>
  /* Card entrance: fade + small upward drift, staggered per card via an
     inline animation-delay (35ms each, capped). BookSlash's anim-fade-up. */
  @keyframes pb-fade-up {
    from {
      opacity: 0;
      transform: translateY(6px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }
  .pb-fade-up {
    animation: pb-fade-up 500ms cubic-bezier(0.16, 1, 0.3, 1) both;
  }
  @media (prefers-reduced-motion: reduce) {
    .pb-fade-up {
      animation: none;
    }
  }
</style>
