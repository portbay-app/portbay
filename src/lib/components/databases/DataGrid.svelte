<script lang="ts">
  /**
   * DataGrid — shared results grid for TableDoc and QueryDoc.
   * Fills its parent container (h-full flex flex-col min-h-0).
   * Sticky toolbar + scrollable rows + sticky pagination footer.
   *
   * Editing (opt-in via `editable`): when the grid is backed by a real table
   * that has a primary key, cells become editable, rows can be added/deleted,
   * and changes stage locally. A "Review N changes" bar lets the user see the
   * exact UPDATE/INSERT/DELETE SQL (rendered server-side) before it runs in a
   * single transaction through `database_client_apply_writes`.
   */
  import Icon from "$lib/components/atoms/Icon.svelte";
  import JsonTree from "$lib/components/databases/JsonTree.svelte";
  import { safeInvoke } from "$lib/ipc";
  import type { DbClientRows } from "$lib/types/databases";

  /** Identifies the concrete table behind the grid so edits can target it. */
  interface EditableConfig {
    instanceId: string;
    schema: string | null;
    table: string;
  }

  /** Structured edit shape mirrored by the Rust `RowEdit` enum. */
  type RowEdit =
    | { kind: "update"; table: string; pk: CellValue[]; set: CellValue[] }
    | { kind: "insert"; table: string; values: CellValue[] }
    | { kind: "delete"; table: string; pk: CellValue[] };
  interface CellValue {
    column: string;
    value: unknown;
  }

  interface Props {
    rows: DbClientRows | null;
    loading?: boolean;
    error?: string | null;
    title?: string;
    emptyText?: string;
    /** Base filename for CSV export (without extension). */
    exportName?: string;
    /** When set and the table has a primary key, the grid becomes editable. */
    editable?: EditableConfig | null;
    /** Called after a successful apply so the parent can reload fresh rows. */
    onApplied?: () => void;
  }

  let {
    rows,
    loading = false,
    error = null,
    title,
    emptyText = "No rows.",
    exportName = "rows",
    editable = null,
    onApplied,
  }: Props = $props();

  const pageSizes = [25, 50, 100];

  let rowFilter = $state<string>("");
  let sortColumn = $state<string | null>(null);
  let sortDirection = $state<"asc" | "desc">("asc");
  let page = $state<number>(0);
  let pageSize = $state<number>(25);
  let inspected = $state<{ label: string; value: unknown } | null>(null);

  // ─── Editing state ────────────────────────────────────────────────────────
  // Pending updates keyed by a row's primary-key signature → { col: newValue }.
  let edits = $state<Record<string, Record<string, unknown>>>({});
  // Primary-key signatures marked for deletion.
  let deletes = $state<Set<string>>(new Set());
  // Draft rows to insert (column → value; only touched columns are sent).
  let inserts = $state<Array<{ tempId: string; values: Record<string, unknown> }>>([]);
  // The cell currently being edited (existing rows only) + its text buffer.
  let editing = $state<{ rowKey: string; col: string } | null>(null);
  let editText = $state<string>("");
  // Review/apply flow.
  let reviewing = $state(false);
  let previewSql = $state<string[]>([]);
  let applying = $state(false);
  let applyError = $state<string | null>(null);

  const pkColumns = $derived(
    rows ? rows.columns.filter((c) => c.primaryKey).map((c) => c.name) : [],
  );
  const canEdit = $derived(!!editable && pkColumns.length > 0);

  // Reset view state AND any staged edits whenever a fresh result set arrives —
  // pending edits are keyed by row identity that may no longer exist.
  $effect(() => {
    void rows;
    rowFilter = "";
    sortColumn = null;
    sortDirection = "asc";
    page = 0;
    inspected = null;
    edits = {};
    deletes = new Set();
    inserts = [];
    editing = null;
    reviewing = false;
    applyError = null;
  });

  function columnIndex(name: string): number {
    return rows ? rows.columns.findIndex((c) => c.name === name) : -1;
  }

  /** Stable signature for a row built from its primary-key column values. */
  function rowKeyOf(row: unknown[]): string {
    const sig: Record<string, unknown> = {};
    for (const name of pkColumns) {
      const idx = columnIndex(name);
      sig[name] = idx >= 0 ? row[idx] : null;
    }
    return JSON.stringify(sig);
  }

  function pkPredicate(rowKey: string): CellValue[] {
    const sig = JSON.parse(rowKey) as Record<string, unknown>;
    return Object.entries(sig).map(([column, value]) => ({ column, value }));
  }

  function cellText(value: unknown): string {
    if (value === null || value === undefined) return "NULL";
    if (typeof value === "string") return value;
    if (typeof value === "number" || typeof value === "boolean") return String(value);
    return JSON.stringify(value);
  }

  function isInspectable(value: unknown): boolean {
    return Array.isArray(value) || (typeof value === "object" && value !== null);
  }

  /** Best-effort conversion of edited text back to a value matching the column. */
  function coerceLike(original: unknown, text: string): unknown {
    if (typeof original === "number") {
      const n = Number(text);
      return text.trim() !== "" && Number.isFinite(n) ? n : text;
    }
    if (typeof original === "boolean") {
      const t = text.trim().toLowerCase();
      if (t === "true" || t === "1") return true;
      if (t === "false" || t === "0") return false;
      return text;
    }
    if (original !== null && typeof original === "object") {
      try {
        return JSON.parse(text);
      } catch {
        return text;
      }
    }
    return text;
  }

  function valuesEqual(a: unknown, b: unknown): boolean {
    if (a === b) return true;
    return JSON.stringify(a) === JSON.stringify(b);
  }

  function displayValue(rowKey: string, colName: string, original: unknown): unknown {
    const e = edits[rowKey];
    return e && colName in e ? e[colName] : original;
  }

  function isEdited(rowKey: string, colName: string): boolean {
    return !!edits[rowKey] && colName in edits[rowKey];
  }

  function startEdit(rowKey: string, col: string, current: unknown) {
    if (!canEdit || deletes.has(rowKey)) return;
    editing = { rowKey, col };
    editText =
      current === null || current === undefined
        ? ""
        : typeof current === "object"
          ? JSON.stringify(current)
          : String(current);
  }

  function commitEdit(original: unknown) {
    if (!editing) return;
    const { rowKey, col } = editing;
    const value = editText === "" ? null : coerceLike(original, editText);
    const next = { ...(edits[rowKey] ?? {}) };
    if (valuesEqual(value, original)) {
      delete next[col];
    } else {
      next[col] = value;
    }
    if (Object.keys(next).length === 0) {
      const clone = { ...edits };
      delete clone[rowKey];
      edits = clone;
    } else {
      edits = { ...edits, [rowKey]: next };
    }
    editing = null;
  }

  /** Focus + select the cell editor as soon as it mounts. */
  function focusEditor(node: HTMLInputElement) {
    node.focus();
    node.select();
  }

  function onEditorKeydown(e: KeyboardEvent, original: unknown) {
    if (e.key === "Enter") {
      e.preventDefault();
      commitEdit(original);
    } else if (e.key === "Escape") {
      e.preventDefault();
      editing = null;
    }
  }

  function toggleDelete(rowKey: string) {
    const next = new Set(deletes);
    if (next.has(rowKey)) next.delete(rowKey);
    else next.add(rowKey);
    deletes = next;
  }

  function addDraftRow() {
    inserts = [...inserts, { tempId: crypto.randomUUID(), values: {} }];
  }

  function removeDraftRow(tempId: string) {
    inserts = inserts.filter((d) => d.tempId !== tempId);
  }

  function setDraftValue(tempId: string, colName: string, original: null, text: string) {
    inserts = inserts.map((d) => {
      if (d.tempId !== tempId) return d;
      const values = { ...d.values };
      if (text === "") delete values[colName];
      else values[colName] = coerceLike(original, text);
      return { ...d, values };
    });
  }

  const changeCount = $derived.by(() => {
    let n = deletes.size;
    for (const rowKey of Object.keys(edits)) {
      if (!deletes.has(rowKey) && Object.keys(edits[rowKey]).length > 0) n += 1;
    }
    n += inserts.filter((d) => Object.keys(d.values).length > 0).length;
    return n;
  });

  function buildEdits(): RowEdit[] {
    if (!editable) return [];
    const table = editable.table;
    const out: RowEdit[] = [];
    for (const [rowKey, cols] of Object.entries(edits)) {
      if (deletes.has(rowKey)) continue;
      const set = Object.entries(cols).map(([column, value]) => ({ column, value }));
      if (set.length === 0) continue;
      out.push({ kind: "update", table, pk: pkPredicate(rowKey), set });
    }
    for (const rowKey of deletes) {
      out.push({ kind: "delete", table, pk: pkPredicate(rowKey) });
    }
    for (const draft of inserts) {
      const values = Object.entries(draft.values).map(([column, value]) => ({ column, value }));
      if (values.length === 0) continue;
      out.push({ kind: "insert", table, values });
    }
    return out;
  }

  function discardChanges() {
    edits = {};
    deletes = new Set();
    inserts = [];
    editing = null;
    reviewing = false;
    applyError = null;
  }

  async function openReview() {
    if (!editable || changeCount === 0) return;
    applyError = null;
    try {
      previewSql = await safeInvoke<string[]>("database_client_preview_writes", {
        id: editable.instanceId,
        edits: buildEdits(),
      });
      reviewing = true;
    } catch (err) {
      applyError = err instanceof Error ? err.message : "Could not prepare the changes.";
      reviewing = true;
    }
  }

  async function applyChanges() {
    if (!editable || applying) return;
    applying = true;
    applyError = null;
    try {
      await safeInvoke("database_client_apply_writes", {
        id: editable.instanceId,
        schema: editable.schema ?? null,
        edits: buildEdits(),
      });
      discardChanges();
      onApplied?.();
    } catch (err) {
      applyError = err instanceof Error ? err.message : "The changes could not be applied.";
    } finally {
      applying = false;
    }
  }

  function compareCellValues(left: unknown, right: unknown): number {
    if (left === right) return 0;
    if (left === null || left === undefined) return 1;
    if (right === null || right === undefined) return -1;
    if (typeof left === "number" && typeof right === "number") return left - right;
    return cellText(left).localeCompare(cellText(right), undefined, {
      numeric: true,
      sensitivity: "base",
    });
  }

  function toggleSort(column: string) {
    if (sortColumn === column) {
      sortDirection = sortDirection === "asc" ? "desc" : "asc";
    } else {
      sortColumn = column;
      sortDirection = "asc";
    }
    page = 0;
  }

  function csvCell(value: unknown): string {
    const text = cellText(value);
    return /[",\n\r]/.test(text) ? `"${text.replaceAll('"', '""')}"` : text;
  }

  const filteredRows = $derived.by(() => {
    if (!rows) return [];
    const q = rowFilter.trim().toLowerCase();
    const sortIndex = sortColumn
      ? rows.columns.findIndex((c) => c.name === sortColumn)
      : -1;
    let result = q
      ? rows.rows.filter((row) =>
          row.some((v) => cellText(v).toLowerCase().includes(q)),
        )
      : [...rows.rows];
    if (sortIndex >= 0) {
      result = result.toSorted((a, b) => {
        const cmp = compareCellValues(a[sortIndex], b[sortIndex]);
        return sortDirection === "asc" ? cmp : -cmp;
      });
    }
    return result;
  });

  const pageCount = $derived(Math.max(1, Math.ceil(filteredRows.length / pageSize)));
  const visibleRows = $derived(
    filteredRows.slice(page * pageSize, page * pageSize + pageSize),
  );
  const pageStart = $derived(filteredRows.length === 0 ? 0 : page * pageSize + 1);
  const pageEnd = $derived(Math.min(filteredRows.length, (page + 1) * pageSize));

  $effect(() => {
    if (page >= pageCount) page = Math.max(0, pageCount - 1);
  });

  function exportCsv() {
    if (!rows || rows.columns.length === 0) return;
    const names = rows.columns.map((c) => c.name);
    const csv = [names, ...filteredRows]
      .map((line) => (line as unknown[]).map(csvCell).join(","))
      .join("\n");
    const blob = new Blob([csv], { type: "text/csv;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = `${exportName}.csv`;
    link.click();
    URL.revokeObjectURL(url);
  }
</script>

<div class="h-full flex flex-col min-h-0">
  {#if error}
    <div class="px-4 py-4 text-[12px] text-status-crashed">{error}</div>
  {:else if loading && !rows}
    <p class="px-4 py-4 text-[12px] text-fg-subtle">Loading…</p>
  {:else if !rows}
    <p class="px-4 py-4 text-[12px] text-fg-subtle">{emptyText}</p>
  {:else if rows.columns.length === 0}
    <p class="px-4 py-4 text-[12px] text-fg-subtle">Query returned no rows.</p>
  {:else}
    <!-- Toolbar (does not scroll) -->
    <div
      class="shrink-0 border-b border-border/60 bg-surface px-3 py-2
             flex flex-wrap items-center gap-2"
    >
      {#if title}
        <span class="text-[12px] font-medium text-fg mr-1">{title}</span>
      {/if}
      <label
        class="inline-flex h-8 min-w-[180px] flex-1 items-center gap-2 rounded-md
               border border-border bg-surface-2/50 px-2 text-[11px] text-fg-subtle"
      >
        <Icon name="search" size={11} />
        <input
          value={rowFilter}
          placeholder="Filter rows"
          class="min-w-0 flex-1 bg-transparent text-fg focus:outline-none"
          oninput={(e) => {
            rowFilter = e.currentTarget.value;
            page = 0;
          }}
        />
      </label>
      {#if canEdit}
        <button
          type="button"
          onclick={addDraftRow}
          class="inline-flex h-8 items-center gap-1.5 rounded-md border border-border
                 px-2 text-[11px] text-fg-muted hover:bg-surface-2 hover:text-fg"
        >
          <Icon name="plus" size={11} />
          Add row
        </button>
      {/if}
      <select
        aria-label="Rows per page"
        value={pageSize}
        class="h-8 rounded-md border border-border bg-surface px-2 text-[11px] text-fg-muted"
        onchange={(e) => {
          pageSize = Number(e.currentTarget.value);
          page = 0;
        }}
      >
        {#each pageSizes as sz (sz)}
          <option value={sz}>{sz} rows</option>
        {/each}
      </select>
      <button
        type="button"
        onclick={exportCsv}
        class="inline-flex h-8 items-center gap-1.5 rounded-md border border-border
               px-2 text-[11px] text-fg-muted hover:bg-surface-2 hover:text-fg"
      >
        <Icon name="file-text" size={11} />
        CSV
      </button>
    </div>

    {#if editable && pkColumns.length === 0}
      <div
        class="shrink-0 px-3 py-1.5 text-[11px] text-fg-subtle bg-surface-2/40
               border-b border-border/60"
      >
        This table has no primary key — editing is disabled.
      </div>
    {/if}

    <!-- Main area: table + optional JSON panel -->
    <div class="flex flex-1 min-h-0 overflow-hidden">
      <!-- Scroll region (only rows scroll) -->
      <div class="flex-1 min-w-0 flex flex-col min-h-0 overflow-hidden">
        <div class="flex-1 min-h-0 overflow-auto">
          <table class="min-w-full text-left text-[12px]">
            <thead class="sticky top-0 bg-surface z-10">
              <tr class="border-b border-border/60">
                {#if canEdit}
                  <th class="w-8 px-2 py-2"></th>
                {/if}
                {#each rows.columns as col (col.name)}
                  <th
                    class="px-3 py-2 font-medium text-fg-muted whitespace-nowrap"
                    aria-sort={sortColumn === col.name
                      ? (sortDirection === "asc" ? "ascending" : "descending")
                      : "none"}
                  >
                    <button
                      type="button"
                      onclick={() => toggleSort(col.name)}
                      aria-label={sortColumn === col.name
                        ? `Sort by ${col.name} ${sortDirection === "asc" ? "descending" : "ascending"}`
                        : `Sort by ${col.name} ascending`}
                      class="inline-flex items-center gap-1 rounded px-1 -ml-1
                             hover:bg-surface-2 hover:text-fg"
                    >
                      {#if col.primaryKey}
                        <Icon name="key" size={9} class="text-fg-subtle" />
                      {/if}
                      <span>{col.name}</span>
                      {#if sortColumn === col.name}
                        <Icon
                          name={sortDirection === "asc" ? "chevron-up" : "chevron-down"}
                          size={10}
                        />
                      {:else}
                        <Icon name="chevrons-up-down" size={10} />
                      {/if}
                    </button>
                    {#if col.dataType}
                      <span class="ml-1 font-normal text-fg-subtle text-[10px]">{col.dataType}</span>
                    {/if}
                  </th>
                {/each}
              </tr>
            </thead>
            <tbody>
              {#each visibleRows as row, rowIndex (`${page}-${rowIndex}`)}
                {@const rowKey = canEdit ? rowKeyOf(row) : ""}
                {@const deleted = canEdit && deletes.has(rowKey)}
                <tr
                  class="border-b border-border/30 hover:bg-surface-2/50
                         {deleted ? 'opacity-50' : ''}"
                >
                  {#if canEdit}
                    <td class="w-8 px-2 py-2 align-top">
                      <button
                        type="button"
                        onclick={() => toggleDelete(rowKey)}
                        title={deleted ? "Undo delete" : "Delete row"}
                        aria-label={deleted ? "Undo delete row" : "Delete row"}
                        class="inline-flex items-center justify-center w-5 h-5 rounded
                               text-fg-subtle/60 hover:text-status-crashed hover:bg-surface-2"
                      >
                        <Icon name={deleted ? "rotate-ccw" : "trash-2"} size={11} />
                      </button>
                    </td>
                  {/if}
                  {#each row as value, colIndex (`${rowIndex}-${colIndex}`)}
                    {@const colName = rows.columns[colIndex]?.name ?? ""}
                    {@const shown = canEdit ? displayValue(rowKey, colName, value) : value}
                    {@const editingThis =
                      canEdit && editing?.rowKey === rowKey && editing?.col === colName}
                    <td
                      class="px-3 py-2 max-w-[260px] align-top font-mono text-[11px]
                             {isEdited(rowKey, colName)
                        ? 'text-accent bg-accent/5'
                        : 'text-fg-muted'}
                             {deleted ? 'line-through' : ''}"
                    >
                      {#if editingThis}
                        <input
                          value={editText}
                          use:focusEditor
                          spellcheck={false}
                          oninput={(e) => (editText = e.currentTarget.value)}
                          onblur={() => commitEdit(value)}
                          onkeydown={(e) => onEditorKeydown(e, value)}
                          class="w-full min-w-[120px] rounded border border-accent/60
                                 bg-surface px-1.5 py-1 font-mono text-[11px] text-fg
                                 focus:outline-none focus:ring-1 focus:ring-accent/50"
                        />
                      {:else if canEdit}
                        <button
                          type="button"
                          onclick={() => startEdit(rowKey, colName, value)}
                          disabled={deleted}
                          title="Click to edit"
                          class="block w-full text-left truncate hover:underline
                                 disabled:cursor-not-allowed
                                 {shown === null ? 'italic text-fg-subtle' : ''}"
                        >
                          {cellText(shown)}
                        </button>
                      {:else if isInspectable(value)}
                        <button
                          type="button"
                          onclick={() =>
                            (inspected = {
                              label: rows?.columns[colIndex]?.name ?? "value",
                              value,
                            })}
                          class="block w-full text-left truncate text-accent hover:underline"
                        >
                          {cellText(value)}
                        </button>
                      {:else}
                        <span
                          class:italic={value === null}
                          class:text-fg-subtle={value === null}
                          class="block truncate"
                          title={cellText(value)}
                        >
                          {cellText(value)}
                        </span>
                      {/if}
                    </td>
                  {/each}
                </tr>
              {/each}

              <!-- Draft (insert) rows -->
              {#if canEdit}
                {#each inserts as draft (draft.tempId)}
                  <tr class="border-b border-border/30 bg-status-running/5">
                    <td class="w-8 px-2 py-2 align-top">
                      <button
                        type="button"
                        onclick={() => removeDraftRow(draft.tempId)}
                        title="Discard new row"
                        aria-label="Discard new row"
                        class="inline-flex items-center justify-center w-5 h-5 rounded
                               text-fg-subtle/60 hover:text-status-crashed hover:bg-surface-2"
                      >
                        <Icon name="x" size={11} />
                      </button>
                    </td>
                    {#each rows.columns as col (col.name)}
                      <td class="px-3 py-1.5 max-w-[260px] align-top">
                        <input
                          value={draft.values[col.name] === undefined
                            ? ""
                            : cellText(draft.values[col.name])}
                          placeholder={col.nullable ? "NULL" : col.name}
                          spellcheck={false}
                          oninput={(e) =>
                            setDraftValue(draft.tempId, col.name, null, e.currentTarget.value)}
                          class="w-full min-w-[100px] rounded border border-border
                                 bg-surface px-1.5 py-1 font-mono text-[11px] text-fg
                                 placeholder:text-fg-subtle/60
                                 focus:outline-none focus:ring-1 focus:ring-accent/50"
                        />
                      </td>
                    {/each}
                  </tr>
                {/each}
              {/if}
            </tbody>
          </table>
          {#if filteredRows.length === 0 && inserts.length === 0}
            <p class="px-4 py-4 text-[12px] text-fg-subtle">
              No rows match the current filter.
            </p>
          {/if}
        </div>

        <!-- Review bar (only when there are staged changes) -->
        {#if canEdit && changeCount > 0}
          <div
            class="shrink-0 border-t border-accent/40 bg-accent/5 px-3 py-2 flex flex-wrap
                   items-center justify-between gap-2 text-[11px]"
          >
            <span class="text-fg">
              {changeCount}
              {changeCount === 1 ? "change" : "changes"} staged
            </span>
            <div class="flex items-center gap-2">
              <button
                type="button"
                onclick={discardChanges}
                class="inline-flex h-7 items-center rounded-md border border-border px-2.5
                       text-fg-muted hover:bg-surface-2 hover:text-fg"
              >
                Discard
              </button>
              <button
                type="button"
                onclick={openReview}
                class="inline-flex h-7 items-center gap-1.5 rounded-md bg-accent px-2.5
                       text-on-accent font-medium hover:brightness-110"
              >
                <Icon name="check" size={11} />
                Review &amp; apply
              </button>
            </div>
          </div>
        {/if}

        <!-- Pagination footer (does not scroll) -->
        <div
          class="shrink-0 border-t border-border/60 bg-surface px-3 py-2 flex flex-wrap
                 items-center justify-between gap-2 text-[11px] text-fg-subtle"
        >
          <span>
            Showing {pageStart}–{pageEnd} of {filteredRows.length} rows
            {#if rows.truncated}
              <span class="text-fg-subtle/80"> · refine your query to load more</span>
            {/if}
          </span>
          <div class="flex items-center gap-1">
            <button
              type="button"
              onclick={() => (page = Math.max(0, page - 1))}
              disabled={page === 0}
              aria-label="Previous page"
              class="inline-flex h-7 w-7 items-center justify-center rounded-md
                     border border-border text-fg-muted hover:bg-surface-2 disabled:opacity-40"
            >
              <Icon name="chevron-left" size={12} />
            </button>
            <span class="min-w-16 text-center">{page + 1} / {pageCount}</span>
            <button
              type="button"
              onclick={() => (page = Math.min(pageCount - 1, page + 1))}
              disabled={page >= pageCount - 1}
              aria-label="Next page"
              class="inline-flex h-7 w-7 items-center justify-center rounded-md
                     border border-border text-fg-muted hover:bg-surface-2 disabled:opacity-40"
            >
              <Icon name="chevron-right" size={12} />
            </button>
          </div>
        </div>
      </div>

      <!-- JSON inspector side panel -->
      {#if inspected}
        <aside
          class="w-[300px] shrink-0 border-l border-border/60 bg-surface/40 overflow-auto"
        >
          <div
            class="sticky top-0 bg-surface px-3 py-2 border-b border-border/60
                   flex items-center justify-between gap-2"
          >
            <span class="text-[12px] font-medium text-fg">JSON</span>
            <button
              type="button"
              onclick={() => (inspected = null)}
              title="Close JSON viewer"
              aria-label="Close JSON viewer"
              class="p-1 rounded text-fg-subtle hover:text-fg hover:bg-surface-2"
            >
              <Icon name="x" size={12} />
            </button>
          </div>
          <div class="p-3">
            <JsonTree label={inspected.label} value={inspected.value} />
          </div>
        </aside>
      {/if}
    </div>
  {/if}

  <!-- Review & apply modal -->
  {#if reviewing}
    <div class="fixed inset-0 z-[70] bg-black/40 backdrop-blur-sm" role="presentation"></div>
    <div
      role="dialog"
      aria-modal="true"
      aria-label="Review database changes"
      class="fixed left-1/2 top-1/2 z-[71] w-[min(640px,calc(100vw-2rem))]
             -translate-x-1/2 -translate-y-1/2 rounded-2xl bg-bg border border-border
             shadow-2xl flex flex-col overflow-hidden max-h-[80vh]"
    >
      <div class="px-5 pt-4 pb-3 border-b border-border">
        <h2 class="text-[14px] font-semibold text-fg">Review changes</h2>
        <p class="mt-0.5 text-[11.5px] text-fg-muted">
          {previewSql.length}
          {previewSql.length === 1 ? "statement" : "statements"} will run in one transaction.
        </p>
      </div>

      <div class="px-5 py-3 overflow-y-auto">
        {#if applyError}
          <div
            class="mb-3 rounded-md border border-status-crashed/40 bg-status-crashed/10
                   px-3 py-2 text-[12px] text-status-crashed"
          >
            {applyError}
          </div>
        {/if}
        {#if previewSql.length > 0}
          <pre
            class="w-full overflow-x-auto rounded-lg bg-surface-2 border border-border
                   px-3 py-2.5 text-[12px] font-mono text-fg leading-relaxed whitespace-pre">{previewSql.join(";\n")};</pre>
        {/if}
      </div>

      <footer
        class="px-5 py-3.5 border-t border-border bg-surface/40 flex items-center justify-end gap-2"
      >
        <button
          type="button"
          disabled={applying}
          onclick={() => (reviewing = false)}
          class="h-8 px-3.5 rounded-md text-[12px] font-medium text-fg border border-border
                 hover:bg-surface-2 disabled:opacity-50"
        >
          Cancel
        </button>
        <button
          type="button"
          disabled={applying || previewSql.length === 0}
          onclick={applyChanges}
          class="h-8 px-3.5 rounded-md text-[12px] font-medium text-on-accent bg-accent
                 hover:brightness-110 disabled:opacity-50 disabled:cursor-not-allowed transition
                 inline-flex items-center gap-1.5"
        >
          {#if applying}
            <Icon name="refresh-cw" size={12} class="animate-spin" />
          {/if}
          Apply changes
        </button>
      </footer>
    </div>
  {/if}
</div>
