<!--
  ImportSection — Settings → Import.

  On mount:  calls `detect_sources` and renders one row per known source
             tool (Herd / ServBay / MAMP) with its presence + site count.
  Per source: "Import sites" expands a preview pulled from the backend,
             with per-row checkboxes, collision flags, and a Commit
             button that calls `import_projects` for the selected ids.

  Project list refresh happens after a successful commit so the new
  rows appear in the dashboard table without a page reload.
-->
<script lang="ts">
  import { onMount } from "svelte";

  import { DashboardCard, Icon } from "$lib/components/atoms";
  import { safeInvoke } from "$lib/ipc";
  import { errorBus } from "$lib/stores/errors.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import type {
    DetectedSource,
    ImportPreviewRow,
    ImportResult,
    ImportSource,
  } from "$lib/types/import";

  let sources = $state<DetectedSource[]>([]);
  let loading = $state<boolean>(true);

  /** Map of source → preview rows once expanded. `null` while loading. */
  let previews = $state<Record<string, ImportPreviewRow[] | null>>({});
  /** Map of source → set of selected suggestedIds for that source. */
  let selected = $state<Record<string, Set<string>>>({});
  /** Source currently committing — locks the row's buttons. */
  let committing = $state<string | null>(null);

  async function refresh() {
    loading = true;
    try {
      sources = await safeInvoke<DetectedSource[]>("detect_sources");
    } catch {
      sources = [];
    } finally {
      loading = false;
    }
  }

  async function loadPreview(source: ImportSource) {
    previews = { ...previews, [source]: null };
    try {
      const rows = await safeInvoke<ImportPreviewRow[]>("preview_import", {
        source,
      });
      previews = { ...previews, [source]: rows };
      // Default: every non-colliding row is selected.
      const sel = new Set<string>();
      for (const row of rows) {
        if (!row.idCollision && !row.pathCollision) {
          sel.add(row.site.suggestedId);
        }
      }
      selected = { ...selected, [source]: sel };
    } catch {
      previews = { ...previews, [source]: [] };
    }
  }

  function toggleRow(source: ImportSource, id: string) {
    const sel = new Set(selected[source] ?? []);
    if (sel.has(id)) {
      sel.delete(id);
    } else {
      sel.add(id);
    }
    selected = { ...selected, [source]: sel };
  }

  function selectAll(source: ImportSource) {
    const rows = previews[source] ?? [];
    const sel = new Set<string>();
    for (const row of rows) {
      sel.add(row.site.suggestedId);
    }
    selected = { ...selected, [source]: sel };
  }

  function selectNonColliding(source: ImportSource) {
    const rows = previews[source] ?? [];
    const sel = new Set<string>();
    for (const row of rows) {
      if (!row.idCollision && !row.pathCollision) {
        sel.add(row.site.suggestedId);
      }
    }
    selected = { ...selected, [source]: sel };
  }

  function deselectAll(source: ImportSource) {
    selected = { ...selected, [source]: new Set() };
  }

  async function commit(source: ImportSource) {
    const ids = Array.from(selected[source] ?? []);
    if (ids.length === 0) return;
    committing = source;
    try {
      const result = await safeInvoke<ImportResult>("import_projects", {
        source,
        ids,
      });
      await projects.refresh();
      errorBus.push({
        code: "IMPORT_OK",
        category: "lifecycle",
        whatHappened: `Imported ${result.imported.length} ${result.imported.length === 1 ? "project" : "projects"} from ${labelFor(source)}.`,
        whyItMatters:
          result.skipped.length > 0
            ? `${result.skipped.length} skipped — see the row error notes below.`
            : "Start them from the projects table when you're ready.",
        whoCausedIt: "system",
        actions: [],
      });
      // Re-fetch preview so the just-imported rows now show as collisions.
      await loadPreview(source);
    } catch {
      /* safeInvoke toast already pushed */
    } finally {
      committing = null;
    }
  }

  function labelFor(source: ImportSource): string {
    return sources.find((s) => s.source === source)?.label ?? source;
  }

  function selectionSize(source: ImportSource): number {
    return selected[source]?.size ?? 0;
  }

  onMount(() => {
    void refresh();
  });
</script>

