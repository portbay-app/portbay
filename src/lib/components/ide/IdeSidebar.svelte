<!--
  IdeSidebar — the VS Code-style primary sidebar. Renders whichever activity
  view is active (Explorer / Deploy / Tunnels / SFTP). All views stay mounted
  (hidden when inactive) so their session + scroll state survive switching, and
  so the cached SSH session is never torn down and re-authenticated.

  Explorer is a remote file tree (IdeFileTree) for browse + edit-in-IDE; SFTP is
  the transfer-focused file manager (FileBrowserPane) — upload/download, sizes,
  perms — sharing the same cached SFTP session, so neither re-prompts. The Agent
  is a right-hand aux panel (see SshWorkspace), not a view here.
-->
<script lang="ts">
  import DeployPane from "$lib/components/connections/DeployPane.svelte";
  import FileBrowserPane from "$lib/components/connections/FileBrowserPane.svelte";
  import HostTunnelsList from "$lib/components/connections/HostTunnelsList.svelte";
  import MlDashboards from "$lib/components/connections/MlDashboards.svelte";
  import IdeFileTree from "$lib/components/ide/IdeFileTree.svelte";
  import type { ActivityView } from "$lib/stores/ideLayout.svelte";
  import type { SshTunnelRuntimeStatus } from "$lib/types/sshTunnels";

  interface Props {
    activeView: ActivityView;
    connectionId: string;
    label: string;
    tunnels: SshTunnelRuntimeStatus[];
    onOpenTunnel: (id: string) => void;
    onAddTunnel: () => void;
    onOpenFile: (path: string) => void;
    activeFilePath: string | null;
    /** Open the agent panel pointed at a directory (from the Explorer tree). */
    onOpenAgentHere?: (dir: string) => void;
    /** Open a folder in the editor area's Files tab (Finder-style browser).
        Wired to plain folder clicks in both the Explorer tree and SFTP view. */
    onOpenFolder?: (path: string) => void;
    /** The project to deploy, when this host page was opened from one. */
    deployProjectId?: string | null;
  }
  let {
    activeView,
    connectionId,
    label,
    tunnels,
    onOpenTunnel,
    onAddTunnel,
    onOpenFile,
    activeFilePath,
    onOpenAgentHere,
    onOpenFolder,
    deployProjectId = null,
  }: Props = $props();

  const TITLES: Record<ActivityView, string> = {
    explorer: "Explorer",
    deploy: "Deploy",
    tunnels: "Tunnels",
    sftp: "SFTP transfers",
  };

  // Mount the SFTP pane only once its tab is first opened (then keep it mounted
  // so its listing + scroll survive switching). This avoids a second
  // connect-on-mount racing Explorer's at workspace load; both share the one
  // cached SFTP session, so there's still only ever one credential prompt.
  let sftpMounted = $state(false);
  $effect(() => {
    if (activeView === "sftp") sftpMounted = true;
  });
</script>

<div class="flex h-full min-w-0 flex-col bg-surface/20">
  <header class="flex h-9 shrink-0 items-center px-3 border-b border-border/50">
    <span class="text-[11px] font-semibold uppercase tracking-wide text-fg-subtle">
      {TITLES[activeView]}
    </span>
  </header>

  <div class="min-h-0 flex-1">
    <div class="h-full" class:hidden={activeView !== "explorer"}>
      <IdeFileTree
        {connectionId}
        {label}
        {onOpenFile}
        activePath={activeFilePath}
        {onOpenAgentHere}
        {onOpenFolder}
      />
    </div>
    <div class="h-full" class:hidden={activeView !== "deploy"}>
      <DeployPane {connectionId} {label} projectId={deployProjectId} />
    </div>
    <div class="h-full overflow-y-auto px-3 py-3" class:hidden={activeView !== "tunnels"}>
      <MlDashboards {connectionId} {label} />
      <HostTunnelsList {tunnels} {onOpenTunnel} {onAddTunnel} />
    </div>
    {#if sftpMounted}
      <div class="h-full" class:hidden={activeView !== "sftp"}>
        <!-- Sidebar variant: plain list, no toolbar — uploads, the Local
             split, the icon-grid view and new-folder live in the Files tab
             on the right (where folder clicks land). -->
        <FileBrowserPane {connectionId} {label} {onOpenFile} {onOpenFolder} variant="sidebar" />
      </div>
    {/if}
  </div>
</div>
