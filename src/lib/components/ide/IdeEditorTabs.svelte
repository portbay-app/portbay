<!--
  IdeEditorTabs — the editor area's tab strip: a pinned Welcome tab followed by
  one tab per open remote file. Shows a dirty dot, closes on the × or
  middle-click, and highlights the active tab. Selection + close flow through
  the `ideEditor` store.
-->
<script lang="ts">
  import Icon from "$lib/components/atoms/Icon.svelte";
  import { ideEditor, type OpenFile } from "$lib/stores/ideEditor.svelte";

  interface Props {
    files: OpenFile[];
    activeFile: string | null;
  }
  let { files, activeFile }: Props = $props();

  function onTabAux(e: MouseEvent, path: string) {
    // Middle-click closes the tab.
    if (e.button === 1) {
      e.preventDefault();
      ideEditor.close(path);
    }
  }
</script>

<div class="flex h-9 shrink-0 items-stretch overflow-x-auto border-b border-border/60 bg-surface/30">
  <!-- Welcome tab — present until dismissed (Home toggle or this ×). -->
  {#if ideEditor.welcomeOpen}
    <div
      class="group inline-flex items-center gap-1.5 border-r border-border/50 pl-3 pr-1.5 transition-colors
        {activeFile === null ? 'bg-surface text-fg' : 'text-fg-subtle hover:text-fg hover:bg-surface-2/50'}"
    >
      <button
        type="button"
        onclick={() => ideEditor.showWelcome()}
        aria-current={activeFile === null ? "page" : undefined}
        class="inline-flex items-center gap-1.5 text-[12px]"
        title="Welcome — host overview"
      >
        <Icon name="home" size={13} />
        Welcome
      </button>
      <button
        type="button"
        onclick={() => ideEditor.closeWelcome()}
        aria-label="Close Welcome"
        title="Close"
        class="grid h-4 w-4 place-items-center rounded text-fg-subtle hover:bg-surface-2 hover:text-fg"
      >
        <Icon name="x" size={12} />
      </button>
    </div>
  {/if}

  <!-- Files tab — the Finder-style remote file manager (singleton, like
       Welcome). Opened by clicking a folder in the Explorer/SFTP sidebar. -->
  {#if ideEditor.filesOpen}
    <div
      class="group inline-flex items-center gap-1.5 border-r border-border/50 pl-3 pr-1.5 transition-colors
        {ideEditor.filesActive ? 'bg-surface text-fg' : 'text-fg-subtle hover:text-fg hover:bg-surface-2/50'}"
    >
      <button
        type="button"
        onclick={() => ideEditor.openFiles()}
        aria-current={ideEditor.filesActive ? "page" : undefined}
        class="inline-flex items-center gap-1.5 text-[12px]"
        title="Files — browse this host"
      >
        <Icon name="folder" size={13} />
        Files
      </button>
      <button
        type="button"
        onclick={() => ideEditor.closeFiles()}
        aria-label="Close Files"
        title="Close"
        class="grid h-4 w-4 place-items-center rounded text-fg-subtle hover:bg-surface-2 hover:text-fg"
      >
        <Icon name="x" size={12} />
      </button>
    </div>
  {/if}

  {#each files as f (f.path)}
    {@const active = activeFile === f.path}
    <div
      class="group inline-flex items-center gap-1.5 border-r border-border/50 pl-3 pr-1.5 transition-colors
        {active ? 'bg-surface text-fg' : 'text-fg-subtle hover:text-fg hover:bg-surface-2/50'}"
    >
      <button
        type="button"
        onclick={() => ideEditor.focus(f.path)}
        onauxclick={(e) => onTabAux(e, f.path)}
        title={f.path}
        class="inline-flex items-center gap-1.5 text-[12px]"
      >
        <Icon name="file-text" size={12} class="shrink-0 text-fg-subtle" />
        <span class="max-w-40 truncate">{f.name}</span>
      </button>
      <button
        type="button"
        onclick={() => ideEditor.close(f.path)}
        aria-label={`Close ${f.name}`}
        title="Close"
        class="grid h-4 w-4 place-items-center rounded text-fg-subtle hover:bg-surface-2 hover:text-fg"
      >
        {#if f.dirty}
          <span class="h-1.5 w-1.5 rounded-full bg-fg-muted group-hover:hidden"></span>
          <span class="hidden group-hover:inline"><Icon name="x" size={12} /></span>
        {:else}
          <Icon name="x" size={12} />
        {/if}
      </button>
    </div>
  {/each}
</div>
