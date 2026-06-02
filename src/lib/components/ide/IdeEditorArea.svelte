<!--
  IdeEditorArea — the editor surface: a tab strip (Welcome + open files) over the
  active editor. The Welcome tab renders the host overview; each open file gets
  its own CodeMirror editor, all kept mounted (hidden when inactive) so undo
  history + scroll survive tab switches. Open-file state lives in `ideEditor`.
-->
<script lang="ts">
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
    <!-- Welcome (pinned) -->
    <div class="h-full overflow-y-auto" class:hidden={ideEditor.activeFile !== null}>
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

    <!-- Open files -->
    {#each ideEditor.files as f (f.path)}
      <div class="h-full" class:hidden={ideEditor.activeFile !== f.path}>
        <IdeEditor {connectionId} path={f.path} name={f.name} active={ideEditor.activeFile === f.path} />
      </div>
    {/each}
  </div>
</div>
