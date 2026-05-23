<!--
  AdvancedFields — exposes the lesser-used Project fields the
  Configuration card doesn't carry: tags, extra ports, services
  (per-project sidecar enablement), and PHP-specific document
  root + php_version.

  Saves through update_project the same way the Environment editor
  does. Each field area has its own dirty cycle so the user can
  edit one thing without touching another.
-->
<script lang="ts">
  import { onMount } from "svelte";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import { ErrorEnvelope } from "$lib/components/errors";
  import { safeInvoke } from "$lib/ipc";
  import { errorBus } from "$lib/stores/errors.svelte";
  import { php } from "$lib/stores/php.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import type { CommandError } from "$lib/types/error";
  import type { ProjectView } from "$lib/types/projects";

  interface Props {
    project: ProjectView;
  }
  let { project }: Props = $props();

  /** Services that PortBay knows how to wire into a project. The
   *  registry stores these as opaque strings; this set controls the
   *  multi-select offered by the UI. Add new entries here when the
   *  reconcile loop learns to wire them. */
  const KNOWN_SERVICES = ["caddy", "mailpit", "mysql", "postgres", "redis"];

  /** Versions PortBay knows about (Homebrew formula coverage). The
   *  picker marks any of these that aren't actually installed with a
   *  warning dot so the user sees they need to brew-install first. */
  const KNOWN_PHP_VERSIONS = ["7.4", "8.0", "8.1", "8.2", "8.3", "8.4"];

  onMount(() => {
    void php.refresh();
  });

  // ────────── Tags ──────────
  let tagsDraft = $state<string[]>([]);
  let newTag = $state<string>("");

  // ────────── Extra ports ──────────
  let extraPortsDraft = $state<string>(""); // comma-separated for easy editing

  // ────────── Services ──────────
  let servicesDraft = $state<string[]>([]);
  let serviceCustom = $state<string>("");

  // ────────── PHP-only ──────────
  let documentRootDraft = $state<string>("");
  let phpVersionDraft = $state<string>("");

  // Save state — shared across sub-sections.
  let saving = $state<boolean>(false);
  let error = $state<CommandError | null>(null);

  function syncFromProject() {
    tagsDraft = [...(project.tags ?? [])];
    newTag = "";
    extraPortsDraft = (project.extraPorts ?? []).join(", ");
    servicesDraft = [...(project.services ?? [])];
    serviceCustom = "";
    documentRootDraft = project.documentRoot ?? "";
    phpVersionDraft = project.phpVersion ?? "";
    error = null;
  }

  $effect(() => {
    const _id = project.id;
    syncFromProject();
  });

  // ────────── Dirty derivations ──────────
  const tagsDirty = $derived.by(() => {
    const original = project.tags ?? [];
    if (tagsDraft.length !== original.length) return true;
    return tagsDraft.some((t, i) => t !== original[i]);
  });

  const extraPortsParsed = $derived.by<number[] | null>(() => {
    const trimmed = extraPortsDraft.trim();
    if (!trimmed) return [];
    const parts = trimmed.split(/[,\s]+/).filter(Boolean);
    const nums: number[] = [];
    for (const p of parts) {
      const n = Number(p);
      if (!Number.isInteger(n) || n < 1 || n > 65535) return null;
      nums.push(n);
    }
    return nums;
  });

  const extraPortsDirty = $derived.by(() => {
    if (extraPortsParsed === null) return false;
    const original = project.extraPorts ?? [];
    if (extraPortsParsed.length !== original.length) return true;
    return extraPortsParsed.some((p, i) => p !== original[i]);
  });

  const servicesDirty = $derived.by(() => {
    const original = project.services ?? [];
    const a = [...servicesDraft].sort();
    const b = [...original].sort();
    if (a.length !== b.length) return true;
    return a.some((s, i) => s !== b[i]);
  });

  const phpDirty = $derived(
    documentRootDraft !== (project.documentRoot ?? "") ||
      phpVersionDraft !== (project.phpVersion ?? ""),
  );

  const isPhp = $derived(project.type === "php");

  const anyDirty = $derived(
    tagsDirty || extraPortsDirty || servicesDirty || (isPhp && phpDirty),
  );

  // ────────── Tag actions ──────────
  function addTag() {
    const t = newTag.trim();
    if (!t) return;
    if (tagsDraft.includes(t)) {
      newTag = "";
      return;
    }
    tagsDraft = [...tagsDraft, t];
    newTag = "";
  }

  function removeTag(t: string) {
    tagsDraft = tagsDraft.filter((x) => x !== t);
  }

  // ────────── Service toggle ──────────
  function toggleService(s: string) {
    if (servicesDraft.includes(s)) {
      servicesDraft = servicesDraft.filter((x) => x !== s);
    } else {
      servicesDraft = [...servicesDraft, s];
    }
  }

  function addCustomService() {
    const s = serviceCustom.trim();
    if (!s || servicesDraft.includes(s)) {
      serviceCustom = "";
      return;
    }
    servicesDraft = [...servicesDraft, s];
    serviceCustom = "";
  }

  // ────────── Save / discard ──────────
  async function save() {
    if (!anyDirty || extraPortsParsed === null) return;
    saving = true;
    error = null;

    const patch: Record<string, unknown> = {};
    if (tagsDirty) patch.tags = tagsDraft;
    if (extraPortsDirty) patch.extraPorts = extraPortsParsed;
    if (servicesDirty) patch.services = servicesDraft;
    if (isPhp && phpDirty) {
      patch.documentRoot = documentRootDraft.trim();
      patch.phpVersion = phpVersionDraft.trim();
    }

    try {
      await safeInvoke<ProjectView>("update_project", {
        id: project.id,
        patch,
      });
      await projects.refresh();
      errorBus.push({
        code: "ADVANCED_SAVED",
        whatHappened: `${project.name} updated.`,
        whyItMatters: "Restart the project for changes to take effect.",
        whoCausedIt: "system",
        severity: "success",
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
</script>

<div class="space-y-5">
  <!-- Tags -->
  <section class="space-y-2">
    <div class="flex items-center justify-between">
      <span class="text-xs uppercase tracking-wide text-fg-subtle">Tags</span>
      {#if tagsDraft.length > 0}
        <span class="text-[10px] text-fg-subtle">{tagsDraft.length}</span>
      {/if}
    </div>
    <div class="flex flex-wrap gap-1.5">
      {#each tagsDraft as tag (tag)}
        <span
          class="inline-flex items-center gap-1 pl-2 pr-1 py-0.5 rounded-md
                 bg-surface-2 text-xs text-fg-muted border border-border"
        >
          {tag}
          <button
            type="button"
            onclick={() => removeTag(tag)}
            aria-label="Remove tag {tag}"
            class="p-0.5 rounded hover:bg-surface text-fg-subtle hover:text-status-crashed"
          >
            <Icon name="x" size={10} />
          </button>
        </span>
      {/each}
      <div class="flex items-center gap-1">
        <input
          type="text"
          bind:value={newTag}
          onkeydown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              addTag();
            }
          }}
          placeholder="add tag…"
          class="px-2 py-0.5 text-xs rounded-md bg-bg border border-border
                 focus:border-accent/60 outline-none w-32"
        />
        {#if newTag.trim()}
          <button
            type="button"
            onclick={addTag}
            class="p-1 rounded-md text-accent hover:bg-accent/10"
            aria-label="Add tag"
          >
            <Icon name="plus" size={11} />
          </button>
        {/if}
      </div>
    </div>
    <p class="text-[10px] text-fg-subtle">
      Free-form labels for grouping. Press Enter to add.
    </p>
  </section>

  <!-- Extra ports -->
  <section class="space-y-2">
    <span class="text-xs uppercase tracking-wide text-fg-subtle">Extra ports</span>
    <input
      type="text"
      bind:value={extraPortsDraft}
      placeholder="24678, 9000"
      spellcheck="false"
      class="w-full px-2.5 py-1.5 rounded-md bg-bg border border-border
             focus:border-accent/60 outline-none text-fg font-mono text-xs"
      class:border-status-crashed={extraPortsParsed === null}
    />
    <p class="text-[10px] text-fg-subtle">
      Additional ports Caddy should route to this project. Comma-separated.
      {#if extraPortsParsed === null}
        <span class="text-status-crashed">
          Must be integers between 1 and 65535.
        </span>
      {/if}
    </p>
  </section>

  <!-- Services -->
  <section class="space-y-2">
    <span class="text-xs uppercase tracking-wide text-fg-subtle">Services</span>
    <div class="flex flex-wrap gap-1.5">
      {#each KNOWN_SERVICES as svc (svc)}
        {@const on = servicesDraft.includes(svc)}
        <button
          type="button"
          onclick={() => toggleService(svc)}
          class="inline-flex items-center gap-1 px-2 py-0.5 rounded-md text-xs
                 border transition-colors"
          class:bg-accent={on}
          class:text-on-accent={on}
          class:border-accent={on}
          class:bg-surface-2={!on}
          class:text-fg-muted={!on}
          class:border-border={!on}
        >
          {#if on}
            <Icon name="check" size={10} />
          {/if}
          {svc}
        </button>
      {/each}
      {#each servicesDraft.filter((s) => !KNOWN_SERVICES.includes(s)) as svc (svc)}
        <span
          class="inline-flex items-center gap-1 pl-2 pr-1 py-0.5 rounded-md
                 bg-accent text-on-accent text-xs"
        >
          <Icon name="check" size={10} />
          {svc}
          <button
            type="button"
            onclick={() => toggleService(svc)}
            aria-label="Remove custom service {svc}"
            class="p-0.5 rounded hover:bg-white/20"
          >
            <Icon name="x" size={10} />
          </button>
        </span>
      {/each}
    </div>
    <div class="flex items-center gap-1.5">
      <input
        type="text"
        bind:value={serviceCustom}
        onkeydown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            addCustomService();
          }
        }}
        placeholder="custom service id…"
        class="flex-1 px-2 py-0.5 text-xs rounded-md bg-bg border border-border
               focus:border-accent/60 outline-none"
      />
      {#if serviceCustom.trim()}
        <button
          type="button"
          onclick={addCustomService}
          class="p-1 rounded-md text-accent hover:bg-accent/10"
          aria-label="Add custom service"
        >
          <Icon name="plus" size={11} />
        </button>
      {/if}
    </div>
    <p class="text-[10px] text-fg-subtle">
      Per-project sidecars to wire (e.g. <code>mysql</code> if the project
      depends on a local database).
    </p>
  </section>

  {#if isPhp}
    <section class="space-y-2">
      <span class="text-xs uppercase tracking-wide text-fg-subtle">PHP</span>
      <div class="grid grid-cols-[110px,1fr] gap-x-3 gap-y-2 items-center text-sm">
        <label for="advanced-doc-root" class="text-fg-muted">Document root</label>
        <input
          id="advanced-doc-root"
          type="text"
          bind:value={documentRootDraft}
          placeholder="public"
          spellcheck="false"
          class="px-2.5 py-1.5 rounded-md bg-bg border border-border
                 focus:border-accent/60 outline-none text-fg font-mono text-xs"
        />
        <label for="advanced-php-ver" class="text-fg-muted">PHP version</label>
        <div class="flex flex-wrap gap-1.5 items-center">
          {#each KNOWN_PHP_VERSIONS as v (v)}
            {@const on = phpVersionDraft === v}
            {@const installed = php.isInstalled(v)}
            <button
              type="button"
              onclick={() => (phpVersionDraft = v)}
              title={installed
                ? `PHP ${v} detected`
                : `Not installed — run brew install php@${v}`}
              class="inline-flex items-center gap-1 px-2 py-0.5 rounded-md text-xs border transition-colors"
              class:bg-accent={on}
              class:text-on-accent={on}
              class:border-accent={on}
              class:bg-surface-2={!on}
              class:text-fg-muted={!on}
              class:border-border={!on}
              class:opacity-60={!installed && !on}
            >
              {#if !installed}
                <Icon name="info" size={10} />
              {/if}
              {v}
            </button>
          {/each}
          <input
            id="advanced-php-ver"
            type="text"
            bind:value={phpVersionDraft}
            placeholder="custom"
            spellcheck="false"
            class="px-2 py-0.5 text-xs rounded-md bg-bg border border-border
                   focus:border-accent/60 outline-none w-24 font-mono"
          />
        </div>
        {#if phpVersionDraft && !php.isInstalled(phpVersionDraft)}
          <span></span>
          <p class="text-[11px] text-status-unhealthy">
            PHP {phpVersionDraft} isn't installed. Run
            <code class="font-mono">brew install php@{phpVersionDraft}</code>
            then re-detect from the PHP panel.
          </p>
        {/if}
      </div>
      <p class="text-[10px] text-fg-subtle">
        Document root is the subfolder Caddy serves (typically
        <code>public</code> for Laravel). PHP version selects which PHP-FPM
        binary handles requests.
      </p>
    </section>
  {/if}

  {#if anyDirty}
    <div
      class="flex items-center justify-end gap-2 pt-2 border-t border-border"
    >
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
        disabled={saving || extraPortsParsed === null}
        class="inline-flex items-center gap-1.5 px-2.5 py-1 text-xs
               rounded-md text-accent border border-accent/40
               hover:bg-accent/10 disabled:opacity-50 transition-colors"
      >
        {#if saving}
          <Icon name="refresh-cw" size={11} class="animate-spin" /> Saving…
        {:else}
          <Icon name="check" size={11} /> Save advanced
        {/if}
      </button>
    </div>
  {/if}

  {#if error}
    <ErrorEnvelope envelope={error} tone="inline" />
  {/if}
</div>
