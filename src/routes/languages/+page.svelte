<!--
  /languages — detect-first runtime container.

  Two-pane layout matching the ServBay reference:
    - Left sub-sidebar (240px): grouped by language, one row per
      detected version. Status dot, version label, install-source
      pill. Languages with zero detected versions show an inline
      "Install via Homebrew" hint.
    - Right pane (1fr): selected version's config panel — a header
      strip with version + source pill + binary path, then declarative
      tabs (FPM / PHP / Extensions for PHP; Info-only for others).

  v1 is detection-only — no install flow, no per-version config
  editing. Those land in follow-up commits on the same kanban card.
-->
<script lang="ts">
  import { onMount } from "svelte";

  import { Icon } from "$lib/components/atoms";
  import { safeInvoke } from "$lib/ipc";
  import type {
    LanguageView,
    VersionView,
    InstallSource,
  } from "$lib/types/runtimes";
  import { sourceLabel } from "$lib/types/runtimes";

  let languages = $state<LanguageView[]>([]);
  let loading = $state<boolean>(true);

  /**
   * Selection state. The user picks `<langId>:<version>` and the
   * right pane renders that version's tabs. `selected` is the
   * id of the active tab inside that version. Both default to the
   * first available value on initial load.
   */
  let selectedKey = $state<string | null>(null);
  let activeTab = $state<string | null>(null);
  let copiedHint = $state<string | null>(null);

  onMount(() => {
    void refresh();
  });

  async function refresh() {
    loading = true;
    try {
      languages = await safeInvoke<LanguageView[]>("list_runtimes");
      if (!selectedKey) {
        // Auto-select the first detected version across all languages,
        // so the right pane is never empty when at least one runtime
        // exists on the machine.
        for (const lang of languages) {
          if (lang.versions.length > 0) {
            selectedKey = `${lang.id}:${lang.versions[0].install.version}`;
            activeTab = lang.versions[0].tabs[0]?.id ?? null;
            break;
          }
        }
      }
    } finally {
      loading = false;
    }
  }

  function selectVersion(langId: string, version: VersionView) {
    selectedKey = `${langId}:${version.install.version}`;
    activeTab = version.tabs[0]?.id ?? null;
  }

  function findSelected():
    | { lang: LanguageView; version: VersionView }
    | null {
    if (!selectedKey) return null;
    const [langId, ver] = selectedKey.split(":");
    const lang = languages.find((l) => l.id === langId);
    if (!lang) return null;
    const version = lang.versions.find((v) => v.install.version === ver);
    if (!version) return null;
    return { lang, version };
  }

  const selected = $derived(findSelected());

  /** Pill colour by source — matches the install-source semantics. */
  const sourceClass: Record<InstallSource, string> = {
    homebrew: "bg-amber-500/15 text-amber-300 border-amber-500/30",
    asdf: "bg-violet-500/15 text-violet-300 border-violet-500/30",
    mise: "bg-emerald-500/15 text-emerald-300 border-emerald-500/30",
    nvm: "bg-cyan-500/15 text-cyan-300 border-cyan-500/30",
    pyenv: "bg-blue-500/15 text-blue-300 border-blue-500/30",
    system: "bg-fg-subtle/15 text-fg-subtle border-fg-subtle/30",
  };

  async function copyHint(hint: string) {
    try {
      await navigator.clipboard.writeText(hint);
      copiedHint = hint;
      setTimeout(() => {
        if (copiedHint === hint) copiedHint = null;
      }, 1500);
    } catch {
      /* clipboard unavailable */
    }
  }

  function copyPath(value: string) {
    void copyHint(value);
  }
</script>

