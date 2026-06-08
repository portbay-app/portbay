<!--
  SshWorkspace — the interactive host workspace, laid out like a VS Code IDE:

    ┌──┬───────────┬────────────────────────┐
    │AB│  Sidebar  │  Editor area (tabs)     │   AB = activity bar
    │  │ (Explorer │  Welcome | file | file… │
    │  │  Deploy   ├────────────────────────┤
    │  │  Tunnels  │  Panel: Terminal|Logs|… │
    │  │  Agent)   │                         │
    ├──┴───────────┴────────────────────────┤
    │ status bar                             │
    └────────────────────────────────────────┘

  Every pane stays mounted (hidden when inactive), so a running shell / loaded
  list / open editor survives navigation and the cached SSH session is reused
  instead of re-authenticated. Layout sizes + which view/panel is open persist
  via `ideLayout`. Open editor files live in `ideEditor` (reset on host switch).
  Ctrl+` toggles the panel; Cmd/Ctrl+B toggles the sidebar.
-->
<script lang="ts">
  import { onMount } from "svelte";

  import HostMark from "$lib/components/atoms/HostMark.svelte";
  import Icon from "$lib/components/atoms/Icon.svelte";
  import IdeActivityBar from "$lib/components/ide/IdeActivityBar.svelte";
  import IdeEditorArea from "$lib/components/ide/IdeEditorArea.svelte";
  import IdePanel from "$lib/components/ide/IdePanel.svelte";
  import IdeSidebar from "$lib/components/ide/IdeSidebar.svelte";
  import IdeStatusBar from "$lib/components/ide/IdeStatusBar.svelte";
  import Resizer from "$lib/components/ide/Resizer.svelte";
  import SshAgent from "$lib/components/connections/SshAgent.svelte";
  import { ideEditor } from "$lib/stores/ideEditor.svelte";
  import { ideLayout } from "$lib/stores/ideLayout.svelte";
  import {
    destination,
    trustMeta,
  } from "$lib/ssh/hostFormat";
  import { invokeQuiet, safeInvoke } from "$lib/ipc";
  import { fetchHostSnapshot, type HostSnapshot } from "$lib/ssh/hostSnapshot";
  import { confirmDialog } from "$lib/stores/confirm.svelte";
  import { sshConnections } from "$lib/stores/sshConnections.svelte";
  import { sshProbe } from "$lib/stores/sshProbe.svelte";
  import type { SshConnectionView } from "$lib/types/sshConnections";
  import type { SshTunnelRuntimeStatus } from "$lib/types/sshTunnels";

  interface Props {
    host: SshConnectionView;
    tunnels: SshTunnelRuntimeStatus[];
    onClose: () => void;
    onEdit: () => void;
    onRemoved: () => void;
    onOpenTunnel: (id: string) => void;
    onAddTunnel: () => void;
    /** Project to pre-fill the Deploy view from, when opened via a project. */
    deployProjectId?: string | null;
  }
  let {
    host,
    tunnels,
    onClose,
    onEdit,
    onRemoved,
    onOpenTunnel,
    onAddTunnel,
    deployProjectId = null,
  }: Props = $props();

  let menuOpen = $state(false);

  // Snapshot state. `connected` flips true once an authenticated command has
  // succeeded this session, then tracks the backend's live session registry
  // (polled below) so the badge turns off when the idle reaper — or an
  // explicit Disconnect — actually closes the host's sessions.
  let snapshot = $state<HostSnapshot | null>(null);
  let snapshotAt = $state<number | null>(null);
  let loadingSnapshot = $state(false);
  let connected = $state(false);

  // Re-sync the badge with reality: ssh_host_connected reports whether ANY
  // session (exec / SFTP / agent / shell) is still open, without touching
  // idle timers — polling can't keep a session alive.
  async function refreshConnected() {
    try {
      const live = await invokeQuiet<boolean | null>("ssh_host_connected", { id: host.id });
      if (typeof live === "boolean") connected = live;
    } catch {
      /* backend unavailable (simulator) — leave the optimistic flag */
    }
  }

  let disconnecting = $state(false);
  async function disconnectHost() {
    menuOpen = false;
    const choice = await confirmDialog.open({
      title: "Disconnect from this host?",
      message:
        `This closes every PortBay session to “${host.name}” — terminals, file browser, ` +
        `deploys and the agent. Anything still running in an open terminal is terminated.\n\n` +
        `Your next action reconnects (using the saved credential, or prompting if none is stored).`,
      icon: "unplug",
      actions: [{ label: "Disconnect", value: "disconnect", icon: "unplug" }],
    });
    if (choice !== "disconnect") return;
    disconnecting = true;
    try {
      await safeInvoke<void>("ssh_host_disconnect", { id: host.id });
      connected = false;
      snapshot = null;
      snapshotAt = null;
    } catch {
      /* safeInvoke already toasted */
    } finally {
      disconnecting = false;
    }
  }

  const dest = $derived(destination(host));
  const probe = $derived(sshProbe.get(host.id));
  const trust = $derived(trustMeta(probe?.trust));

  // Host-key chip, browser-padlock style: a quiet outline chip where only the
  // small glyph carries tone — except "changed" (possible MITM), which is loud
  // on purpose. Label + explanation live in the hover tooltip.
  const TRUST_CHIP: Record<
    "ok" | "warn" | "danger" | "neutral",
    { icon: "shield-check" | "shield" | "shield-alert"; iconTone: string; chip: string }
  > = {
    ok: { icon: "shield-check", iconTone: "text-status-running", chip: "border-border/60 text-fg-muted" },
    warn: { icon: "shield-alert", iconTone: "text-status-unhealthy", chip: "border-border/60 text-fg-muted" },
    danger: {
      icon: "shield-alert",
      iconTone: "text-status-crashed",
      chip: "border-status-crashed/40 bg-status-crashed/10 text-status-crashed",
    },
    neutral: { icon: "shield", iconTone: "text-fg-subtle", chip: "border-border/60 text-fg-muted" },
  };
  const trustChip = $derived(TRUST_CHIP[trust.tone]);

  // Probe lazily if the table hasn't already (e.g. a deep link to ?host=…).
  $effect(() => {
    if (!sshProbe.get(host.id)) void sshProbe.probe(host.id);
  });

  // A host switch is a fresh workspace: drop the previous host's open editor
  // tabs so they don't leak across connections.
  let lastHostId = $state<string | null>(null);
  $effect(() => {
    if (lastHostId !== null && lastHostId !== host.id) {
      ideEditor.reset();
    }
    lastHostId = host.id;
  });

  // Mount the Agent panel only once it's first opened, then keep it mounted so
  // its session + transcript survive toggling it shut. Connecting the host's
  // agent session (and probing for a model) is wasted work if the user never
  // opens it, so this lazy-latch defers that until the first open.
  let agentMounted = $state(false);
  $effect(() => {
    if (ideLayout.agentVisible) agentMounted = true;
  });

  // "Open agent here" from the Explorer tree: reveal the agent panel (mounting it
  // on first use) and push the chosen directory into its working dir. The nonce
  // makes the same folder re-applyable, and lets the agent override the cwd its
  // restored thread loaded on first mount.
  let agentCwdRequest = $state<{ path: string; nonce: number } | null>(null);
  let agentCwdSeq = 0;
  function openAgentAt(dir: string) {
    agentMounted = true;
    if (!ideLayout.agentVisible) ideLayout.toggleAgent();
    agentCwdRequest = { path: dir, nonce: ++agentCwdSeq };
  }

  // Run the one-shot snapshot command (prompting once for a credential if
  // needed). Success is also our proof the host is reachable + authenticating,
  // so we stamp last-used and flip `connected`.
  async function loadSnapshot() {
    if (loadingSnapshot) return;
    loadingSnapshot = true;
    try {
      snapshot = await fetchHostSnapshot(host.id, dest);
      snapshotAt = Math.floor(Date.now() / 1000);
      connected = true;
      void sshConnections.touch(host.id);
      void sshConnections.refresh();
      void sshProbe.probe(host.id);
    } catch {
      /* connectWithPrompt already surfaced any real failure */
    } finally {
      loadingSnapshot = false;
    }
  }

  async function removeHost() {
    menuOpen = false;
    const choice = await confirmDialog.open({
      title: "Remove host from PortBay?",
      message:
        `This removes “${host.name}” from PortBay only — its saved connection and any ` +
        `keychain password.\n\nYour ~/.ssh/config and any source you imported it from stay untouched.`,
      destructive: true,
      icon: "trash-2",
      actions: [
        { label: "Remove from PortBay", value: "remove", tone: "destructive", icon: "trash-2" },
      ],
    });
    if (choice !== "remove") return;
    const ok = await sshConnections.remove(host.id);
    if (ok) onRemoved();
  }

  // VS Code-style shortcuts, scoped to while this workspace is mounted.
  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "`" && (e.ctrlKey || e.metaKey)) {
        e.preventDefault();
        ideLayout.togglePanel();
      } else if (e.key.toLowerCase() === "b" && (e.ctrlKey || e.metaKey) && !e.shiftKey) {
        e.preventDefault();
        ideLayout.toggleSidebar();
      }
    };
    window.addEventListener("keydown", onKey);
    // Warm the connection on entry: one authed command flips `connected` and
    // (via the coalesced prompt + secret cache) means the shell / file tree that
    // mount alongside don't ask again.
    void loadSnapshot();
    // Keep the Connected badge honest while the workspace is open — the idle
    // reaper closes sessions after 15 quiet minutes and the badge must follow.
    const poll = setInterval(() => void refreshConnected(), 30_000);
    return () => {
      window.removeEventListener("keydown", onKey);
      clearInterval(poll);
      ideEditor.reset();
    };
  });
