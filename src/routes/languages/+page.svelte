<!--
  /languages — runtime browser.

  Two-pane layout:
    - Left rail (260px): one collapsible group per language. Each group
      header carries the language name + detected-version count; each
      version row carries the language mark + "<Display> <version>" +
      install-source pill + small status chip.
    - Right pane: header strip with status dot, name, source pill, and a
      copyable binary path. Below it, tabs of `KvRow`s rendered as
      readable form fields (label above; value in a monospace field with
      copy/reveal affordances for paths).

  Detection-only — install / config-edit flows land on follow-up cards.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { revealItemInDir } from "@tauri-apps/plugin-opener";

  import { Icon } from "$lib/components/atoms";
  import LanguageMark from "$lib/components/runtimes/LanguageMark.svelte";
  import { safeInvoke } from "$lib/ipc";
  import type {
    LanguageView,
    VersionView,
    InstallSource,
  } from "$lib/types/runtimes";
  import { sourceLabel } from "$lib/types/runtimes";

  let languages = $state<LanguageView[]>([]);
  let loading = $state<boolean>(true);

  let selectedKey = $state<string | null>(null);
  let activeTab = $state<string | null>(null);
  let copiedHint = $state<string | null>(null);
  let collapsed = $state<Record<string, boolean>>({});

  onMount(() => {
    void refresh();
  });

  async function refresh() {
    loading = true;
    try {
      languages = await safeInvoke<LanguageView[]>("list_runtimes");
      if (!selectedKey) {
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

  function toggleGroup(langId: string) {
    collapsed = { ...collapsed, [langId]: !collapsed[langId] };
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

  /**
   * Pill colour by source. Tokens are pre-composed so Tailwind's JIT
   * sees the full class strings — dynamic interpolation would otherwise
   * silently drop them.
   */
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

  function copyValue(value: string) {
    void copyHint(value);
  }

  async function revealPath(path: string) {
    try {
      await revealItemInDir(path);
    } catch {
      /* opener pushes its own toast */
    }
  }
</script>

<div class="h-full flex">
  <!-- Sub-sidebar: grouped languages -->
  <aside
    class="w-[260px] shrink-0 border-r border-border bg-surface/40
           overflow-y-auto"
    aria-label="Languages"
  >
    <div
      class="sticky top-0 z-10 px-4 pt-4 pb-3 flex items-center justify-between
             bg-surface/40 backdrop-blur-sm border-b border-border/40"
    >
      <h2
        class="text-[11px] font-semibold uppercase tracking-wider text-fg-subtle"
      >
        Languages
      </h2>
      <button
        type="button"
        onclick={refresh}
        disabled={loading}
        class="text-fg-muted hover:text-fg disabled:opacity-40 p-1 rounded-md
               hover:bg-surface-2 transition-colors"
        title="Rescan installed runtimes"
        aria-label="Rescan installed runtimes"
      >
        <Icon name="refresh-cw" size={12} />
      </button>
    </div>

    {#if loading && languages.length === 0}
      <div class="px-4 py-6 text-[12px] text-fg-subtle">
        Detecting runtimes…
      </div>
    {:else}
      <div class="px-2 py-3 space-y-1">
        {#each languages as lang (lang.id)}
          {@const isCollapsed = collapsed[lang.id] ?? false}
          {@const installed = lang.versions.length > 0}
          <div>
            <!-- Group header — collapsible -->
            <button
              type="button"
              onclick={() => toggleGroup(lang.id)}
              class="w-full flex items-center gap-2 px-2 py-1.5 rounded-md
                     text-left text-accent hover:bg-surface-2/50
                     transition-colors"
              aria-expanded={!isCollapsed}
            >
              <Icon
                name={isCollapsed ? "chevron-right" : "chevron-down"}
                size={11}
              />
              <span class="text-[13px] font-semibold tracking-tight">
                {lang.displayName}
              </span>
              {#if installed}
                <span
                  class="ml-auto text-[10px] font-mono tabular-nums text-fg-subtle"
                >
                  {lang.versions.length}
                </span>
              {:else}
                <span
                  class="ml-auto inline-block w-1.5 h-1.5 rounded-full bg-fg-subtle/40"
                  aria-hidden="true"
                ></span>
              {/if}
            </button>

            {#if !isCollapsed}
              <div class="mt-0.5 space-y-0.5 pl-1">
                {#if !installed}
                  <button
                    type="button"
                    onclick={() => copyHint(lang.installHint)}
                    class="w-full text-left px-2.5 py-2 ml-1 rounded-md
                           text-[11px] text-fg-subtle hover:text-fg-muted
                           hover:bg-surface-2/60 border border-dashed
                           border-border/60 transition-colors"
                    title="Click to copy install command"
                  >
                    <span class="block font-mono leading-snug">
                      {copiedHint === lang.installHint
                        ? "Copied!"
                        : lang.installHint}
                    </span>
                  </button>
                {:else}
                  {#each lang.versions as version (version.install.version)}
                    {@const key = `${lang.id}:${version.install.version}`}
                    {@const isActive = selectedKey === key}
                    <button
                      type="button"
                      onclick={() => selectVersion(lang.id, version)}
                      class="w-full flex items-center gap-2.5 px-2 py-1.5
                             rounded-md text-left transition-colors
                             {isActive
                        ? 'bg-accent/10 text-fg ring-1 ring-inset ring-accent/30'
                        : 'text-fg-muted hover:text-fg hover:bg-surface-2/60'}"
                    >
                      <LanguageMark id={lang.id} size={20} />
                      <span class="flex-1 min-w-0 truncate text-[12.5px]">
                        <span class="font-medium text-fg">
                          {lang.displayName}
                        </span>
                        <span class="text-fg-subtle font-mono ml-1">
                          {version.install.version}
                        </span>
                      </span>
                      <span
                        class="text-[9px] font-mono px-1.5 py-0.5 rounded
                               border {sourceClass[version.install.source]}"
                      >
                        {sourceLabel[version.install.source]}
                      </span>
                    </button>
                  {/each}
                {/if}
              </div>
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
          <Icon name="file-code" size={28} class="text-fg-subtle mx-auto" />
          <p class="mt-3 text-[13px] text-fg-muted">
            {#if languages.every((l) => l.versions.length === 0)}
              No runtimes detected on this machine.
            {:else}
              Select a version from the sidebar to view its configuration.
            {/if}
          </p>
          {#if languages.every((l) => l.versions.length === 0)}
            <p class="mt-2 text-[12px] text-fg-subtle leading-relaxed">
              Install one via Homebrew, asdf, mise, nvm, or pyenv — PortBay
              picks it up on the next rescan.
            </p>
          {/if}
        </div>
      </div>
    {:else}
      {@const { lang, version } = selected}
      <!-- Header strip -->
      <header class="px-8 pt-7 pb-5 border-b border-border/70">
        <div class="flex items-center gap-3">
          <LanguageMark id={lang.id} size={28} />
          <div class="min-w-0">
            <h1 class="text-[18px] font-semibold tracking-tight text-fg">
              <span
                class="inline-block w-2 h-2 rounded-full bg-status-running
                       align-middle mr-2"
                aria-hidden="true"
              ></span>
              {lang.displayName}
              {version.install.version}
              <span class="text-fg-muted font-normal">Config</span>
            </h1>
            <div class="mt-1.5 flex items-center gap-2 flex-wrap">
              <span
                class="text-[10px] font-mono px-1.5 py-0.5 rounded border
                       {sourceClass[version.install.source]}"
              >
                {sourceLabel[version.install.source]}
              </span>
              <button
                type="button"
                onclick={() => copyValue(version.install.binary)}
                class="text-[11px] font-mono text-fg-subtle hover:text-fg-muted
                       transition-colors"
                title="Click to copy path"
              >
                {copiedHint === version.install.binary
                  ? "Copied!"
                  : version.install.binary}
              </button>
            </div>
          </div>
        </div>
      </header>

      <!-- Tabs -->
      {#if version.tabs.length > 0}
        <div
          class="px-8 border-b border-border/70 flex gap-1 sticky top-0
                 bg-bg/95 backdrop-blur-sm z-10"
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
              class="px-3 py-2.5 text-[12px] font-medium border-b-2
                     transition-colors -mb-px
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
            <div class="px-8 py-6">
              {#if tab.rows.length === 0}
                <p class="text-[12px] text-fg-subtle">No data in this tab.</p>
              {:else}
                <div
                  class="grid gap-x-6 gap-y-5 grid-cols-1 md:grid-cols-2"
                  role="list"
                >
                  {#each tab.rows as row (row.label)}
                    <div role="listitem" class="min-w-0">
                      <span
                        class="block text-[11px] font-medium text-fg-muted mb-1.5"
                      >
                        {row.label}
                      </span>
                      <div class="flex items-stretch gap-1.5">
                        <div
                          class="flex-1 min-w-0 px-3 py-2 rounded-md bg-surface-2/60
                                 border border-border/60 text-[12px] font-mono
                                 text-fg break-all"
                        >
                          {#if row.value}
                            {row.value}
                          {:else}
                            <span class="text-fg-subtle">—</span>
                          {/if}
                        </div>
                        <button
                          type="button"
                          onclick={() => copyValue(row.value)}
                          title={copiedHint === row.value
                            ? "Copied!"
                            : "Copy value"}
                          aria-label="Copy value"
                          class="shrink-0 inline-flex items-center justify-center
                                 w-9 px-2 rounded-md border border-border/60
                                 text-fg-muted hover:text-fg
                                 hover:bg-surface-2 transition-colors
                                 {copiedHint === row.value
                            ? 'text-status-running'
                            : ''}"
                        >
                          <Icon
                            name={copiedHint === row.value ? "check" : "link"}
                            size={13}
                          />
                        </button>
                        {#if row.isPath && row.value}
                          <button
                            type="button"
                            onclick={() => revealPath(row.value)}
                            title="Reveal in Finder"
                            aria-label="Reveal {row.label} in Finder"
                            class="shrink-0 inline-flex items-center justify-center
                                   w-9 px-2 rounded-md border border-border/60
                                   text-accent hover:text-accent-hover
                                   hover:bg-accent/10 transition-colors"
                          >
                            <Icon name="folder" size={13} />
                          </button>
                        {/if}
                      </div>
                      {#if row.hint}
                        <p
                          class="mt-1.5 text-[10.5px] text-fg-subtle leading-relaxed"
                        >
                          {row.hint}
                        </p>
                      {/if}
                    </div>
                  {/each}
                </div>
              {/if}
            </div>
          {/if}
        {/each}
      {/if}
    {/if}
  </section>
</div>
