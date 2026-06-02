<!--
  SshHostPanel — the SSH Access detail panel (right pane). Shows a selected
  host's connection facts, live host-key fingerprint (from the probe), recent
  activity, and the port-forwards riding on it. The primary
  "Open Host" action triggers the VS Code-style credential prompt, authenticates
  to prove the connection, then hands off to the host workspace (the next screen
  — see the `openWorkspace` seam). Files / Run remain reachable from the split
  menu.
-->
<script lang="ts">
  import HostMark from "$lib/components/atoms/HostMark.svelte";
  import Icon from "$lib/components/atoms/Icon.svelte";
  import HostTunnelsList from "$lib/components/connections/HostTunnelsList.svelte";
  import { safeInvoke, invokeQuiet } from "$lib/ipc";
  import { providerLabel } from "$lib/ssh/providers";
  import {
    absoluteTime,
    authSummary,
    dateLabel,
    destination,
    healthMeta,
    relativeTime,
    stageMeta,
  } from "$lib/ssh/hostFormat";
  import { confirmDialog } from "$lib/stores/confirm.svelte";
  import { deployPanel } from "$lib/stores/deployPanel.svelte";
  import { errorBus } from "$lib/stores/errors.svelte";
  import { fileBrowser } from "$lib/stores/fileBrowser.svelte";
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
    /** Open the interactive host workspace (the primary "Open Host" action). */
    onOpenWorkspace: () => void;
  }
  let {
    host,
    tunnels,
    onClose,
    onEdit,
    onRemoved,
    onOpenTunnel,
    onAddTunnel,
    onOpenWorkspace,
  }: Props = $props();

  let menuOpen = $state(false);
  let actionsOpen = $state(false);
  let copied = $state<string | null>(null);

  const dest = $derived(destination(host));
  const probe = $derived(sshProbe.get(host.id));
  const health = $derived(healthMeta(probe?.health));
  const stage = $derived(stageMeta(host.stage));
  const auth = $derived(authSummary(host));
  const prov = $derived(providerLabel(host.environment));

  // Host-key trust against the local known_hosts (from the probe).
  const TRUST_META = {
    trusted: { label: "Trusted", tone: "text-status-running", hint: "Key matches your known_hosts." },
    new: { label: "New host", tone: "text-fg-muted", hint: "Not yet recorded — trust is set on first connect." },
    changed: { label: "Key changed", tone: "text-status-crashed", hint: "The host key differs from the one recorded in known_hosts." },
    unknown: { label: "Unknown", tone: "text-fg-subtle", hint: "Not probed yet." },
  } as const;
  const trustMeta = $derived(probe ? TRUST_META[probe.trust] : null);
  let resettingTrust = $state(false);
  let hasSavedSecret = $state(false);
  let forgettingSecret = $state(false);

  // Check whether the OS keychain holds a saved secret for this host.
  // Re-runs whenever the host changes. Errors treated as false (best-effort).
  $effect(() => {
    const id = host.id;
    void invokeQuiet<boolean>("ssh_has_stored_credential", { id })
      .then((v) => { hasSavedSecret = v; })
      .catch(() => { hasSavedSecret = false; });
  });

  // Remove this host's known_hosts entry (reset trust). Extra-careful confirm on
  // a "changed" key, since that's also what a real MITM would look like.
  async function resetTrust() {
    const changed = probe?.trust === "changed";
    const choice = await confirmDialog.open({
      title: changed ? "Reset changed host key?" : "Forget this host key?",
      message: changed
        ? "Removes the recorded key from ~/.ssh/known_hosts. Only do this if you know the host legitimately changed (rebuilt server, new key) — a changed key can also signal an interception. The next connection records the new key."
        : "Removes this host's entry from ~/.ssh/known_hosts. The next connection re-establishes trust on first use.",
      destructive: changed,
      icon: "shield",
      actions: [
        { label: changed ? "Reset key" : "Forget host key", value: "go", tone: changed ? "destructive" : undefined, icon: "rotate-ccw" },
      ],
    });
    if (choice !== "go") return;
    resettingTrust = true;
    try {
      const removed = await safeInvoke<number>("ssh_known_host_remove", { id: host.id });
      errorBus.push({
        code: "SSH_KNOWN_HOST_RESET",
        category: "infrastructure",
        whatHappened:
          removed > 0
            ? `Removed ${removed} known_hosts entr${removed === 1 ? "y" : "ies"} for this host.`
            : "No known_hosts entry was found for this host.",
        whyItMatters: "The next connection re-establishes host-key trust (TOFU).",
        whoCausedIt: "user",
        severity: "success",
        actions: [],
      });
      await sshProbe.probe(host.id);
    } catch {
      /* safeInvoke toasted */
    } finally {
      resettingTrust = false;
    }
  }

  // Remove the OS keychain entry for this host's saved password/passphrase.
  async function forgetSecret() {
    const choice = await confirmDialog.open({
      title: "Forget saved secret?",
      message:
        `Removes the saved password or passphrase for "${host.name}" from your OS keychain. ` +
        `The next connection will ask for it again.`,
      destructive: false,
      icon: "key",
      actions: [
        { label: "Forget", value: "go", icon: "trash-2" },
      ],
    });
    if (choice !== "go") return;
    forgettingSecret = true;
    try {
      await safeInvoke("ssh_forget_credentials", { id: host.id });
      hasSavedSecret = await invokeQuiet<boolean>("ssh_has_stored_credential", { id: host.id }).catch(() => false);
    } catch {
      /* safeInvoke toasted */
    } finally {
      forgettingSecret = false;
    }
  }

  // Probe lazily if the table hasn't already (e.g. deep-link to ?host=).
  $effect(() => {
    if (!sshProbe.get(host.id)) void sshProbe.probe(host.id);
  });

  async function copy(key: string, value: string, what: string) {
    try {
      await navigator.clipboard.writeText(value);
      copied = key;
      setTimeout(() => {
        if (copied === key) copied = null;
      }, 1500);
      errorBus.push({
        code: "COPIED",
        category: "infrastructure",
        whatHappened: what,
        whyItMatters: "Copied to your clipboard.",
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });
    } catch {
      /* no clipboard permission */
    }
  }

  // Open Host: stamp last-used and hand off to the interactive host workspace.
  // The actual authenticated connect (and any credential prompt) happens inside
  // the workspace — on its Connect/Refresh, or when a tab opens a session — so
  // we don't prompt twice for the one-shot secret.
  function openHost() {
    void sshConnections.touch(host.id);
    onOpenWorkspace();
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
</script>

<aside class="flex h-full w-[400px] shrink-0 flex-col overflow-hidden border-l border-border/70 bg-surface/40">
  <!-- Svelte can briefly re-render this fragment with `host` undefined while the
       parent's {#key host.id}/{#if detailHost} block tears the panel down on a
       host switch; guard so a stale teardown pass can't read `host.*`. -->
  {#if host}
  <!-- Header -->
  <header class="flex items-start gap-3 px-5 pt-5 pb-4">
    <HostMark environment={host.environment} size={36} class="mt-0.5 shrink-0" />
    <div class="min-w-0 flex-1">
      <div class="flex items-center gap-2">
        <h2 class="min-w-0 truncate text-[16px] font-semibold text-fg">{host.name}</h2>
        <span class="flex shrink-0 items-center gap-1">
          <span class="w-1.5 h-1.5 rounded-full {health.dotClass}"></span>
          <span class="text-[11px] text-fg-muted">{health.label}</span>
        </span>
      </div>
      <p class="mt-0.5 truncate font-mono text-[12px] text-fg-subtle">{host.sshHost}</p>
    </div>

    <div class="relative shrink-0">
      <button
        type="button"
        onclick={() => (menuOpen = !menuOpen)}
        class="rounded-md p-1.5 text-fg-muted hover:bg-surface-2 hover:text-fg"
        aria-label="Host actions"
      >
        <Icon name="more-horizontal" size={16} />
      </button>
      {#if menuOpen}
        <button type="button" class="fixed inset-0 z-10 cursor-default" aria-label="Close menu" onclick={() => (menuOpen = false)}></button>
        <div class="absolute right-0 z-20 mt-1 w-44 rounded-lg border border-border bg-surface p-1 shadow-xl">
          <button type="button" onclick={() => { menuOpen = false; onEdit(); }} class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg">
            <Icon name="pencil" size={13} /> Edit host
          </button>
          <button type="button" onclick={() => { menuOpen = false; void sshConnections.detectOs(host.id); }} disabled={sshConnections.isBusy(`${host.id}:os`)} class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg disabled:opacity-50">
            <Icon name="server" size={13} /> Detect OS
          </button>
          <button type="button" onclick={removeHost} disabled={host.inUse} title={host.inUse ? "Remove this host's tunnels first" : ""} class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-status-crashed hover:bg-status-crashed/10 disabled:opacity-50">
            <Icon name="trash-2" size={13} /> Remove
          </button>
        </div>
      {/if}
    </div>
    <button type="button" onclick={onClose} class="shrink-0 rounded-md p-1.5 text-fg-muted hover:bg-surface-2 hover:text-fg" aria-label="Close panel">
      <Icon name="x" size={16} />
    </button>
  </header>

  <div class="min-h-0 flex-1 overflow-y-auto px-5 pb-5">
    <!-- Info rows -->
    <dl class="space-y-2.5 text-[12.5px]">
      {#snippet row(label: string, value: string, copyKey?: string)}
        <div class="flex items-baseline gap-3">
          <dt class="w-28 shrink-0 text-fg-subtle">{label}</dt>
          <dd class="flex min-w-0 flex-1 items-center gap-1.5 text-fg">
            <span class="min-w-0 truncate {label === 'Host' || label === 'Fingerprint' ? 'font-mono' : ''}">{value}</span>
            {#if copyKey}
              <button type="button" onclick={() => copy(copyKey, value, `${label} copied.`)} class="shrink-0 rounded p-0.5 text-fg-subtle hover:text-fg" aria-label={`Copy ${label}`}>
                <Icon name={copied === copyKey ? "check" : "copy"} size={12} />
              </button>
            {/if}
          </dd>
        </div>
      {/snippet}

      {@render row("Host", host.sshHost, "host")}
      {@render row("Port", String(host.sshPort))}
      {@render row("Username", host.sshUser || "—", host.sshUser ? "user" : undefined)}
      {@render row("Authentication", auth.detail ? `${auth.label} (${auth.detail})` : auth.label)}
      {@render row("Fingerprint", probe?.fingerprint ?? "Not probed yet", probe?.fingerprint ? "fp" : undefined)}
      <div class="flex items-baseline gap-3">
        <dt class="w-28 shrink-0 text-fg-subtle">Host key</dt>
        <dd class="flex min-w-0 flex-1 items-center gap-2">
          {#if trustMeta}
            <span class="inline-flex items-center gap-1 {trustMeta.tone}" title={trustMeta.hint}>
              <Icon name="shield" size={12} /> {trustMeta.label}
            </span>
            {#if probe && probe.trust !== "unknown"}
              <button
                type="button"
                onclick={resetTrust}
                disabled={resettingTrust}
                class="ml-auto inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-[11px] text-fg-muted hover:bg-surface-2 hover:text-fg disabled:opacity-50"
              >
                <Icon name="rotate-ccw" size={11} /> {probe.trust === "changed" ? "Reset key" : "Forget"}
              </button>
            {/if}
          {:else}
            <span class="text-fg-subtle">Not probed yet</span>
          {/if}
        </dd>
      </div>
      {#if hasSavedSecret}
        <div class="flex items-baseline gap-3">
          <dt class="w-28 shrink-0 text-fg-subtle">Saved secret</dt>
          <dd class="flex min-w-0 flex-1 items-center gap-2">
            <span class="inline-flex items-center gap-1 text-fg-muted">
              <Icon name="key" size={12} /> Stored in keychain
            </span>
            <button
              type="button"
              onclick={forgetSecret}
              disabled={forgettingSecret}
              class="ml-auto inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-[11px] text-fg-muted hover:bg-surface-2 hover:text-fg disabled:opacity-50"
            >
              <Icon name="trash-2" size={11} /> Forget saved secret
            </button>
          </dd>
        </div>
      {/if}
      {@render row("Provider / Region", prov ? (host.region ? `${prov} / ${host.region}` : prov) : "—")}
      {@render row("Created", dateLabel(host.createdAt))}
      <div class="flex items-baseline gap-3">
        <dt class="w-28 shrink-0 text-fg-subtle">Last Used</dt>
        <dd class="min-w-0 flex-1 text-fg">
          {relativeTime(host.lastUsed)}
          {#if host.lastUsed}<span class="text-fg-subtle">({absoluteTime(host.lastUsed)})</span>{/if}
        </dd>
      </div>
      {#if stage || (host?.tags ?? []).length}
        <div class="flex items-baseline gap-3">
          <dt class="w-28 shrink-0 text-fg-subtle">Tags</dt>
          <dd class="flex min-w-0 flex-1 flex-wrap items-center gap-1.5">
            {#if stage}
              <span class="inline-flex items-center rounded-md px-1.5 py-0.5 text-[10.5px] font-medium {stage.chipClass}">{stage.label}</span>
            {/if}
            {#each host?.tags ?? [] as tag (tag)}
              <span class="rounded bg-surface-2 px-1.5 py-0.5 text-[10.5px] text-fg-muted">{tag}</span>
            {/each}
          </dd>
        </div>
      {/if}
    </dl>

    <!-- Open Host (primary) + split menu (Files / Run) -->
    <div class="relative mt-5 flex">
      <button
        type="button"
        onclick={openHost}
        class="flex flex-1 items-center justify-center gap-2 h-11 rounded-l-xl bg-accent text-on-accent
               text-[13px] font-semibold hover:brightness-110 active:brightness-95 transition"
      >
        <Icon name="terminal" size={15} />
        Open Host
      </button>
      <button
        type="button"
        onclick={() => (actionsOpen = !actionsOpen)}
        class="grid place-items-center w-11 h-11 rounded-r-xl border-l border-on-accent/20 bg-accent text-on-accent hover:brightness-110"
        aria-label="More open actions"
      >
        <Icon name="chevron-down" size={15} />
      </button>
      {#if actionsOpen}
        <button type="button" class="fixed inset-0 z-10 cursor-default" aria-label="Close" onclick={() => (actionsOpen = false)}></button>
        <div class="absolute right-0 top-full z-20 mt-1 w-44 rounded-lg border border-border bg-surface p-1 shadow-xl">
          <button type="button" onclick={() => { actionsOpen = false; fileBrowser.open(host.id, dest); }} class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg">
            <Icon name="folder" size={13} /> Browse files (SFTP)
          </button>
          <button type="button" onclick={() => { actionsOpen = false; deployPanel.open(host.id, dest); }} class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12.5px] text-fg-muted hover:bg-surface-2 hover:text-fg">
            <Icon name="terminal" size={13} /> Run / deploy
          </button>
        </div>
      {/if}
    </div>

    <!-- Recent activity. Real per-session history lands with the host workspace;
         until then we surface the one signal we have honestly. -->
    <section class="mt-5 rounded-xl border border-border/70 bg-surface px-4 py-3">
      <h3 class="text-[12px] font-semibold text-fg">Recent activity</h3>
      {#if host.lastUsed}
        <div class="mt-2.5 flex items-center gap-2.5">
          <span class="grid place-items-center w-6 h-6 rounded-md bg-surface-2 text-fg-muted"><Icon name="terminal" size={12} /></span>
          <span class="min-w-0 flex-1 truncate font-mono text-[11.5px] text-fg-muted">{dest}</span>
          <span class="shrink-0 text-[11px] text-fg-subtle">{relativeTime(host.lastUsed)}</span>
        </div>
      {:else}
        <p class="mt-2 text-[12px] text-fg-subtle">No sessions yet.</p>
      {/if}
    </section>


    <!-- Port forwards riding on this host -->
    <section class="mt-3">
      <HostTunnelsList {tunnels} {onOpenTunnel} {onAddTunnel} />
    </section>
  </div>
  {/if}
</aside>