</script>

<!-- Guard against a teardown pass re-rendering with `host` cleared. -->
{#if host}
  <section class="flex h-full min-w-0 flex-col bg-surface/20">
    <!-- Slim host header -->
    <header class="flex items-center gap-3 border-b border-border/60 px-4 py-2.5">
      <HostMark environment={host.environment} size={28} class="shrink-0" />
      <div class="flex min-w-0 items-center gap-2.5">
        <h1 class="min-w-0 truncate text-[14px] font-semibold tracking-tight text-fg">{host.name}</h1>
        <span class="truncate font-mono text-[11.5px] text-fg-subtle">{dest}:{host.sshPort}</span>
        {#if probe}
          <span
            class="inline-flex shrink-0 items-center gap-1 rounded-full border px-2 py-[3px]
                   text-[10.5px] font-medium leading-none {trustChip.chip}"
          >
            <Icon name={trustChip.icon} size={11} class={trustChip.iconTone} />
            {trust.label}
          </span>
        {/if}
      </div>

      <div class="ml-auto flex shrink-0 items-center gap-1.5">
        <button
          type="button"
          onclick={loadSnapshot}
          disabled={loadingSnapshot}
          class="inline-flex items-center gap-1.5 h-7 px-2.5 rounded-md text-[11.5px] font-medium
                 border border-border text-fg-muted hover:bg-surface-2 hover:text-fg disabled:opacity-60"
        >
          <Icon name={loadingSnapshot ? "refresh-cw" : "rotate-cw"} size={12} class={loadingSnapshot ? "animate-spin" : ""} />
          {loadingSnapshot ? "Connecting…" : "Refresh"}
        </button>

        <div class="relative">
          <button
            type="button"
            onclick={() => (menuOpen = !menuOpen)}
            class="grid place-items-center w-7 h-7 rounded-md border border-border text-fg-muted hover:bg-surface-2 hover:text-fg"
            aria-label="Host actions"
          >
            <Icon name="more-horizontal" size={15} />
          </button>
          {#if menuOpen}
            <button type="button" class="fixed inset-0 z-10 cursor-default" aria-label="Close menu" onclick={() => (menuOpen = false)}></button>
            <div class="absolute right-0 z-20 mt-1 w-44 rounded-lg border border-border bg-surface p-1 shadow-xl">
              <button type="button" onclick={() => { menuOpen = false; onEdit(); }} class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg">
                <Icon name="pencil" size={13} /> Edit host
              </button>
              <button type="button" onclick={() => { menuOpen = false; void sshConnections.detectOs(host.id); }} disabled={sshConnections.isBusy(`${host.id}:os`)} class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg disabled:opacity-50">
                <Icon name="server-cog" size={13} /> Detect OS
              </button>
              <button type="button" onclick={disconnectHost} disabled={!connected || disconnecting} title={connected ? "Close every PortBay session to this host" : "No open sessions"} class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg disabled:opacity-50">
                <Icon name="unplug" size={13} /> {disconnecting ? "Disconnecting…" : "Disconnect"}
              </button>
              <button type="button" onclick={removeHost} disabled={host.inUse} title={host.inUse ? "Remove this host's tunnels first" : ""} class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-status-crashed hover:bg-status-crashed/10 disabled:opacity-50">
                <Icon name="trash-2" size={13} /> Remove
              </button>
            </div>
          {/if}
        </div>

        <button type="button" onclick={onClose} class="grid place-items-center w-7 h-7 rounded-md border border-border text-fg-muted hover:bg-surface-2 hover:text-fg" aria-label="Back to hosts" title="Back to hosts">
          <Icon name="x" size={15} />
        </button>
      </div>
    </header>

    <!-- Main: activity bar | sidebar | (editor area / panel) -->
    <div class="flex min-h-0 flex-1">
      <IdeActivityBar
        activeView={ideLayout.activeView}
        sidebarVisible={ideLayout.sidebarVisible}
        agentVisible={ideLayout.agentVisible}
        homeActive={ideEditor.welcomeActive}
        terminalActive={ideLayout.panelVisible && ideLayout.panelTab === "terminal"}
        tunnelCount={tunnels.length}
        onSelect={(v) => ideLayout.selectView(v)}
        onHome={() => ideEditor.toggleWelcome()}
        onToggleTerminal={() => {
          if (ideLayout.panelVisible && ideLayout.panelTab === "terminal") ideLayout.togglePanel();
          else ideLayout.showPanelTab("terminal");
        }}
        onToggleAgent={() => ideLayout.toggleAgent()}
        onSettings={onEdit}
      />

      {#if ideLayout.sidebarVisible}
        <div class="min-w-0 shrink-0" style="width: {ideLayout.sidebarWidth}px">
          <IdeSidebar
            activeView={ideLayout.activeView}
            connectionId={host.id}
            label={dest}
            {tunnels}
            {onOpenTunnel}
            {onAddTunnel}
            onOpenFile={(path) => ideEditor.open(path)}
            activeFilePath={ideEditor.activeFile}
            onOpenAgentHere={openAgentAt}
            onOpenFolder={(path) => ideEditor.openFiles(path)}
            {deployProjectId}
          />
        </div>
        <Resizer
          axis="x"
          value={ideLayout.sidebarWidth}
          set={(px) => ideLayout.setSidebarWidth(px)}
          aria-label="Resize sidebar"
        />
      {/if}

      <!-- Editor area + bottom panel -->
      <div class="flex min-w-0 flex-1 flex-col">
        <div class="min-h-0 flex-1">
          <IdeEditorArea
            connectionId={host.id}
            {host}
            {dest}
            {snapshot}
            {snapshotAt}
            {loadingSnapshot}
            {connected}
            {probe}
            onRefresh={loadSnapshot}
            {onAddTunnel}
          />
        </div>

        {#if ideLayout.panelVisible}
          <Resizer
            axis="y"
            value={ideLayout.panelHeight}
            set={(px) => ideLayout.setPanelHeight(px)}
            invert
            aria-label="Resize panel"
          />
          <div class="shrink-0" style="height: {ideLayout.panelHeight}px">
            <IdePanel
              connectionId={host.id}
              label={dest}
              {host}
              panelTab={ideLayout.panelTab}
              onSelectTab={(t) => ideLayout.showPanelTab(t)}
              onClose={() => ideLayout.togglePanel()}
            />
          </div>
        {/if}
      </div>

      <!-- Right-hand Agent aux panel (VS Code secondary sidebar). Mounted lazily
           on first open, then kept mounted (hidden when collapsed) so its
           session + transcript survive toggling. Re-keyed per host so switching
           hosts reconnects the agent to the new box. The resizer only renders
           while the panel is open. -->
      {#if agentMounted && ideLayout.agentVisible}
        <Resizer
          axis="x"
          value={ideLayout.agentWidth}
          set={(px) => ideLayout.setAgentWidth(px)}
          invert
          aria-label="Resize agent panel"
        />
      {/if}
      {#if agentMounted}
        <aside
          class="min-w-0 shrink-0 border-l border-border/60"
          class:hidden={!ideLayout.agentVisible}
          style="width: {ideLayout.agentWidth}px"
        >
          {#key host.id}
            <SshAgent connectionId={host.id} label={dest} cwdRequest={agentCwdRequest} onClose={() => ideLayout.toggleAgent()} />
          {/key}
        </aside>
      {/if}
    </div>

    <IdeStatusBar
      hostName={host.name}
      {dest}
      port={host.sshPort}
      {connected}
      health={probe?.health ?? "unknown"}
      tunnelCount={tunnels.length}
      panelVisible={ideLayout.panelVisible}
      onTogglePanel={() => ideLayout.togglePanel()}
    />
  </section>
{/if}
