<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { page } from "$app/state";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import StatusDot from "$lib/components/atoms/StatusDot.svelte";
  import HostConnectionForm from "$lib/components/connections/HostConnectionForm.svelte";
  import SshHostTable from "$lib/components/connections/SshHostTable.svelte";
  import SshHostPanel from "$lib/components/connections/SshHostPanel.svelte";
  import SshWorkspace from "$lib/components/connections/SshWorkspace.svelte";
  import IdentitiesManager from "$lib/components/connections/IdentitiesManager.svelte";
  import SshConfigImport from "$lib/components/connections/SshConfigImport.svelte";
  import { entitlements } from "$lib/stores/entitlements.svelte";
  import { errorBus } from "$lib/stores/errors.svelte";
  import { sshTunnels } from "$lib/stores/sshTunnels.svelte";
  import { sshConnections } from "$lib/stores/sshConnections.svelte";
  import { sshProbe } from "$lib/stores/sshProbe.svelte";
  import { fileBrowser } from "$lib/stores/fileBrowser.svelte";
  import { deployPanel } from "$lib/stores/deployPanel.svelte";
  import { confirmDialog } from "$lib/stores/confirm.svelte";
  import type {
    OpenSshTunnelDatabaseInput,
    SaveSshTunnelInput,
    SshForwardKind,
    SshTunnelRuntimeStatus,
  } from "$lib/types/sshTunnels";
  import type { SshConnectionView } from "$lib/types/sshConnections";

  const DB_ENGINES: { id: OpenSshTunnelDatabaseInput["engine"]; label: string }[] = [
    { id: "postgres", label: "PostgreSQL" },
    { id: "mysql", label: "MySQL" },
    { id: "mariadb", label: "MariaDB" },
    { id: "redis", label: "Redis" },
    { id: "mongo", label: "MongoDB" },
    { id: "memcached", label: "Memcached" },
  ];

  let copied = $state<string | null>(null);
  let editing = $state(false);
  // When a tunnel is created from a host, remember it so cancel returns there.
  let tunnelFormHost = $state<string | null>(null);
  let dbEngine = $state<OpenSshTunnelDatabaseInput["engine"]>("postgres");
  let remotePath = $state<string>("~/app");

  let form = $state<SaveSshTunnelInput>({
    id: null,
    name: "",
    sshHost: "",
    sshPort: 22,
    sshUser: "",
    authKind: "key",
    keyPath: "",
    password: "",
    localHost: "127.0.0.1",
    localPort: null,
    remoteHost: "localhost",
    remotePort: 5432,
    forwardKind: "local",
    proxyJump: "",
    keepAlive: false,
    autoReconnect: false,
  });

  onMount(() => {
    sshTunnels.startPolling();
    void sshConnections.refresh().then(refreshHealth);
    return () => sshTunnels.stopPolling();
  });

  // Probe every saved host's reachability/health for the table's Health column.
  // Explicit — runs on mount and the table's Refresh button, never a poll.
  function refreshHealth() {
    void sshProbe.probeAll(sshConnections.value.map((c) => c.id));
  }

  // Which surface to show is driven by the `?tunnel` query param so it is
  // deep-linkable and back/forward-aware — and, as a query (not a path)
  // change, it never disturbs the shared SSH/Cloudflare rail:
  //   absent → the list (or empty state)
  //   "new"  → the create form
  //   "<id>" → that tunnel's detail; its Edit button flips `editing` on to
  //            reuse the same form for re-pointing.
  const tunnelParam = $derived(page.url.searchParams.get("tunnel"));
  const creating = $derived(tunnelParam === "new");
  const detailTunnel = $derived<SshTunnelRuntimeStatus | null>(
    tunnelParam && tunnelParam !== "new"
      ? (sshTunnels.value.find((t) => t.id === tunnelParam) ?? null)
      : null,
  );
  const showForm = $derived(creating || (detailTunnel !== null && editing));

  // Host-first surface. The dashboard is the landing view once ≥1 connection
  // exists; `?host=new` is the add-host form, `?host=<id>` drills into a host
  // (its tunnels + Files/Run). `editingHost` flips the host form on over a host.
  let editingHost = $state(false);
  const hostParam = $derived(page.url.searchParams.get("host"));
  const creatingHost = $derived(hostParam === "new");
  const detailHost = $derived<SshConnectionView | null>(
    hostParam && hostParam !== "new"
      ? (sshConnections.find(hostParam) ?? null)
      : null,
  );
  const showHostForm = $derived(creatingHost || (detailHost !== null && editingHost));
  const hostTunnels = $derived(
    detailHost ? sshTunnels.value.filter((t) => t.connectionId === detailHost.id) : [],
  );
  // The landing shows the dashboard once any host is saved; below that, the
  // host-first empty state. A tunnel always has a connection, so "tunnels exist"
  // implies "hosts exist".
  const showDashboard = $derived(sshConnections.count > 0);
  // `?identities=1` opens the reusable-identities manager over the dashboard.
  const managingIdentities = $derived(page.url.searchParams.get("identities") === "1");
  // `?import=1` opens the ~/.ssh/config import preview over the dashboard.
  const importingConfig = $derived(page.url.searchParams.get("import") === "1");

  // Selecting a real host (`?host=<id>`) opens the interactive host workspace
  // directly as a full-pane VS Code-style IDE takeover — no separate "open"
  // step. Internal layout (which view/panel is open) lives in `ideLayout`, not
  // the URL. `host=new` and the edit form are handled by the branches above, so
  // `detailHost` is null there.
  const workspaceHost = $derived<SshConnectionView | null>(detailHost);

  function openIdentities() {
    void goto("/ssh?identities=1", { keepFocus: true, noScroll: true });
  }

  function openImport() {
    void goto("/ssh?import=1", { keepFocus: true, noScroll: true });
  }

  // After an import, land on the first new host (or the dashboard if none).
  function onImportDone(firstId: string | null) {
    if (firstId) openHost(firstId);
    else openDashboard();
  }

  function openTunnel(id: string) {
    editing = false;
    void goto(`/ssh?tunnel=${encodeURIComponent(id)}`, {
      keepFocus: true,
      noScroll: true,
    });
  }

  function openHost(id: string) {
    editing = false;
    editingHost = false;
    // Don't stamp last-used on mere selection — that made a host look "used"
    // just for being opened. The workspace's snapshot stamps it only after a
    // successful authenticated connect (see SshWorkspace.loadSnapshot), so
    // "last used" reflects a real connection.
    void goto(`/ssh?host=${encodeURIComponent(id)}`, { keepFocus: true, noScroll: true });
  }

  function openDashboard() {
    editing = false;
    editingHost = false;
    resetForm();
    void goto("/ssh", { keepFocus: true, noScroll: true });
  }

  function startCreateHost() {
    editingHost = false;
    void goto("/ssh?host=new", { keepFocus: true, noScroll: true });
  }

  // Open a host straight into its edit form from the table's quick-actions menu.
  function editHostFromTable(id: string) {
    editing = false;
    editingHost = true;
    void goto(`/ssh?host=${encodeURIComponent(id)}`, { keepFocus: true, noScroll: true });
  }

  // Remove a host from the table's quick-actions menu (same contract as the
  // workspace's Remove: PortBay state only, never ~/.ssh/config).
  async function removeHostFromTable(id: string) {
    const host = sshConnections.find(id);
    if (!host) return;
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
    await sshConnections.remove(id);
  }

  function startEditHost() {
    editingHost = true;
  }

  function onHostSaved(saved: SshConnectionView) {
    editingHost = false;
    openHost(saved.id);
  }

  function onHostCancel() {
    if (editingHost && detailHost) {
      editingHost = false;
    } else {
      openDashboard();
    }
  }

  // Add a tunnel for a specific host: prefill the connection fields so the
  // find-or-create matches the existing connection (no duplicate host).
  function startCreateForHost(conn: SshConnectionView) {
    resetForm();
    form.sshHost = conn.sshHost;
    form.sshPort = conn.sshPort;
    form.sshUser = conn.sshUser;
    form.authKind = conn.authKind;
    form.keyPath = conn.keyPath ?? "";
    form.proxyJump = conn.proxyJump ?? "";
    editing = false;
    tunnelFormHost = conn.id;
    void goto("/ssh?tunnel=new", { keepFocus: true, noScroll: true });
  }

  function startEdit(tunnel: SshTunnelRuntimeStatus) {
    editTunnel(tunnel);
    editing = true;
  }

  // Back/cancel out of the form: an edit returns to the tunnel's detail (the
  // URL is already there), a create returns to the originating host (or the
  // dashboard when added standalone).
  function cancelForm() {
    if (form.id) {
      editing = false;
    } else if (tunnelFormHost) {
      openHost(tunnelFormHost);
    } else {
      openDashboard();
    }
  }

  async function removeTunnel(id: string) {
    const host = detailTunnel?.connectionId ?? null;
    await sshTunnels.remove(id);
    await sshConnections.refresh();
    if (host) openHost(host);
    else openDashboard();
  }

  function resetForm() {
    form = {
      id: null,
      name: "",
      sshHost: "",
      sshPort: 22,
      sshUser: "",
      authKind: "key",
      keyPath: "",
      password: "",
      localHost: "127.0.0.1",
      localPort: null,
      remoteHost: "localhost",
      remotePort: 5432,
      forwardKind: "local",
      proxyJump: "",
      keepAlive: false,
      autoReconnect: false,
    };
  }

  async function submit() {
    const saved = await sshTunnels.save(form);
    if (saved) {
      editing = false;
      resetForm();
      // A new tunnel may have created a connection / changed tunnel counts.
      void sshConnections.refresh();
      void goto(`/ssh?tunnel=${encodeURIComponent(saved.id)}`, {
        keepFocus: true,
        noScroll: true,
      });
    }
  }

  async function copyText(id: string, text: string, whatHappened: string, whyItMatters: string) {
    try {
      await navigator.clipboard.writeText(text);
      copied = id;
      setTimeout(() => {
        if (copied === id) copied = null;
      }, 1500);
      errorBus.push({
        code: "COPIED",
        category: "infrastructure",
        whatHappened,
        whyItMatters,
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });
    } catch {
      /* no clipboard permission */
    }
  }

  function editTunnel(tunnel: SshTunnelRuntimeStatus) {
    form = {
      id: tunnel.id,
      name: tunnel.name,
      sshHost: tunnel.sshHost,
      sshPort: tunnel.sshPort,
      sshUser: tunnel.sshUser,
      authKind: tunnel.authKind,
      keyPath: tunnel.keyPath ?? "",
      password: "",
      localHost: tunnel.localHost,
      localPort: tunnel.localPort,
      remoteHost: tunnel.remoteHost,
      remotePort: tunnel.remotePort,
      forwardKind: tunnel.forwardKind,
      proxyJump: tunnel.proxyJump ?? "",
      keepAlive: tunnel.keepAlive,
      autoReconnect: tunnel.autoReconnect,
    };
  }

  async function copyCommand(tunnel: SshTunnelRuntimeStatus) {
    await copyText(
      tunnel.id,
      tunnel.command,
      "SSH command copied.",
      "You can run the exact same tunnel in a terminal.",
    );
  }

  function statusFor(tunnel: SshTunnelRuntimeStatus): "running" | "stopped" | "starting" {
    if (sshTunnels.isBusy(tunnel.id)) return "starting";
    return tunnel.running ? "running" : "stopped";
  }

  function forwardLabel(kind: SshForwardKind): string {
    switch (kind) {
      case "local":
        return "Local";
      case "reverse":
        return "Reverse";
      case "socks":
        return "SOCKS";
    }
  }

  const proLocked = $derived(!entitlements.isPro);

  function sshDestination(tunnel: SshTunnelRuntimeStatus): string {
    return tunnel.sshUser.trim()
      ? `${tunnel.sshUser}@${tunnel.sshHost}`
      : tunnel.sshHost;
  }

  function sshConfigAlias(tunnel: SshTunnelRuntimeStatus): string {
    const slug =
      (tunnel.name || tunnel.sshHost)
        .toLowerCase()
        .replace(/[^a-z0-9]+/g, "-")
        .replace(/^-+|-+$/g, "")
        .slice(0, 48) || tunnel.id;
    return `portbay-${slug}`;
  }

  function hasExplicitTransportOptions(tunnel: SshTunnelRuntimeStatus): boolean {
    return Boolean(
      tunnel.sshPort !== 22 ||
        tunnel.keyPath?.trim() ||
        tunnel.proxyJump?.trim() ||
        tunnel.keepAlive ||
        tunnel.autoReconnect,
    );
  }

  function sshRemoteTarget(tunnel: SshTunnelRuntimeStatus): string {
    return hasExplicitTransportOptions(tunnel) ? sshConfigAlias(tunnel) : sshDestination(tunnel);
  }

  function sshConfigSnippet(tunnel: SshTunnelRuntimeStatus): string {
    const lines = [`Host ${sshConfigAlias(tunnel)}`, `  HostName ${tunnel.sshHost}`];
    if (tunnel.sshUser.trim()) lines.push(`  User ${tunnel.sshUser.trim()}`);
    if (tunnel.sshPort !== 22) lines.push(`  Port ${tunnel.sshPort}`);
    if (tunnel.keyPath?.trim()) lines.push(`  IdentityFile ${tunnel.keyPath.trim()}`);
    if (tunnel.proxyJump?.trim()) lines.push(`  ProxyJump ${tunnel.proxyJump.trim()}`);
    if (tunnel.keepAlive || tunnel.autoReconnect) {
      lines.push("  ServerAliveInterval 15", "  ServerAliveCountMax 3");
    }
    return lines.join("\n");
  }

  function shellQuote(value: string): string {
    if (/^[A-Za-z0-9_./:@=+,-]+$/.test(value)) return value;
    return `'${value.replaceAll("'", "'\\''")}'`;
  }

  function sshTransportArgs(tunnel: SshTunnelRuntimeStatus): string[] {
    const args: string[] = [];
    if (tunnel.sshPort !== 22) args.push("-p", String(tunnel.sshPort));
    if (tunnel.keyPath?.trim()) args.push("-i", tunnel.keyPath.trim());
    if (tunnel.proxyJump?.trim()) args.push("-J", tunnel.proxyJump.trim());
    return args;
  }

  function sshTransportCommand(tunnel: SshTunnelRuntimeStatus): string {
    const args = sshTransportArgs(tunnel).map(shellQuote);
    return ["ssh", ...args].join(" ");
  }

  function sshCommandPrefix(tunnel: SshTunnelRuntimeStatus): string {
    const args = sshTransportArgs(tunnel).map(shellQuote);
    return ["ssh", ...args, shellQuote(sshDestination(tunnel))].join(" ");
  }

  function normalizedRemotePath(): string {
    return remotePath.trim() || "~";
  }

  function remoteEditorCommand(tunnel: SshTunnelRuntimeStatus, cli: "code" | "cursor"): string {
    return `${cli} --remote ${shellQuote(`ssh-remote+${sshRemoteTarget(tunnel)}`)} ${shellQuote(normalizedRemotePath())}`;
  }

  function rsyncUploadCommand(tunnel: SshTunnelRuntimeStatus): string {
    const remote = `${sshDestination(tunnel)}:${normalizedRemotePath().replace(/\/?$/, "/")}`;
    return `rsync -az --progress -e ${shellQuote(sshTransportCommand(tunnel))} ./ ${shellQuote(remote)}`;
  }

  function scpUploadCommand(tunnel: SshTunnelRuntimeStatus): string {
    const remote = `${sshDestination(tunnel)}:${normalizedRemotePath().replace(/\/?$/, "/")}`;
    return `scp -r ${sshTransportArgs(tunnel).map(shellQuote).join(" ")} ./ ${shellQuote(remote)}`.replace(/\s+/g, " ").trim();
  }

  function remoteGitPullCommand(tunnel: SshTunnelRuntimeStatus): string {
    const remoteCommand = `cd ${shellQuote(normalizedRemotePath())} && git pull --ff-only`;
    return `${sshCommandPrefix(tunnel)} ${shellQuote(remoteCommand)}`;
  }
