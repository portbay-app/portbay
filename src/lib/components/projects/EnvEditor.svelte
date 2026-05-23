<!--
  EnvEditor — per-project environment variable table.

  Owns its own dirty state + save cycle. Saves through `update_project`
  with `{ env: { ... } }` (the Rust patch struct already supports it).
  Env changes don't take effect until the project's next start, so we
  surface a small restart nudge when dirty rows exist alongside a
  running project.

  Sensitive-named values (matching `secret|key|token|password` case-
  insensitively in the key) render masked by default with a per-row
  reveal toggle. Bulk paste accepts multi-line `KEY=value` input and
  splits into rows. `.env` import reads a user-picked file through
  the existing dialog plugin and parses the same way.
-->
<script lang="ts">
  import { open as openDialog } from "@tauri-apps/plugin-dialog";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import { ErrorEnvelope } from "$lib/components/errors";
  import { safeInvoke } from "$lib/ipc";
  import { errorBus } from "$lib/stores/errors.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import type { CommandError } from "$lib/types/error";
  import type { ProjectView } from "$lib/types/projects";

  interface Props {
    project: ProjectView;
  }
  let { project }: Props = $props();

  /** Row id used for {#each} keying — env order is irrelevant for
   *  semantics, but stable keys matter for input focus during edits. */
  interface Row {
    rid: number;
    key: string;
    value: string;
    revealed: boolean;
  }

  let rows = $state<Row[]>([]);
  let saving = $state<boolean>(false);
  let error = $state<CommandError | null>(null);
  let pasteMode = $state<boolean>(false);
  let pasteText = $state<string>("");
  let nextRid = 1;

  /** Pattern to mask values whose key looks sensitive. */
  const SENSITIVE_KEY = /(secret|key|token|password|passwd|pwd)/i;

  function freshRow(key = "", value = ""): Row {
    return { rid: nextRid++, key, value, revealed: false };
  }

  function syncFromProject() {
    rows = Object.entries(project.env ?? {})
      .sort(([a], [b]) => a.localeCompare(b))
      .map(([k, v]) => freshRow(k, v));
    error = null;
    pasteMode = false;
    pasteText = "";
  }

  // Re-sync whenever the selected project changes.
  $effect(() => {
    // Tracked deps: project.id + project.env reference.
    const _id = project.id;
    syncFromProject();
  });

  /** True when any local edit differs from what the registry currently holds. */
  const dirty = $derived.by(() => {
    const current: Record<string, string> = {};
    for (const r of rows) {
      const k = r.key.trim();
      if (!k) continue;
      current[k] = r.value;
    }
    const original = project.env ?? {};
    const ckeys = Object.keys(current);
    const okeys = Object.keys(original);
    if (ckeys.length !== okeys.length) return true;
    for (const k of ckeys) {
      if (original[k] !== current[k]) return true;
    }
    return false;
  });

  /** True when the project is currently running and env edits are
   *  staged — used to show the "Restart to apply" nudge. */
  const restartNeeded = $derived(dirty && project.status === "running");

  function addRow() {
    rows = [...rows, freshRow()];
  }

  function removeRow(rid: number) {
    rows = rows.filter((r) => r.rid !== rid);
  }

  function toggleReveal(rid: number) {
    rows = rows.map((r) => (r.rid === rid ? { ...r, revealed: !r.revealed } : r));
  }

  /** Parse `KEY=value` lines into rows. Lines starting with `#` and
   *  blank lines are skipped. Quoted values are unwrapped. */
  function parseDotenv(text: string): Array<[string, string]> {
    const out: Array<[string, string]> = [];
    for (const raw of text.split(/\r?\n/)) {
      const line = raw.trim();
      if (!line || line.startsWith("#")) continue;
      const eq = line.indexOf("=");
      if (eq <= 0) continue;
      const key = line.slice(0, eq).trim();
      let value = line.slice(eq + 1).trim();
      // Strip surrounding single/double quotes if matched on both ends.
      if (
        (value.startsWith('"') && value.endsWith('"')) ||
        (value.startsWith("'") && value.endsWith("'"))
      ) {
        value = value.slice(1, -1);
      }
      if (key) out.push([key, value]);
    }
    return out;
  }

  function applyPaste() {
    const parsed = parseDotenv(pasteText);
    if (parsed.length === 0) {
      pasteMode = false;
      pasteText = "";
      return;
    }
    // Merge with existing rows: parsed entries replace matching keys,
    // new keys append.
    const byKey = new Map<string, Row>();
    for (const r of rows) {
      const k = r.key.trim();
      if (k) byKey.set(k, r);
    }
    for (const [k, v] of parsed) {
      const existing = byKey.get(k);
      if (existing) {
        byKey.set(k, { ...existing, value: v });
      } else {
        byKey.set(k, freshRow(k, v));
      }
    }
    rows = Array.from(byKey.values()).sort((a, b) =>
      a.key.localeCompare(b.key),
    );
    pasteMode = false;
    pasteText = "";
  }

  async function importDotenvFile() {
    try {
      const picked = await openDialog({
        directory: false,
        multiple: false,
        title: "Pick a .env file",
        filters: [{ name: ".env files", extensions: ["env", "*"] }],
      });
      if (typeof picked !== "string") return;
      // The backend parses + size-caps; we merge the result into rows
      // straight away (no intermediate paste step needed).
      const parsed = await safeInvoke<Array<[string, string]>>("read_dotenv", {
        path: picked,
      });
      if (parsed.length === 0) {
        errorBus.push({
          code: "DOTENV_EMPTY",
          whatHappened: "No KEY=value pairs found in that file.",
          whyItMatters: "Pick a different file or use bulk paste.",
          whoCausedIt: "user",
          actions: [],
        });
        return;
      }
      const byKey = new Map<string, Row>();
      for (const r of rows) {
        const k = r.key.trim();
        if (k) byKey.set(k, r);
      }
      for (const [k, v] of parsed) {
        const existing = byKey.get(k);
        byKey.set(k, existing ? { ...existing, value: v } : freshRow(k, v));
      }
      rows = Array.from(byKey.values()).sort((a, b) =>
        a.key.localeCompare(b.key),
      );
    } catch {
      /* safeInvoke already toasted the envelope */
    }
  }

  async function save() {
    if (!dirty) return;
    saving = true;
    error = null;
    // Build a clean env object — drop empty-key rows, last-write-wins
    // on duplicate keys.
    const env: Record<string, string> = {};
    for (const r of rows) {
      const k = r.key.trim();
      if (!k) continue;
      env[k] = r.value;
    }
    try {
      await safeInvoke<ProjectView>("update_project", {
        id: project.id,
        patch: { env },
      });
      await projects.refresh();
      errorBus.push({
        code: "ENV_SAVED",
        whatHappened: `Environment updated for ${project.name}.`,
        whyItMatters: restartNeeded
          ? "Restart the project for changes to take effect."
          : "Next start will use the new values.",
        whoCausedIt: "system",
        actions: [],
      });
    } catch (e) {
      error = e as CommandError;
    } finally {
      saving = false;
    }
  }

  function discard() {
    syncFromProject();
  }

  async function restartProject() {
    try {
      await safeInvoke("restart_project", { id: project.id });
    } catch {
      /* toast already pushed */
    }
  }
