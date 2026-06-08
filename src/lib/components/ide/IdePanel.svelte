<!--
  IdePanel — the VS Code-style bottom panel: a tab strip (Terminal / Logs /
  Processes / Ports) over the active pane. All panes stay mounted (hidden when
  inactive) so a running shell or a loaded list survives switching tabs, and the
  cached SSH session is reused rather than re-authenticated.
-->
<script lang="ts">
  import Icon from "$lib/components/atoms/Icon.svelte";
  import type { IconName } from "$lib/components/atoms/Icon.svelte";
  import SshGpu from "$lib/components/connections/SshGpu.svelte";
  import SshJobs from "$lib/components/connections/SshJobs.svelte";
  import SshLogs from "$lib/components/connections/SshLogs.svelte";
  import SshPorts from "$lib/components/connections/SshPorts.svelte";
  import SshProblems from "$lib/components/connections/SshProblems.svelte";
  import SshProcesses from "$lib/components/connections/SshProcesses.svelte";
  import SshTerminalTabs from "$lib/components/connections/SshTerminalTabs.svelte";
  import type { PanelTab } from "$lib/stores/ideLayout.svelte";
  import type { SshConnectionView } from "$lib/types/sshConnections";

  interface Props {
    connectionId: string;
    label: string;
    host: SshConnectionView;
    panelTab: PanelTab;
    onSelectTab: (tab: PanelTab) => void;
    onClose: () => void;
  }
  let { connectionId, label, host, panelTab, onSelectTab, onClose }: Props = $props();

  const TABS: { id: PanelTab; label: string; icon: IconName }[] = [
    { id: "terminal", label: "Terminal", icon: "terminal" },
    { id: "logs", label: "Logs", icon: "file-text" },
    { id: "processes", label: "Processes", icon: "list" },
    { id: "gpu", label: "GPU", icon: "cpu" },
    { id: "jobs", label: "Jobs", icon: "layers" },
    { id: "ports", label: "Ports", icon: "circle-dot" },
    { id: "problems", label: "Problems", icon: "alert-triangle" },
  ];
</script>

<section class="flex h-full min-h-0 flex-col border-t border-border/60 bg-surface/20">
  <header class="flex h-8 shrink-0 items-center gap-1 border-b border-border/50 px-2">
    {#each TABS as t (t.id)}
      {@const active = panelTab === t.id}
      <button
        type="button"
        onclick={() => onSelectTab(t.id)}
        aria-current={active ? "page" : undefined}
        class="inline-flex items-center gap-1.5 h-7 px-2.5 rounded text-[11.5px] font-medium transition-colors
          {active ? 'text-fg bg-surface-2' : 'text-fg-subtle hover:text-fg'}"
      >
        <Icon name={t.icon} size={13} />
        {t.label}
      </button>
    {/each}
    <button
      type="button"
      onclick={onClose}
      title="Hide panel (Ctrl+`)"
      aria-label="Hide panel"
      class="ml-auto grid h-6 w-6 place-items-center rounded text-fg-subtle hover:bg-surface-2 hover:text-fg"
    >
      <Icon name="chevron-down" size={15} />
    </button>
  </header>

  <div class="min-h-0 flex-1">
    <div class="h-full" class:hidden={panelTab !== "terminal"}>
      <SshTerminalTabs {connectionId} {label} />
    </div>
    <div class="h-full" class:hidden={panelTab !== "logs"}>
      <SshLogs {connectionId} {label} />
    </div>
    <div class="h-full" class:hidden={panelTab !== "processes"}>
      <SshProcesses {connectionId} {label} active={panelTab === "processes"} />
    </div>
    <div class="h-full" class:hidden={panelTab !== "gpu"}>
      <SshGpu {connectionId} {label} active={panelTab === "gpu"} />
    </div>
    <div class="h-full" class:hidden={panelTab !== "jobs"}>
      <SshJobs {connectionId} {label} active={panelTab === "jobs"} />
    </div>
    <div class="h-full" class:hidden={panelTab !== "ports"}>
      <SshPorts {connectionId} {label} {host} active={panelTab === "ports"} />
    </div>
    <div class="h-full" class:hidden={panelTab !== "problems"}>
      <SshProblems {connectionId} {label} active={panelTab === "problems"} />
    </div>
  </div>
</section>
