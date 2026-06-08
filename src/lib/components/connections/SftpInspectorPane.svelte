<!--
  SftpInspectorPane — the Finder-style right-hand column of the SFTP browser:
  a single click in the listing selects an entry and shows it here. Folders
  show a navigable contents list (click to drill in, back to retrace) plus a
  Get-Info block; files show a preview (image / text excerpt) over the same
  info block. Opening (navigate / edit / extract) stays the parent's job via
  the onReveal / onOpen callbacks, so the pane never duplicates that logic.
-->
<script lang="ts">
  import Icon from "$lib/components/atoms/Icon.svelte";
  import { invokeQuiet } from "$lib/ipc";
  import { sftpListDir, posixParent, formatMode, formatMtime, formatSize, type SftpPreview } from "$lib/sftp";
  import { isArchive } from "$lib/sftpArchive";
  import type { SftpEntry } from "$lib/types/sshTunnels";

  let {
    connectionId,
    entry,
    onClose,
    onReveal,
    onOpen,
    onDownload,
    onExtract,
  }: {
    connectionId: string;
    entry: SftpEntry;
    onClose: () => void;
    /** Jump the main listing to a folder path. */
    onReveal: (path: string) => void;
    /** Open a (non-archive) file the host's standard way (editor / preview). */
    onOpen: (e: SftpEntry) => void;
    onDownload: (e: SftpEntry) => void;
    /** Extract an archive. Hosts without an extract flow omit it — archives
        then offer Download only. */
    onExtract?: (e: SftpEntry) => void;
  } = $props();

  // The node on display: the selected entry, unless the user has drilled into
  // it (panel-only navigation). `trail` is the back stack; a new selection
  // resets both.
  let drilled = $state<SftpEntry | null>(null);
  let trail = $state<SftpEntry[]>([]);
  const node = $derived(drilled ?? entry);

  $effect(() => {
    void entry; // a new selection resets panel-local navigation
    drilled = null;
    trail = [];
  });

  function drill(e: SftpEntry) {
    trail = [...trail, node];
    drilled = e;
  }

  function back() {
    const prev = trail[trail.length - 1];
    if (!prev) return;
    trail = trail.slice(0, -1);
    drilled = prev;
  }

  // Per-node payload: folder → its listing, file → a quiet preview attempt
  // (no toast — an unpreviewable selection is normal, not an error).
  let items = $state<SftpEntry[] | null>(null);
  let itemsError = $state(false);
  let preview = $state<SftpPreview | null>(null);
  let previewPending = $state(false);
  // Recursive folder size, computed via `du` on the host — SFTP can't stat a
  // tree. null while pending/failed; the Size row falls back to the count.
  let dirSize = $state<number | null>(null);
  let dirSizePending = $state(false);
  let loadToken = 0;

  /** POSIX shell single-quote escaping (matches sftpArchive's). */
  const shq = (p: string) => `'${p.replaceAll("'", "'\\''")}'`;

  $effect(() => {
    const n = node;
    const token = ++loadToken;
    items = null;
    itemsError = false;
    preview = null;
    previewPending = false;
    dirSize = null;
    dirSizePending = false;
    if (n.isDir) {
      sftpListDir(connectionId, n.path).then(
        (list) => {
          if (token === loadToken) items = list;
        },
        () => {
          if (token === loadToken) itemsError = true;
        },
      );
      // `du -sk` is the portable form (BSD has no -b); KiB → bytes. A
      // permission-denied subtree still prints a (partial) total on exit 1,
      // so parse stdout regardless of the exit code. Quiet on failure.
      dirSizePending = true;
      invokeQuiet<{ stdout: string; stderr: string; exitCode: number }>("ssh_exec_run", {
        input: { connectionId, command: `du -sk ${shq(n.path)}`, cwd: null },
      }).then(
        (res) => {
          if (token !== loadToken) return;
          dirSizePending = false;
          const kb = parseInt(res.stdout.trim(), 10);
          if (Number.isFinite(kb) && kb >= 0) dirSize = kb * 1024;
        },
        () => {
          if (token === loadToken) dirSizePending = false;
        },
      );
    } else {
      previewPending = true;
      invokeQuiet<SftpPreview>("sftp_read_preview", {
        input: { connectionId, path: n.path },
      }).then(
        (p) => {
          if (token !== loadToken) return;
          preview = p;
          previewPending = false;
        },
        () => {
          // Too large / unreadable — the info block still renders.
          if (token === loadToken) previewPending = false;
        },
      );
    }
  });

  function kindOf(e: SftpEntry): string {
    if (e.isDir) return e.isSymlink ? "Folder (symlink)" : "Folder";
    if (isArchive(e.name)) return "Archive";
    const m = e.name.match(/\.([A-Za-z0-9]+)$/);
    return m ? `${m[1].toUpperCase()} file` : "Document";
  }

  function iconFor(e: SftpEntry): "folder" | "file-code" | "file-text" | "archive" | "image" {
    if (e.isDir) return "folder";
    if (isArchive(e.name)) return "archive";
    if (/\.(png|jpe?g|gif|webp|bmp|ico|avif|svg)$/i.test(e.name)) return "image";
    return /\.(rs|ts|js|tsx|jsx|py|rb|php|go|java|c|h|cpp|json|sh|svelte|vue)$/i.test(e.name)
      ? "file-code"
      : "file-text";
  }

  let copied = $state(false);
  let copiedTimer: ReturnType<typeof setTimeout> | null = null;
  async function copyPath() {
    try {
      await navigator.clipboard.writeText(node.path);
      copied = true;
      if (copiedTimer) clearTimeout(copiedTimer);
      copiedTimer = setTimeout(() => (copied = false), 1200);
    } catch {
      /* no clipboard permission — silently no-op */
    }
  }