</script>

<div class="space-y-3">
  {#if pasteMode}
    <div class="rounded-md border border-border bg-bg p-3">
      <div class="flex items-center justify-between mb-2">
        <span class="text-xs text-fg-muted">
          Paste <code>KEY=value</code> lines, one per row.
        </span>
        <div class="flex items-center gap-1">
          <button
            type="button"
            onclick={() => {
              pasteMode = false;
              pasteText = "";
            }}
            class="text-xs text-fg-subtle hover:text-fg px-2 py-1"
          >
            Cancel
          </button>
          <button
            type="button"
            onclick={applyPaste}
            disabled={!pasteText.trim()}
            class="inline-flex items-center gap-1 text-xs text-accent
                   border border-accent/40 hover:bg-accent/10 disabled:opacity-50
                   rounded-md px-2 py-1 transition-colors"
          >
            <Icon name="check" size={11} />
            Apply
          </button>
        </div>
      </div>
      <textarea
        bind:value={pasteText}
        rows="6"
        spellcheck="false"
        placeholder="DATABASE_URL=postgres://localhost/myapp&#10;API_KEY=…"
        class="w-full font-mono text-xs px-2 py-1.5 rounded-md
               bg-surface border border-border outline-none
               focus:border-accent/60 resize-y"
      ></textarea>
    </div>
  {/if}

  {#if rows.length === 0 && !pasteMode}
    <div
      class="rounded-md border border-dashed border-border p-4 text-center text-xs text-fg-muted"
    >
      No environment variables yet. Add one, paste a block, or import a
      <code>.env</code> file.
    </div>
  {/if}

  {#if rows.length > 0}
    <div class="rounded-md border border-border overflow-hidden">
      <div
        class="grid grid-cols-[1fr,1.5fr,auto] gap-px bg-border text-[11px]
               uppercase tracking-wide text-fg-subtle"
      >
        <div class="bg-surface-2 px-2.5 py-1.5">Name</div>
        <div class="bg-surface-2 px-2.5 py-1.5">Value</div>
        <div class="bg-surface-2 px-2.5 py-1.5"></div>
      </div>
      {#each rows as row (row.rid)}
        {@const sensitive = SENSITIVE_KEY.test(row.key)}
        <div
          class="grid grid-cols-[1fr,1.5fr,auto] gap-px bg-border"
        >
          <div class="bg-surface">
            <input
              type="text"
              bind:value={row.key}
              placeholder="KEY"
              spellcheck="false"
              class="w-full px-2.5 py-1.5 font-mono text-xs bg-transparent
                     outline-none focus:bg-surface-2"
            />
          </div>
          <div class="bg-surface">
            <input
              type={sensitive && !row.revealed ? "password" : "text"}
              bind:value={row.value}
              placeholder="value"
              spellcheck="false"
              class="w-full px-2.5 py-1.5 font-mono text-xs bg-transparent
                     outline-none focus:bg-surface-2"
            />
          </div>
          <div class="bg-surface flex items-center gap-0.5 pr-1">
            {#if sensitive}
              <button
                type="button"
                onclick={() => toggleReveal(row.rid)}
                title={row.revealed ? "Mask value" : "Reveal value"}
                aria-label={row.revealed ? "Mask value" : "Reveal value"}
                class="p-1.5 rounded-md text-fg-subtle hover:text-fg hover:bg-surface-2"
              >
                <Icon name={row.revealed ? "x" : "info"} size={12} />
              </button>
            {/if}
            <button
              type="button"
              onclick={() => removeRow(row.rid)}
              title="Remove row"
              aria-label="Remove row"
              class="p-1.5 rounded-md text-fg-subtle hover:text-status-crashed hover:bg-surface-2"
            >
              <Icon name="x" size={12} />
            </button>
          </div>
        </div>
      {/each}
    </div>
  {/if}

  <div class="flex items-center justify-between gap-2 flex-wrap">
    <div class="flex items-center gap-1.5">
      <button
        type="button"
        onclick={addRow}
        class="inline-flex items-center gap-1 text-xs text-fg-muted
               border border-border hover:text-fg hover:bg-surface-2
               rounded-md px-2 py-1 transition-colors"
      >
        <Icon name="plus" size={11} /> Add row
      </button>
      <button
        type="button"
        onclick={() => (pasteMode = true)}
        class="inline-flex items-center gap-1 text-xs text-fg-muted
               border border-border hover:text-fg hover:bg-surface-2
               rounded-md px-2 py-1 transition-colors"
      >
        <Icon name="file-text" size={11} /> Bulk paste
      </button>
      <button
        type="button"
        onclick={importDotenvFile}
        class="inline-flex items-center gap-1 text-xs text-fg-muted
               border border-border hover:text-fg hover:bg-surface-2
               rounded-md px-2 py-1 transition-colors"
      >
        <Icon name="folder" size={11} /> Import .env
      </button>
    </div>
    {#if dirty}
      <div class="flex items-center gap-1.5">
        <button
          type="button"
          onclick={discard}
          class="px-2.5 py-1 text-xs rounded-md text-fg-muted hover:text-fg
                 hover:bg-surface-2 transition-colors"
        >
          Discard
        </button>
        <button
          type="button"
          onclick={save}
          disabled={saving}
          class="inline-flex items-center gap-1.5 px-2.5 py-1 text-xs
                 rounded-md text-accent border border-accent/40
                 hover:bg-accent/10 disabled:opacity-50 transition-colors"
        >
          {#if saving}
            <Icon name="refresh-cw" size={11} class="animate-spin" />
            Saving…
          {:else}
            <Icon name="check" size={11} />
            Save environment
          {/if}
        </button>
      </div>
    {/if}
  </div>

  {#if restartNeeded}
    <div
      class="flex items-center justify-between gap-2 px-3 py-2 rounded-md
             border border-status-unhealthy/40 bg-status-unhealthy/5"
    >
      <span class="text-xs text-fg-muted">
        Saved changes won't be visible until the project restarts.
      </span>
      <button
        type="button"
        onclick={restartProject}
        class="inline-flex items-center gap-1 text-xs text-status-unhealthy
               hover:bg-status-unhealthy/10 rounded-md px-2 py-1 transition-colors"
      >
        <Icon name="rotate-cw" size={11} />
        Restart now
      </button>
    </div>
  {/if}

  {#if error}
    <ErrorEnvelope envelope={error} tone="inline" />
  {/if}
</div>