<div class="h-full flex">
  <!-- Sub-sidebar: grouped languages -->
  <aside
    class="w-60 shrink-0 border-r border-border bg-surface/40 overflow-y-auto"
    aria-label="Languages"
  >
    <div class="px-4 pt-4 pb-2 flex items-center justify-between">
      <h2 class="text-[10px] font-mono uppercase tracking-wider text-fg-subtle">
        Languages
      </h2>
      <button
        type="button"
        onclick={refresh}
        disabled={loading}
        class="text-fg-muted hover:text-fg disabled:opacity-40 p-0.5 rounded"
        title="Rescan installed runtimes"
        aria-label="Rescan installed runtimes"
      >
        <Icon name="refresh-cw" size={11} />
      </button>
    </div>

    {#if loading && languages.length === 0}
      <div class="px-4 py-6 text-xs text-fg-subtle">Detecting runtimes…</div>
    {:else}
      <div class="px-2 pb-6 space-y-2">
        {#each languages as lang (lang.id)}
          <div>
            <div
              class="px-2 py-1 text-[11px] font-semibold uppercase tracking-wider text-fg-muted flex items-center gap-1.5"
            >
              <span
                class="inline-block h-1.5 w-1.5 rounded-full
                       {lang.versions.length > 0
                  ? 'bg-status-running'
                  : 'bg-fg-subtle/40'}"
                aria-hidden="true"
              ></span>
              {lang.displayName}
              {#if lang.versions.length > 0}
                <span class="ml-auto text-fg-subtle font-mono normal-case">
                  {lang.versions.length}
                </span>
              {/if}
            </div>

            {#if lang.versions.length === 0}
              <button
                type="button"
                onclick={() => copyHint(lang.installHint)}
                class="w-full text-left px-2 py-1.5 ml-2 rounded text-[11px]
                       text-fg-subtle hover:text-fg-muted hover:bg-surface-2/60
                       border border-dashed border-border/60
                       transition-colors group"
                title="Click to copy"
              >
                <span class="font-mono">
                  {copiedHint === lang.installHint ? "Copied!" : lang.installHint}
                </span>
              </button>
            {:else}
              {#each lang.versions as version (version.install.version)}
                {@const key = `${lang.id}:${version.install.version}`}
                {@const isActive = selectedKey === key}
                <button
                  type="button"
                  onclick={() => selectVersion(lang.id, version)}
                  class="w-full flex items-center gap-2 px-2 py-1.5 rounded-md
                         text-left transition-colors
                         {isActive
                    ? 'bg-accent/15 text-fg'
                    : 'text-fg-muted hover:text-fg hover:bg-surface-2/40'}"
                >
                  <span
                    class="inline-block h-1.5 w-1.5 rounded-full
                           {isActive
                      ? 'bg-status-running'
                      : 'bg-fg-subtle/60'}"
                    aria-hidden="true"
                  ></span>
                  <span class="text-xs font-medium flex-1 min-w-0 truncate">
                    {lang.displayName}
                    {version.install.version}
                  </span>
                  <span
                    class="text-[9px] font-mono px-1.5 py-0.5 rounded border
                           {sourceClass[version.install.source]}"
                  >
                    {sourceLabel[version.install.source]}
                  </span>
                </button>
              {/each}
            {/if}
          </div>
        {/each}
      </div>
    {/if}
  </aside>

  <!-- Right pane: config for the selected version -->
  <section class="flex-1 min-w-0 overflow-y-auto">
    {#if !selected}
      <div class="h-full flex items-center justify-center">
        <div class="text-center max-w-sm px-6">
          <div class="text-sm text-fg-muted">
            {#if languages.every((l) => l.versions.length === 0)}
              No runtimes detected on this machine.
              <div class="mt-2 text-xs text-fg-subtle">
                Install one via Homebrew, asdf, mise, nvm, or pyenv —
                PortBay will pick it up on the next rescan.
              </div>
            {:else}
              Select a version from the sidebar to view its configuration.
            {/if}
          </div>
        </div>
      </div>
    {:else}
      {@const { lang, version } = selected}
      <!-- Header strip -->
      <header class="px-6 pt-6 pb-4 border-b border-border">
        <div class="flex items-center gap-3">
          <span
            class="inline-block h-2 w-2 rounded-full bg-status-running"
            aria-hidden="true"
          ></span>
          <h1 class="text-lg font-semibold text-fg">
            {lang.displayName}
            {version.install.version}
          </h1>
          <span
            class="text-[10px] font-mono px-1.5 py-0.5 rounded border
                   {sourceClass[version.install.source]}"
          >
            {sourceLabel[version.install.source]}
          </span>
        </div>
        <button
          type="button"
          onclick={() => copyPath(version.install.binary)}
          class="mt-2 text-[11px] font-mono text-fg-subtle hover:text-fg-muted
                 transition-colors text-left"
          title="Click to copy path"
        >
          {copiedHint === version.install.binary ? "Copied!" : version.install.binary}
        </button>
      </header>

      <!-- Tabs -->
      {#if version.tabs.length > 0}
        <div
          class="px-6 border-b border-border flex gap-1"
          role="tablist"
          aria-label="Configuration tabs"
        >
          {#each version.tabs as tab (tab.id)}
            {@const isActive = activeTab === tab.id}
            <button
              type="button"
              role="tab"
              aria-selected={isActive}
              onclick={() => (activeTab = tab.id)}
              class="px-3 py-2 text-xs font-medium border-b-2 transition-colors
                     {isActive
                ? 'border-accent text-fg'
                : 'border-transparent text-fg-muted hover:text-fg'}"
            >
              {tab.label}
            </button>
          {/each}
        </div>

        <!-- Active tab content -->
        {#each version.tabs as tab (tab.id)}
          {#if activeTab === tab.id}
            <div class="px-6 py-5">
              {#if tab.rows.length === 0}
                <p class="text-xs text-fg-subtle">No data in this tab.</p>
              {:else}
                <dl class="grid grid-cols-[200px,1fr] gap-x-6 gap-y-3 text-xs">
                  {#each tab.rows as row (row.label)}
                    <dt class="text-fg-muted pt-0.5">{row.label}</dt>
                    <dd class="text-fg min-w-0">
                      {#if row.isPath}
                        <button
                          type="button"
                          onclick={() => copyPath(row.value)}
                          class="font-mono text-left hover:text-accent transition-colors break-all"
                          title="Click to copy"
                        >
                          {copiedHint === row.value ? "Copied!" : row.value}
                        </button>
                      {:else}
                        <span class="font-mono break-all">{row.value}</span>
                      {/if}
                      {#if row.hint}
                        <div class="mt-1 text-[10px] text-fg-subtle leading-relaxed">
                          {row.hint}
                        </div>
                      {/if}
                    </dd>
                  {/each}
                </dl>
              {/if}
            </div>
          {/if}
        {/each}
      {/if}
    {/if}
  </section>
</div>