</script>

<!-- SSH surface — the inner rail (SSH / Cloudflare) lives in the (connections)
     layout, so this is a single pane that drills list → detail → editor. -->
<section class="h-full min-w-0 overflow-y-auto">
  {#if showForm}
    <!-- Create / edit editor -->
    <header class="px-8 pt-6 pb-4 border-b border-border/60">
      <button
        type="button"
        onclick={cancelForm}
        class="inline-flex items-center gap-1.5 text-[12px] text-fg-muted hover:text-fg transition-colors"
      >
        <Icon name="chevron-left" size={14} />
        {form.id ? "Back to tunnel" : "Back to tunnels"}
      </button>
      <h1 class="mt-3 text-[17px] font-semibold tracking-tight text-fg">
        {form.id ? "Edit tunnel" : "Add SSH tunnel"}
      </h1>
      <p class="mt-1 text-[12.5px] text-fg-muted">
        First-use host keys use OpenSSH accept-new; changed host keys still fail.
      </p>
    </header>

    <!-- @container: field grids below reflow to the *pane* width (which is
         offset by the sidebar + shared rail), not the raw viewport width. -->
    <div class="px-8 py-6 @container">
      <form
        class="w-full rounded-lg border border-border/70 bg-surface px-5 py-5 space-y-3"
        onsubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        <label class="block text-[11px] font-medium text-fg-subtle">
          Name
          <input bind:value={form.name} class="mt-1 w-full h-8 rounded-md border border-border bg-surface px-2 text-[12px] text-fg" placeholder="Production DB inspect" />
        </label>

        <div class="grid grid-cols-[1fr_74px] gap-2 @max-[380px]:grid-cols-1">
          <label class="block text-[11px] font-medium text-fg-subtle">
            SSH host
            <input bind:value={form.sshHost} class="mt-1 w-full h-8 rounded-md border border-border bg-surface px-2 text-[12px] text-fg" placeholder="bastion.example.com or Host alias" />
          </label>
          <label class="block text-[11px] font-medium text-fg-subtle">
            Port
            <input bind:value={form.sshPort} type="number" min="1" class="mt-1 w-full h-8 rounded-md border border-border bg-surface px-2 text-[12px] text-fg" />
          </label>
        </div>

        <label class="block text-[11px] font-medium text-fg-subtle">
          User <span class="font-normal text-fg-subtle">(optional for Host aliases)</span>
          <input bind:value={form.sshUser} class="mt-1 w-full h-8 rounded-md border border-border bg-surface px-2 text-[12px] text-fg" placeholder="deploy, ubuntu, ec2-user, or blank" />
        </label>

        <div class="grid grid-cols-2 gap-2 @max-[380px]:grid-cols-1">
          <label class="block text-[11px] font-medium text-fg-subtle">
            Auth
            <select bind:value={form.authKind} class="mt-1 w-full h-8 rounded-md border border-border bg-surface px-2 text-[12px] text-fg">
              <option value="key">Key / agent</option>
              <option value="password">Password (keychain)</option>
            </select>
          </label>
          <label class="block text-[11px] font-medium text-fg-subtle">
            Type
            <select bind:value={form.forwardKind} class="mt-1 w-full h-8 rounded-md border border-border bg-surface px-2 text-[12px] text-fg">
              <option value="local">Local -L</option>
              <option value="reverse" disabled={proLocked}>Reverse -R</option>
              <option value="socks" disabled={proLocked}>SOCKS -D</option>
            </select>
          </label>
        </div>

        {#if form.authKind === "key"}
          <label class="block text-[11px] font-medium text-fg-subtle">
            Key path
            <input bind:value={form.keyPath} class="mt-1 w-full h-8 rounded-md border border-border bg-surface px-2 text-[12px] text-fg" placeholder="Optional; uses ssh-agent by default" />
          </label>
        {:else}
          <label class="block text-[11px] font-medium text-fg-subtle">
            Password
            <input bind:value={form.password} type="password" class="mt-1 w-full h-8 rounded-md border border-border bg-surface px-2 text-[12px] text-fg" placeholder="Stored only in keychain" />
          </label>
        {/if}

        <div class="grid grid-cols-2 gap-2 @max-[380px]:grid-cols-1">
          <label class="block text-[11px] font-medium text-fg-subtle">
            Local port
            <input bind:value={form.localPort} type="number" min="1" class="mt-1 w-full h-8 rounded-md border border-border bg-surface px-2 text-[12px] text-fg" placeholder="Auto" />
          </label>
          <label class="block text-[11px] font-medium text-fg-subtle">
            Remote port
            <input bind:value={form.remotePort} type="number" min="0" class="mt-1 w-full h-8 rounded-md border border-border bg-surface px-2 text-[12px] text-fg" />
          </label>
        </div>

        <label class="block text-[11px] font-medium text-fg-subtle">
          Remote host
          <input bind:value={form.remoteHost} class="mt-1 w-full h-8 rounded-md border border-border bg-surface px-2 text-[12px] text-fg" placeholder="localhost or db.internal" />
        </label>

        <label class="block text-[11px] font-medium text-fg-subtle">
          ProxyJump
          <input bind:value={form.proxyJump} class="mt-1 w-full h-8 rounded-md border border-border bg-surface px-2 text-[12px] text-fg" placeholder="Optional: user@jump-host" />
        </label>
        <p class="-mt-2 text-[10.5px] text-fg-subtle">
          One jump host is included. Comma-separated multi-hop chains are a Pro profile feature.
        </p>

        <label class="flex items-center gap-2 text-[12px] text-fg-muted">
          <input bind:checked={form.keepAlive} disabled={proLocked} type="checkbox" class="rounded border-border disabled:opacity-50" />
          Keep alive with ServerAliveInterval {proLocked ? "(Pro)" : ""}
        </label>

        <label class="flex items-center gap-2 text-[12px] text-fg-muted">
          <input bind:checked={form.autoReconnect} disabled={proLocked} type="checkbox" class="rounded border-border disabled:opacity-50" />
          Auto-reconnect after drops {proLocked ? "(Pro)" : ""}
        </label>

        <div class="flex items-center gap-2 pt-1">
          <button
            type="submit"
            disabled={sshTunnels.isBusy(form.id ?? "__new")}
            class="flex-1 inline-flex items-center justify-center gap-1.5 h-9 rounded-md
                   text-[12px] font-medium bg-accent text-on-accent hover:brightness-110
                   disabled:opacity-50"
          >
            <Icon name={form.id ? "check" : "plus"} size={12} />
            {form.id ? "Save changes" : "Save profile"}
          </button>
          <button
            type="button"
            onclick={cancelForm}
            class="inline-flex items-center justify-center gap-1.5 h-9 px-4 rounded-md
                   text-[12px] font-medium border border-border text-fg-muted hover:text-fg hover:bg-surface-2"
          >
            Cancel
          </button>
        </div>
      </form>
    </div>
  {:else if detailTunnel}
    {@const selected = detailTunnel}
    <!-- Tunnel detail -->
    <header class="px-8 pt-6 pb-4 border-b border-border/60">
      <button
        type="button"
        onclick={() => openHost(selected.connectionId)}
        class="inline-flex items-center gap-1.5 text-[12px] text-fg-muted hover:text-fg transition-colors"
      >
        <Icon name="chevron-left" size={14} />
        Back to host
      </button>
    </header>

    <div class="px-8 py-6">
      <article class="rounded-lg border border-border/70 bg-surface px-5 py-4">
        <div class="flex items-start gap-3">
          <div class="grid place-items-center w-9 h-9 rounded-lg bg-surface-2 text-fg-muted">
            <Icon name="terminal" size={17} />
          </div>
          <div class="min-w-0 flex-1">
            <div class="flex items-center gap-2">
              <StatusDot status={statusFor(selected)} size="md" pulse={sshTunnels.isBusy(selected.id)} />
              <h2 class="text-[15px] font-semibold text-fg truncate">{selected.name}</h2>
              <span class="rounded bg-surface-2 px-1.5 py-0.5 text-[10.5px] text-fg-muted">
                {selected.state === "reconnecting" ? "Reconnecting" : selected.running ? "Live" : "Down"}
              </span>
            </div>
            <p class="mt-1 font-mono text-[12px] text-fg-subtle truncate">
              {sshDestination(selected)}:{selected.sshPort}
            </p>
          </div>
          <button
            type="button"
            onclick={() => fileBrowser.open(selected.connectionId, sshDestination(selected))}
            class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md text-[12px]
                   font-medium border border-border text-fg-muted
                   hover:bg-surface-2 hover:text-fg"
            title="Browse and transfer files over SFTP"
          >
            <Icon name="folder" size={12} />
            Files
          </button>
          <button
            type="button"
            onclick={() => deployPanel.open(selected.connectionId, sshDestination(selected))}
            class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md text-[12px]
                   font-medium border border-border text-fg-muted
                   hover:bg-surface-2 hover:text-fg"
            title="Run commands / deploy on the remote host"
          >
            <Icon name="terminal" size={12} />
            Run
          </button>
          {#if selected.running}
            <button
              type="button"
              onclick={() => sshTunnels.stop(selected.id)}
              disabled={sshTunnels.isBusy(selected.id)}
              class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md text-[12px]
                     font-medium border border-status-crashed/40 text-status-crashed
                     hover:bg-status-crashed/10 disabled:opacity-50"
            >
              <Icon name="circle-stop" size={12} />
              Stop
            </button>
          {:else}
            <button
              type="button"
              onclick={() => sshTunnels.start(selected.id)}
              disabled={sshTunnels.isBusy(selected.id)}
              class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md text-[12px]
                     font-medium bg-accent text-on-accent hover:brightness-110
                     disabled:opacity-50"
            >
              <Icon name={sshTunnels.isBusy(selected.id) ? "refresh-cw" : "play"} size={12}
                class={sshTunnels.isBusy(selected.id) ? "animate-spin" : ""} />
              Start
            </button>
          {/if}
        </div>

        <dl class="mt-5 grid grid-cols-2 gap-3 text-[12px]">
          <div>
            <dt class="text-fg-subtle">Local endpoint</dt>
            <dd class="mt-0.5 font-mono text-fg">
              {selected.localHost}:{selected.localPort}
            </dd>
          </div>
          <div>
            <dt class="text-fg-subtle">Remote endpoint</dt>
            <dd class="mt-0.5 font-mono text-fg">
              {selected.remoteHost}:{selected.remotePort}
            </dd>
          </div>
          <div>
            <dt class="text-fg-subtle">Jump host</dt>
            <dd class="mt-0.5 font-mono text-fg">{selected.proxyJump ?? "None"}</dd>
          </div>
          <div>
            <dt class="text-fg-subtle">Forward type</dt>
            <dd class="mt-0.5 text-fg">{forwardLabel(selected.forwardKind)}</dd>
          </div>
        </dl>

        <div class="mt-5 rounded-md border border-border/70 bg-surface-2/50 p-3">
          <div class="mb-2 flex items-center justify-between gap-2">
            <span class="text-[11px] font-medium uppercase text-fg-subtle">
              Equivalent command
            </span>
            <button
              type="button"
              onclick={() => copyCommand(selected)}
              class="inline-flex items-center gap-1 rounded px-2 py-1 text-[11px]
                     text-fg-muted hover:text-fg hover:bg-surface"
            >
              <Icon name="copy" size={11} />
              {copied === selected.id ? "Copied" : "Copy"}
            </button>
          </div>
          <code class="block whitespace-pre-wrap break-all font-mono text-[11px] leading-relaxed text-fg">
            {selected.command}
          </code>
        </div>

        <div class="mt-4 rounded-md border border-border/70 bg-surface-2/40 p-3">
          <div class="mb-3 flex items-center justify-between gap-2">
            <div>
              <span class="text-[11px] font-medium uppercase text-fg-subtle">
                Remote files
              </span>
              <p class="mt-0.5 text-[11px] text-fg-subtle">
                Use OpenSSH-compatible tools for editing and deploys; copy a host block first when this profile has a key, port, jump host, or enterprise proxy alias.
              </p>
            </div>
          </div>
          <label class="block text-[11px] font-medium text-fg-subtle">
            Remote path
            <input
              bind:value={remotePath}
              class="mt-1 w-full h-8 rounded-md border border-border bg-surface px-2 font-mono text-[12px] text-fg"
              placeholder="~/app or /var/www/html"
            />
          </label>
          {#if hasExplicitTransportOptions(selected)}
            <div class="mt-3 rounded border border-border/60 bg-surface px-2.5 py-2">
              <div class="flex items-center justify-between gap-2">
                <div class="min-w-0">
                  <p class="text-[11px] font-medium text-fg">OpenSSH host alias</p>
                  <p class="mt-0.5 truncate font-mono text-[10.5px] text-fg-subtle">
                    {sshConfigAlias(selected)}
                  </p>
                </div>
                <button
                  type="button"
                  onclick={() => copyText(`${selected.id}:ssh-config`, sshConfigSnippet(selected), "SSH config host block copied.", "Add it to ~/.ssh/config so VS Code, Cursor, rsync, scp, and OpenSSH share the same connection settings.")}
                  class="inline-flex items-center justify-center gap-1.5 h-8 px-2 rounded-md text-[11px]
                         font-medium border border-border text-fg-muted hover:text-fg hover:bg-surface-2"
                >
                  <Icon name="copy" size={11} />
                  {copied === `${selected.id}:ssh-config` ? "Copied" : "SSH config"}
                </button>
              </div>
              <p class="mt-2 text-[10.5px] text-fg-subtle leading-relaxed">
                Add this block to <code class="font-mono">~/.ssh/config</code> before using editor commands that need this profile's port, key, jump, or keep-alive settings.
              </p>
            </div>
          {/if}
          <p class="mt-2 text-[10.5px] text-fg-subtle leading-relaxed">
            For Teleport, Boundary, Cloudflare Access, AWS SSM, or VPN/private-network hosts, use the provider-generated
            <code class="font-mono">Host</code> alias as SSH host when it already contains
            <code class="font-mono">ProxyCommand</code> or certificates.
          </p>
          <div class="mt-3 grid grid-cols-2 gap-2">
            <button
              type="button"
              onclick={() => copyText(`${selected.id}:code`, remoteEditorCommand(selected, "code"), "VS Code Remote-SSH command copied.", "Open the remote folder through VS Code's Remote-SSH extension.")}
              class="inline-flex items-center justify-center gap-1.5 h-8 px-2 rounded-md text-[11px]
                     font-medium border border-border text-fg-muted hover:text-fg hover:bg-surface"
            >
              <Icon name="copy" size={11} />
              {copied === `${selected.id}:code` ? "Copied" : "VS Code"}
            </button>
            <button
              type="button"
              onclick={() => copyText(`${selected.id}:cursor`, remoteEditorCommand(selected, "cursor"), "Cursor Remote-SSH command copied.", "Open the remote folder through Cursor's VS Code-compatible Remote-SSH flow.")}
              class="inline-flex items-center justify-center gap-1.5 h-8 px-2 rounded-md text-[11px]
                     font-medium border border-border text-fg-muted hover:text-fg hover:bg-surface"
            >
              <Icon name="copy" size={11} />
              {copied === `${selected.id}:cursor` ? "Copied" : "Cursor"}
            </button>
            <button
              type="button"
              onclick={() => copyText(`${selected.id}:rsync`, rsyncUploadCommand(selected), "rsync upload command copied.", "rsync uploads changed files over SSH and is safer for repeated deploys than blind recursive copies.")}
              class="inline-flex items-center justify-center gap-1.5 h-8 px-2 rounded-md text-[11px]
                     font-medium border border-border text-fg-muted hover:text-fg hover:bg-surface"
            >
              <Icon name="copy" size={11} />
              {copied === `${selected.id}:rsync` ? "Copied" : "rsync upload"}
            </button>
            <button
              type="button"
              onclick={() => copyText(`${selected.id}:git`, remoteGitPullCommand(selected), "Remote git pull command copied.", "Run the update on the server when the deploy strategy is Git on the remote host.")}
              class="inline-flex items-center justify-center gap-1.5 h-8 px-2 rounded-md text-[11px]
                     font-medium border border-border text-fg-muted hover:text-fg hover:bg-surface"
            >
              <Icon name="copy" size={11} />
              {copied === `${selected.id}:git` ? "Copied" : "git pull"}
            </button>
          </div>
          <button
            type="button"
            onclick={() => copyText(`${selected.id}:scp`, scpUploadCommand(selected), "scp fallback command copied.", "Use scp only for simple one-off uploads where rsync is unavailable.")}
            class="mt-2 w-full inline-flex items-center justify-center gap-1.5 h-8 px-2 rounded-md text-[11px]
                   font-medium border border-border text-fg-muted hover:text-fg hover:bg-surface"
          >
            <Icon name="copy" size={11} />
            {copied === `${selected.id}:scp` ? "Copied" : "Copy scp fallback"}
          </button>
        </div>

        <div class="mt-4 flex flex-wrap items-center gap-2">
          <button
            type="button"
            onclick={() => sshTunnels.test(selected.id)}
            disabled={sshTunnels.isBusy(selected.id)}
            class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md text-[12px]
                   font-medium border border-border text-fg-muted hover:text-fg hover:bg-surface-2"
          >
            <Icon name="circle-check" size={12} />
            Test SSH
          </button>
          <button
            type="button"
            onclick={() => startEdit(selected)}
            disabled={sshTunnels.isBusy(selected.id)}
            class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md text-[12px]
                   font-medium border border-border text-fg-muted hover:text-fg hover:bg-surface-2"
          >
            <Icon name="pencil" size={12} />
            Edit / re-point
          </button>
          <select
            bind:value={dbEngine}
            class="h-8 rounded-md border border-border bg-surface px-2 text-[12px] text-fg"
          >
            {#each DB_ENGINES as engine}
              <option value={engine.id}>{engine.label}</option>
            {/each}
          </select>
          <button
            type="button"
            onclick={() => sshTunnels.openDatabase({ id: selected.id, engine: dbEngine })}
            disabled={!selected.running || sshTunnels.isBusy(`${selected.id}:db`)}
            class="inline-flex items-center gap-1.5 h-8 px-3 rounded-md text-[12px]
                   font-medium bg-surface-2 text-fg hover:bg-surface-2/70 disabled:opacity-50"
          >
            <Icon name="database" size={12} />
            Open DB client
          </button>
          <button
            type="button"
            onclick={() => removeTunnel(selected.id)}
            disabled={sshTunnels.isBusy(selected.id)}
            class="ml-auto inline-flex items-center gap-1.5 h-8 px-3 rounded-md text-[12px]
                   font-medium text-status-crashed hover:bg-status-crashed/10 disabled:opacity-50"
          >
            <Icon name="trash-2" size={12} />
            Delete
          </button>
        </div>
      </article>
    </div>
  {:else if showHostForm}
    <!-- Add / edit a saved host. Keyed so editing a different host remounts the
         form with fresh seed values. -->
    {#key detailHost?.id ?? "new"}
      <HostConnectionForm
        connection={detailHost}
        onsaved={onHostSaved}
        oncancel={onHostCancel}
      />
    {/key}
  {:else if managingIdentities}
    <!-- Reusable identities manager -->
    <IdentitiesManager onclose={openDashboard} />
  {:else if importingConfig}
    <!-- Import hosts from ~/.ssh/config -->
    <SshConfigImport onclose={openDashboard} ondone={onImportDone} />
  {:else if workspaceHost}
    <!-- Interactive host workspace (full-pane takeover). Keyed by host id so a
         host switch remounts cleanly; the tab is a prop, so switching tabs keeps
         the component (and its snapshot) mounted. -->
    {@const wsHost = workspaceHost}
    {#key wsHost.id}
      <SshWorkspace
        host={wsHost}
        tunnels={hostTunnels}
        onClose={openDashboard}
        onEdit={startEditHost}
        onRemoved={openDashboard}
        onOpenTunnel={openTunnel}
        onAddTunnel={() => startCreateForHost(wsHost)}
      />
    {/key}
  {:else if showDashboard}
    <!-- Host workbench: searchable table (left) + detail panel (right). The
         table stays put while a host's panel slides in, driven by ?host=<id>. -->
    <div class="flex h-full min-w-0">
      <SshHostTable
        connections={sshConnections.value}
        selectedId={detailHost?.id ?? null}
        onselect={openHost}
        onadd={startCreateHost}
        onimport={openImport}
        onmanageIdentities={openIdentities}
        onrefresh={refreshHealth}
        onedit={editHostFromTable}
        ondetectOs={(id) => void sshConnections.detectOs(id)}
        onremove={removeHostFromTable}
      />
      {#if detailHost}
        {@const selectedHost = detailHost}
        {#key selectedHost.id}
          <SshHostPanel
            host={selectedHost}
            tunnels={hostTunnels}
            onClose={openDashboard}
            onEdit={startEditHost}
            onRemoved={openDashboard}
            onOpenTunnel={openTunnel}
            onAddTunnel={() => startCreateForHost(selectedHost)}
            onOpenWorkspace={() => openHost(selectedHost.id)}
          />
        {/key}
      {/if}
    </div>
  {:else}
    <!-- Host-first empty state (no saved connections yet) -->
    <header class="px-8 pt-8 pb-5 border-b border-border/60">
      <div class="flex items-center gap-2.5">
        <Icon name="server" size={18} class="text-accent" />
        <h1 class="text-[17px] font-semibold tracking-tight text-fg">SSH Hosts</h1>
        <button
          type="button"
          onclick={startCreateHost}
          class="ml-auto inline-flex items-center gap-1.5 h-8 px-3.5 rounded-md text-[12px]
                 font-medium text-on-accent bg-accent shadow-sm hover:brightness-110
                 active:brightness-95 transition"
        >
          <Icon name="plus" size={13} />
          Add host
        </button>
      </div>
      <p class="mt-1.5 text-[12.5px] text-fg-muted leading-relaxed max-w-2xl">
        Save a remote host once, then ride tunnels, file transfers, and deploys on it.
      </p>
    </header>

    <div class="px-8 py-6">
      <div class="rounded-xl border border-dashed border-border px-6 py-12 text-center">
        <span class="inline-grid place-items-center w-12 h-12 rounded-xl bg-surface-2 text-fg-subtle mx-auto">
          <Icon name="server" size={24} />
        </span>
        <p class="mt-3 text-[13px] font-medium text-fg">No SSH hosts yet</p>
        <p class="mt-1.5 text-[12px] text-fg-subtle leading-relaxed max-w-md mx-auto">
          Add a bastion, VPS, or cluster login. PortBay stores the host and key
          path only — passwords live in your OS keychain.
        </p>
        <div class="mt-4 flex items-center justify-center gap-2">
          <button
            type="button"
            onclick={startCreateHost}
            class="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg
                   text-[12.5px] text-accent border border-accent/40
                   hover:bg-accent/10 hover:border-accent/60 active:scale-[0.98] transition"
          >
            <Icon name="plus" size={14} />
            Add SSH host
          </button>
          <button
            type="button"
            onclick={openImport}
            class="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg
                   text-[12.5px] text-fg-muted border border-border
                   hover:text-fg hover:bg-surface-2 active:scale-[0.98] transition"
          >
            <Icon name="file-text" size={14} />
            Import from ~/.ssh/config
          </button>
        </div>
      </div>
    </div>
  {/if}
</section>