</script>

<aside class="flex h-full min-h-0 flex-col border-l border-border bg-surface">
  <!-- Top bar: back through the panel's own trail · close. -->
  <div class="flex items-center gap-1 border-b border-border px-2 py-1.5">
    <button
      type="button"
      onclick={back}
      class="inline-flex h-6 items-center rounded px-1 text-fg-muted hover:bg-surface-2 hover:text-fg {trail.length === 0 ? 'invisible' : ''}"
      title="Back"
      aria-label="Back"
    >
      <Icon name="chevron-left" size={13} />
    </button>
    <span class="min-w-0 flex-1 truncate text-center text-[11px] font-medium text-fg-muted">
      {trail.length > 0 ? node.name : "Info"}
    </span>
    <button
      type="button"
      onclick={onClose}
      class="inline-flex h-6 items-center rounded px-1 text-fg-muted hover:bg-surface-2 hover:text-fg"
      title="Hide info panel"
      aria-label="Hide info panel"
    >
      <Icon name="x" size={13} />
    </button>
  </div>

  <div class="min-h-0 flex-1 overflow-y-auto">
    <!-- Hero: large icon (or image thumbnail), name, kind. -->
    <div class="flex flex-col items-center gap-1.5 px-4 pb-3 pt-4 text-center">
      {#if preview?.kind === "image" && preview.base64}
        <img
          src={`data:${preview.mime};base64,${preview.base64}`}
          alt={node.name}
          class="max-h-36 max-w-full rounded-lg border border-border/60 object-contain shadow-sm"
        />
      {:else}
        <div class="grid h-14 w-14 place-items-center rounded-xl bg-surface-2/70">
          <Icon
            name={iconFor(node)}
            size={30}
            class={node.isDir ? "text-accent" : "text-fg-muted"}
          />
        </div>
      {/if}
      <p class="w-full break-words px-1 text-[13px] font-semibold leading-tight text-fg">
        {node.name}{node.isSymlink ? " ↗" : ""}
      </p>
      <p class="text-[11px] text-fg-subtle">
        {kindOf(node)}{node.isDir ? "" : ` · ${formatSize(node.size)}`}
      </p>
    </div>

    {#if !node.isDir && preview?.kind === "text" && preview.text}
      <!-- Text excerpt — a glance at the file without opening it. -->
      <div class="mx-3 mb-3 max-h-40 overflow-hidden rounded-lg border border-border/70 bg-surface-2/30 px-2.5 py-2">
        <pre class="whitespace-pre-wrap break-words font-mono text-[10.5px] leading-relaxed text-fg-muted">{preview.text.slice(0, 2000)}</pre>
      </div>
    {/if}

    {#if node.isDir}
      <!-- Folder contents: click drills the panel in (back retraces),
           double-click hands off to the main pane (navigate / open). -->
      <div class="mx-3 mb-3 overflow-hidden rounded-lg border border-border/70">
        <div class="flex items-center border-b border-border/60 bg-surface-2/40 px-2.5 py-1.5 text-[10.5px] font-medium uppercase tracking-wide text-fg-subtle">
          Contents
          <span class="ml-auto normal-case tracking-normal">
            {#if items}{items.length} item{items.length === 1 ? "" : "s"}{/if}
          </span>
        </div>
        {#if itemsError}
          <p class="px-2.5 py-3 text-center text-[11px] text-fg-subtle">Couldn't read this folder.</p>
        {:else if items === null}
          <p class="px-2.5 py-3 text-center text-[11px] text-fg-subtle">Loading…</p>
        {:else if items.length === 0}
          <p class="px-2.5 py-3 text-center text-[11px] text-fg-subtle">Empty folder.</p>
        {:else}
          <ul class="max-h-64 overflow-y-auto py-0.5">
            {#each items as it (it.path)}
              <li>
                <button
                  type="button"
                  onclick={() => drill(it)}
                  ondblclick={() =>
                    it.isDir
                      ? onReveal(it.path)
                      : isArchive(it.name)
                        ? onExtract?.(it)
                        : onOpen(it)}
                  class="flex w-full items-center gap-2 px-2.5 py-1 text-left hover:bg-surface-2/60"
                  title={it.path}
                >
                  <Icon
                    name={iconFor(it)}
                    size={13}
                    class={it.isDir ? "shrink-0 text-accent" : "shrink-0 text-fg-subtle"}
                  />
                  <span class="min-w-0 flex-1 truncate text-[12px] text-fg">{it.name}</span>
                  {#if it.isDir}
                    <Icon name="chevron-right" size={11} class="shrink-0 text-fg-subtle" />
                  {:else}
                    <span class="shrink-0 font-mono text-[10.5px] text-fg-subtle">{formatSize(it.size)}</span>
                  {/if}
                </button>
              </li>
            {/each}
          </ul>
        {/if}
      </div>
    {:else if previewPending}
      <p class="mx-3 mb-3 text-center text-[11px] text-fg-subtle">Loading preview…</p>
    {/if}

    <!-- Get-Info block. -->
    <div class="mx-3 mb-3 divide-y divide-border/50 overflow-hidden rounded-lg border border-border/70 text-[11.5px]">
      <div class="flex items-baseline justify-between gap-3 px-2.5 py-1.5">
        <span class="shrink-0 text-fg-subtle">Kind</span>
        <span class="min-w-0 truncate text-right text-fg">{kindOf(node)}</span>
      </div>
      <div class="flex items-baseline justify-between gap-3 px-2.5 py-1.5">
        <span class="shrink-0 text-fg-subtle">Size</span>
        <span class="min-w-0 truncate text-right font-mono text-fg">
          {#if node.isDir}
            <!-- Real (recursive) size from `du`; "—" when exec isn't
                 available (the Contents header still shows the count). -->
            {#if dirSize !== null}
              {formatSize(dirSize)}
            {:else if dirSizePending}
              <span class="text-fg-subtle">Calculating…</span>
            {:else}
              —
            {/if}
          {:else}
            {formatSize(node.size)}
          {/if}
        </span>
      </div>
      <div class="flex items-center justify-between gap-3 px-2.5 py-1.5">
        <span class="shrink-0 text-fg-subtle">Where</span>
        <button
          type="button"
          onclick={copyPath}
          class="group/where flex min-w-0 items-center gap-1 text-right"
          title={copied ? "Copied" : `${node.path}\nClick to copy the full path`}
        >
          <span class="min-w-0 truncate font-mono text-fg group-hover/where:text-accent">
            {posixParent(node.path)}
          </span>
          <Icon
            name={copied ? "check" : "copy"}
            size={11}
            class="shrink-0 {copied ? 'text-status-running' : 'text-fg-subtle opacity-0 group-hover/where:opacity-100'}"
          />
        </button>
      </div>
      <div class="flex items-baseline justify-between gap-3 px-2.5 py-1.5">
        <span class="shrink-0 text-fg-subtle">Modified</span>
        <span class="min-w-0 truncate text-right text-fg">{formatMtime(node.mtimeSecs)}</span>
      </div>
      <div class="flex items-baseline justify-between gap-3 px-2.5 py-1.5">
        <span class="shrink-0 text-fg-subtle">Permissions</span>
        <span class="min-w-0 truncate text-right font-mono text-fg">
          {#if node.permissions != null}
            {formatMode(node.permissions)} · {(node.permissions & 0o777).toString(8).padStart(3, "0")}
          {:else}
            —
          {/if}
        </span>
      </div>
    </div>
  </div>

  <!-- Actions. -->
  <div class="flex items-center gap-1.5 border-t border-border px-3 py-2">
    {#if node.isDir}
      <button
        type="button"
        onclick={() => onReveal(node.path)}
        class="inline-flex h-7 flex-1 items-center justify-center gap-1.5 rounded-md bg-accent px-2 text-[12px] font-medium text-on-accent hover:brightness-110"
      >
        <Icon name="folder-open" size={13} /> Open
      </button>
    {:else}
      {#if isArchive(node.name)}
        {#if onExtract}
          <button
            type="button"
            onclick={() => onExtract(node)}
            class="inline-flex h-7 flex-1 items-center justify-center gap-1.5 rounded-md bg-accent px-2 text-[12px] font-medium text-on-accent hover:brightness-110"
          >
            <Icon name="package" size={13} /> Extract
          </button>
        {/if}
      {:else}
        <button
          type="button"
          onclick={() => onOpen(node)}
          class="inline-flex h-7 flex-1 items-center justify-center gap-1.5 rounded-md bg-accent px-2 text-[12px] font-medium text-on-accent hover:brightness-110"
        >
          <Icon name="pencil" size={13} /> Open
        </button>
      {/if}
      <button
        type="button"
        onclick={() => onDownload(node)}
        class="inline-flex h-7 flex-1 items-center justify-center gap-1.5 rounded-md bg-surface-2 px-2 text-[12px] font-medium text-fg hover:bg-surface-2/70"
      >
        <Icon name="arrow-up" size={13} class="rotate-180" /> Download
      </button>
    {/if}
  </div>
</aside>