<DashboardCard title="Import from another tool" flush>
  <p class="text-xs text-fg-muted">
    Detects sites you've already set up in Laravel Herd, ServBay, or MAMP and
    registers them as PortBay projects. Original tool's config is left
    untouched.
  </p>

  {#if loading}
    <p class="text-xs text-fg-subtle mt-3">Scanning…</p>
  {:else}
    <div class="mt-3 space-y-3">
      {#each sources as src (src.source)}
        <div class="border border-border rounded-md">
          <div class="flex items-center gap-2 px-3 py-2">
            <div class="flex-1 min-w-0">
              <div class="text-sm font-medium text-fg">{src.label}</div>
              <div class="text-[11px] text-fg-muted">
                {#if src.present}
                  {src.siteCount} site{src.siteCount === 1 ? "" : "s"} found
                  {#if src.note}· {src.note}{/if}
                {:else}
                  not installed
                  {#if src.note}· {src.note}{/if}
                {/if}
              </div>
            </div>
            {#if src.present && src.siteCount > 0}
              {#if previews[src.source] === undefined}
                <button
                  type="button"
                  onclick={() => loadPreview(src.source)}
                  class="text-xs px-2.5 py-1 rounded-md border border-border text-fg-muted hover:text-fg hover:border-border-strong transition-colors"
                >
                  Preview sites
                </button>
              {/if}
            {/if}
          </div>

          {#if previews[src.source] !== undefined}
            {#if previews[src.source] === null}
              <p class="px-3 pb-3 text-xs text-fg-subtle">Loading preview…</p>
            {:else}
              <div class="px-3 pb-3 space-y-2">
                <div class="flex items-center gap-2 text-[11px] text-fg-muted">
                  <button
                    type="button"
                    onclick={() => selectAll(src.source)}
                    class="px-1.5 py-0.5 rounded hover:bg-surface-2"
                  >
                    All
                  </button>
                  <button
                    type="button"
                    onclick={() => selectNonColliding(src.source)}
                    class="px-1.5 py-0.5 rounded hover:bg-surface-2"
                  >
                    Skip collisions
                  </button>
                  <button
                    type="button"
                    onclick={() => deselectAll(src.source)}
                    class="px-1.5 py-0.5 rounded hover:bg-surface-2"
                  >
                    None
                  </button>
                  <span class="ml-auto">
                    {selectionSize(src.source)}/{previews[src.source]?.length ?? 0}
                    selected
                  </span>
                </div>

                <div class="border border-border rounded overflow-hidden">
                  {#each previews[src.source] ?? [] as row, i (row.site.path)}
                    <label
                      class="flex items-center gap-2 px-2 py-1.5 text-xs cursor-pointer border-b border-border last:border-b-0 hover:bg-surface-2/40"
                    >
                      <input
                        type="checkbox"
                        checked={selected[src.source]?.has(row.site.suggestedId) ?? false}
                        onchange={() => toggleRow(src.source, row.site.suggestedId)}
                        class="accent-accent"
                      />
                      <div class="flex-1 min-w-0">
                        <div class="font-medium text-fg truncate">
                          {row.site.suggestedName}
                          <span class="font-mono text-[10px] text-fg-subtle ml-1">
                            {row.site.suggestedId}
                          </span>
                        </div>
                        <div class="font-mono text-[10px] text-fg-muted truncate">
                          {row.site.hostname}
                          {#if row.site.phpVersion}
                            <span class="ml-2 text-fg-subtle">
                              PHP {row.site.phpVersion}
                            </span>
                          {/if}
                          {#if row.site.https}
                            <span class="ml-2 text-status-running">https</span>
                          {/if}
                        </div>
                      </div>
                      {#if row.idCollision}
                        <span
                          title="A project with this id already exists"
                          class="text-[10px] text-status-unhealthy"
                        >
                          id taken
                        </span>
                      {/if}
                      {#if row.pathCollision}
                        <span
                          title="A project with this path already exists"
                          class="text-[10px] text-status-unhealthy"
                        >
                          path in use
                        </span>
                      {/if}
                    </label>
                  {/each}
                </div>

                <button
                  type="button"
                  disabled={selectionSize(src.source) === 0 || committing === src.source}
                  onclick={() => commit(src.source)}
                  class="text-xs px-3 py-1.5 rounded-md text-accent border border-accent/40 hover:bg-accent/10 transition-colors disabled:opacity-50"
                >
                  {#if committing === src.source}
                    <Icon name="refresh-cw" size={11} class="animate-spin" />
                    Importing…
                  {:else}
                    Import {selectionSize(src.source)}
                    site{selectionSize(src.source) === 1 ? "" : "s"}
                  {/if}
                </button>
              </div>
            {/if}
          {/if}
        </div>
      {/each}
    </div>
  {/if}
</DashboardCard>
