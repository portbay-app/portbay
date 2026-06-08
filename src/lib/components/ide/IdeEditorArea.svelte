<!--
  IdeEditorArea — the editor surface: a tab strip (Welcome + open files) over the
  active editor. The Welcome tab renders the host overview; each open file gets
  its own CodeMirror editor, all kept mounted (hidden when inactive) so undo
  history + scroll survive tab switches. Open-file state lives in `ideEditor`.
-->
<script lang="ts">
  import FileBrowserPane from "$lib/components/connections/FileBrowserPane.svelte";
  import Icon from "$lib/components/atoms/Icon.svelte";
  import IdeEditor from "$lib/components/ide/IdeEditor.svelte";
  import IdeEditorTabs from "$lib/components/ide/IdeEditorTabs.svelte";
  import IdeWelcome from "$lib/components/ide/IdeWelcome.svelte";
  import { ideEditor } from "$lib/stores/ideEditor.svelte";
  import type { HostSnapshot } from "$lib/ssh/hostSnapshot";
  import type { ProbeResult, SshConnectionView } from "$lib/types/sshConnections";

  interface Props {
    connectionId: string;
    host: SshConnectionView;
    dest: string;
    snapshot: HostSnapshot | null;
    snapshotAt: number | null;
    loadingSnapshot: boolean;
    connected: boolean;
    probe: ProbeResult | null;
    onRefresh: () => void;
    onAddTunnel: () => void;
  }
  let {
    connectionId,
    host,
    dest,
    snapshot,
    snapshotAt,
    loadingSnapshot,
    connected,
    probe,
    onRefresh,
    onAddTunnel,
  }: Props = $props();
</script>

<div class="flex h-full min-h-0 flex-col">
  <IdeEditorTabs files={ideEditor.files} activeFile={ideEditor.activeFile} />

  <div class="min-h-0 flex-1">
    <!-- Welcome (dismissable via the Home toggle / its tab ×). -->
    <div class="h-full overflow-y-auto" class:hidden={!ideEditor.welcomeActive}>
      <IdeWelcome
        {host}
        {dest}
        {snapshot}
        {snapshotAt}
        {loadingSnapshot}
        {connected}
        {probe}
        {onRefresh}
        {onAddTunnel}
      />
    </div>

    <!-- Files — the Finder-style remote file manager, full-featured (browse,
         upload/download, search, rename/chmod/delete) with its built-in
         right-hand inspector column. Kept mounted while its tab is open so
         the listing + navigation survive tab switches. -->
    {#if ideEditor.filesOpen}
      <div class="h-full" class:hidden={!ideEditor.filesActive}>
        <FileBrowserPane
          {connectionId}
          label={dest}
          onOpenFile={(p) => ideEditor.open(p)}
          navigateRequest={ideEditor.filesRequest}
        />
      </div>
    {/if}

    <!-- Empty state: no file open and Welcome dismissed. -->
    {#if ideEditor.activeFile === null && !ideEditor.welcomeOpen}
      <div class="flex h-full flex-col items-center justify-center gap-2 text-center text-fg-subtle">
        <Icon name="home" size={22} class="opacity-60" />
        <p class="text-[12.5px]">No file open. Pick one from the Explorer, or open Home from the rail.</p>
      </div>
    {/if}

    <!-- Open files -->
    {#each ideEditor.files as f (f.path)}
      <div class="h-full" class:hidden={ideEditor.activeFile !== f.path}>
        <IdeEditor {connectionId} path={f.path} name={f.name} active={ideEditor.activeFile === f.path} label={dest} />
      </div>
    {/each}
  </div>
</div>
