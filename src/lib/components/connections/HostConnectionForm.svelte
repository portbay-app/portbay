<!--
  HostConnectionForm — add or edit a saved SSH connection (host + auth + display
  metadata) for the connections dashboard. This is the "+" / Edit surface; it
  saves a standalone connection, independent of any tunnel.
-->
<script lang="ts">
  import { onMount, untrack } from "svelte";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import { sshConnections } from "$lib/stores/sshConnections.svelte";
  import { sshIdentities } from "$lib/stores/sshIdentities.svelte";
  import type {
    SaveSshConnectionInput,
    SshConnectionView,
    SshProxyKind,
  } from "$lib/types/sshConnections";
  import type { SshAuthKind } from "$lib/types/sshTunnels";

  interface Props {
    connection?: SshConnectionView | null;
    onsaved: (saved: SshConnectionView) => void;
    oncancel: () => void;
  }
  let { connection = null, onsaved, oncancel }: Props = $props();

  // A small fixed palette so the JIT keeps the swatch backgrounds; `null` = none.
  const PALETTE: string[] = [
    "#4d9cff",
    "#a855f7",
    "#22c55e",
    "#f97316",
    "#ef4444",
    "#eab308",
    "#9ca3af",
  ];

  const editing = untrack(() => connection !== null);

  // Seed editable form state from the connection once (untrack), matching the
  // CardEditor pattern; the parent keys this component per host so it remounts.
  let name = $state(untrack(() => connection?.name ?? ""));
  let sshHost = $state(untrack(() => connection?.sshHost ?? ""));
  let sshPort = $state<number>(untrack(() => connection?.sshPort ?? 22));
  let sshUser = $state(untrack(() => connection?.sshUser ?? ""));
  let authKind = $state<SshAuthKind>(untrack(() => connection?.authKind ?? "key"));
  let keyPath = $state(untrack(() => connection?.keyPath ?? ""));
  let proxyJump = $state(untrack(() => connection?.proxyJump ?? ""));
  let password = $state("");

  // Optional forward proxy (Advanced). Collapsed unless the host already has one.
  let proxyEnabled = $state(untrack(() => connection?.proxy != null));
  let proxyKind = $state<SshProxyKind>(untrack(() => connection?.proxy?.kind ?? "socks5"));
  let proxyHost = $state(untrack(() => connection?.proxy?.host ?? ""));
  let proxyPort = $state<number>(untrack(() => connection?.proxy?.port ?? 1080));
  let proxyUser = $state(untrack(() => connection?.proxy?.username ?? ""));
  let proxyPassword = $state("");
  let advancedOpen = $state(untrack(() => connection?.proxy != null));
  let tagsText = $state(untrack(() => (connection?.tags ?? []).join(", ")));
  let environment = $state<string>(untrack(() => connection?.environment ?? ""));
  let stage = $state<string>(untrack(() => connection?.stage ?? ""));
  let region = $state<string>(untrack(() => connection?.region ?? ""));
  let color = $state<string | null>(untrack(() => connection?.color ?? null));

  // Environment options grouped for the brand-mark select. "" = Auto-detect
  // (Detect OS sets it; a manual pick here overrides).
  const ENV_GROUPS: { label: string; options: { id: string; name: string }[] }[] = [
    {
      label: "Control panel",
      options: [
        { id: "cpanel", name: "cPanel" },
        { id: "plesk", name: "Plesk" },
        { id: "directadmin", name: "DirectAdmin" },
        { id: "cyberpanel", name: "CyberPanel" },
        { id: "webmin", name: "Webmin" },
      ],
    },
    {
      label: "Operating system",
      options: [
        { id: "ubuntu", name: "Ubuntu" },
        { id: "debian", name: "Debian" },
        { id: "alpine", name: "Alpine" },
        { id: "rhel", name: "RHEL / Rocky / Alma" },
        { id: "centos", name: "CentOS" },
        { id: "fedora", name: "Fedora" },
        { id: "amazonlinux", name: "Amazon Linux" },
        { id: "arch", name: "Arch" },
      ],
    },
    {
      label: "Cloud",
      options: [
        { id: "aws", name: "AWS" },
        { id: "digitalocean", name: "DigitalOcean" },
        { id: "gcp", name: "Google Cloud" },
        { id: "azure", name: "Azure" },
        { id: "hetzner", name: "Hetzner" },
        { id: "linode", name: "Linode" },
      ],
    },
  ];
  let notes = $state(untrack(() => connection?.notes ?? ""));
  let identityId = $state<string>(untrack(() => connection?.identityId ?? ""));

  // A chosen identity supplies user / key / auth; hide those fields then.
  const usesIdentity = $derived(identityId !== "");

  onMount(() => {
    if (!sshIdentities.loaded) void sshIdentities.refresh();
  });

  const key = $derived(connection?.id ?? "__new");
  const busy = $derived(sshConnections.isBusy(key));

  // A configured proxy needs a host; an empty host means "no proxy".
  const proxyActive = $derived(proxyEnabled && proxyHost.trim() !== "");

  async function submit() {
    const input: SaveSshConnectionInput = {
      id: connection?.id ?? null,
      name: name.trim(),
      sshHost: sshHost.trim(),
      sshPort: Number(sshPort) || 22,
      sshUser: sshUser.trim(),
      authKind,
      keyPath: keyPath.trim() || null,
      proxyJump: proxyJump.trim() || null,
      identityId: identityId || null,
      proxy: proxyActive
        ? {
            kind: proxyKind,
            host: proxyHost.trim(),
            port: Number(proxyPort) || (proxyKind === "socks5" ? 1080 : 8080),
            username: proxyUser.trim() || null,
          }
        : null,
      tags: tagsText
        .split(",")
        .map((t) => t.trim())
        .filter(Boolean),
      color,
      notes: notes.trim() || null,
      environment: environment || null,
      stage: stage || null,
      region: region.trim() || null,
      password: !usesIdentity && authKind === "password" ? password : null,
      // Only meaningful for an authenticated proxy; blank preserves the stored one.
      proxyPassword: proxyActive && proxyUser.trim() ? proxyPassword : null,
    };
    const saved = await sshConnections.save(input);
    if (saved) onsaved(saved);
  }
</script>

<section class="h-full min-w-0 overflow-y-auto">
  <header class="px-8 pt-6 pb-4 border-b border-border/60">
    <button
      type="button"
      onclick={oncancel}
      class="inline-flex items-center gap-1.5 text-[12px] text-fg-muted hover:text-fg transition-colors"
    >
      <Icon name="chevron-left" size={14} />
      Back
    </button>
    <h1 class="mt-3 text-[17px] font-semibold tracking-tight text-fg">
      {editing ? "Edit host" : "Add SSH host"}
    </h1>
    <p class="mt-1 text-[12.5px] text-fg-muted">
      Saved hosts hold connection + auth once; tunnels, files, and deploys reuse them.
    </p>
  </header>

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
        <input bind:value={name} class="mt-1 w-full h-8 rounded-md border border-border bg-surface px-2 text-[12px] text-fg" placeholder="Production bastion" />
      </label>

      <div class="grid grid-cols-[1fr_74px] gap-2 @max-[380px]:grid-cols-1">
        <label class="block text-[11px] font-medium text-fg-subtle">
          SSH host
          <input bind:value={sshHost} class="mt-1 w-full h-8 rounded-md border border-border bg-surface px-2 text-[12px] text-fg" placeholder="bastion.example.com or Host alias" />
        </label>
        <label class="block text-[11px] font-medium text-fg-subtle">
          Port
          <input bind:value={sshPort} type="number" min="1" class="mt-1 w-full h-8 rounded-md border border-border bg-surface px-2 text-[12px] text-fg" />
        </label>
      </div>

      {#if sshIdentities.count > 0}
        <label class="block text-[11px] font-medium text-fg-subtle">
          Identity <span class="font-normal text-fg-subtle">(reuse a saved login)</span>
          <select bind:value={identityId} class="mt-1 w-full h-8 rounded-md border border-border bg-surface px-2 text-[12px] text-fg">
            <option value="">None — set auth on this host</option>
            {#each sshIdentities.value as ident (ident.id)}
              <option value={ident.id}>{ident.name}{ident.sshUser ? ` (${ident.sshUser})` : ""}</option>
            {/each}
          </select>
        </label>
      {/if}

      <label class="block text-[11px] font-medium text-fg-subtle">
        User
        <span class="font-normal text-fg-subtle">
          {usesIdentity ? "(optional — overrides the identity)" : "(optional for Host aliases)"}
        </span>
        <input bind:value={sshUser} class="mt-1 w-full h-8 rounded-md border border-border bg-surface px-2 text-[12px] text-fg" placeholder="deploy, ubuntu, ec2-user, or blank" />
      </label>

      {#if usesIdentity}
        <p class="text-[11px] text-fg-subtle leading-relaxed">
          Auth (method + key) comes from the selected identity. Leave the user blank to inherit it too.
        </p>
      {:else}
        <label class="block text-[11px] font-medium text-fg-subtle">
          Auth
          <select bind:value={authKind} class="mt-1 w-full h-8 rounded-md border border-border bg-surface px-2 text-[12px] text-fg">
            <option value="key">Key</option>
            <option value="agent">SSH agent</option>
            <option value="password">Password (keychain)</option>
          </select>
        </label>

        {#if authKind === "password"}
          <label class="block text-[11px] font-medium text-fg-subtle">
            Password
            <input bind:value={password} type="password" class="mt-1 w-full h-8 rounded-md border border-border bg-surface px-2 text-[12px] text-fg" placeholder={editing ? "Leave blank to keep current" : "Stored only in keychain"} />
          </label>
        {:else if authKind === "key"}
          <label class="block text-[11px] font-medium text-fg-subtle">
            Key path
            <input bind:value={keyPath} class="mt-1 w-full h-8 rounded-md border border-border bg-surface px-2 text-[12px] text-fg" placeholder="Optional; falls back to your SSH agent" />
          </label>
        {/if}
      {/if}

      <label class="block text-[11px] font-medium text-fg-subtle">
        ProxyJump <span class="font-normal text-fg-subtle">(optional)</span>
        <input bind:value={proxyJump} class="mt-1 w-full h-8 rounded-md border border-border bg-surface px-2 text-[12px] text-fg" placeholder="user@jump-host or bastion1,bastion2" />
        <span class="mt-1 block font-normal text-[10.5px] text-fg-subtle leading-relaxed">
          Chain multiple hops with commas (OpenSSH <code>-J</code> syntax): <code>jump1,user@jump2:2222</code>. Jump hosts authenticate with your key/agent.
        </span>
      </label>

      <div class="rounded-md border border-border/70">
        <button
          type="button"
          onclick={() => (advancedOpen = !advancedOpen)}
          class="flex w-full items-center justify-between px-2.5 h-8 text-[11px] font-medium text-fg-subtle hover:text-fg"
          aria-expanded={advancedOpen}
        >
          <span>Advanced — proxy</span>
          <Icon name={advancedOpen ? "chevron-down" : "chevron-right"} size={14} />
        </button>

        {#if advancedOpen}
          <div class="border-t border-border/60 px-2.5 py-3 space-y-2.5">
            <label class="flex items-center gap-2 text-[11px] font-medium text-fg-subtle">
              <input type="checkbox" bind:checked={proxyEnabled} class="rounded border-border" />
              Connect through a SOCKS5 / HTTP proxy
            </label>

            {#if proxyEnabled}
              <div class="grid grid-cols-[110px_1fr_74px] gap-2 @max-[380px]:grid-cols-1">
                <label class="block text-[11px] font-medium text-fg-subtle">
                  Type
                  <select bind:value={proxyKind} class="mt-1 w-full h-8 rounded-md border border-border bg-surface px-2 text-[12px] text-fg">
                    <option value="socks5">SOCKS5</option>
                    <option value="http">HTTP CONNECT</option>
                  </select>
                </label>
                <label class="block text-[11px] font-medium text-fg-subtle">
                  Proxy host
                  <input bind:value={proxyHost} class="mt-1 w-full h-8 rounded-md border border-border bg-surface px-2 text-[12px] text-fg" placeholder="10.0.0.1" />
                </label>
                <label class="block text-[11px] font-medium text-fg-subtle">
                  Port
                  <input bind:value={proxyPort} type="number" min="1" class="mt-1 w-full h-8 rounded-md border border-border bg-surface px-2 text-[12px] text-fg" />
                </label>
              </div>

              <div class="grid grid-cols-2 gap-2 @max-[380px]:grid-cols-1">
                <label class="block text-[11px] font-medium text-fg-subtle">
                  Proxy user <span class="font-normal text-fg-subtle">(optional)</span>
                  <input bind:value={proxyUser} class="mt-1 w-full h-8 rounded-md border border-border bg-surface px-2 text-[12px] text-fg" placeholder="Leave blank for an open proxy" />
                </label>
                {#if proxyUser.trim()}
                  <label class="block text-[11px] font-medium text-fg-subtle">
                    Proxy password
                    <input bind:value={proxyPassword} type="password" class="mt-1 w-full h-8 rounded-md border border-border bg-surface px-2 text-[12px] text-fg" placeholder={editing ? "Leave blank to keep current" : "Stored only in keychain"} />
                  </label>
                {/if}
              </div>
            {/if}
          </div>
        {/if}
      </div>

      <label class="block text-[11px] font-medium text-fg-subtle">
        Tags <span class="font-normal text-fg-subtle">(comma-separated)</span>
        <input bind:value={tagsText} class="mt-1 w-full h-8 rounded-md border border-border bg-surface px-2 text-[12px] text-fg" placeholder="prod, db, eu-west" />
      </label>

      <label class="block text-[11px] font-medium text-fg-subtle">
        Provider <span class="font-normal text-fg-subtle">(shows a brand mark; auto-detected on Detect OS)</span>
        <select bind:value={environment} class="mt-1 w-full h-8 rounded-md border border-border bg-surface px-2 text-[12px] text-fg">
          <option value="">Auto-detect (on Detect OS)</option>
          {#each ENV_GROUPS as group (group.label)}
            <optgroup label={group.label}>
              {#each group.options as opt (opt.id)}
                <option value={opt.id}>{opt.name}</option>
              {/each}
            </optgroup>
          {/each}
        </select>
      </label>

      <div class="grid grid-cols-2 gap-2 @max-[380px]:grid-cols-1">
        <label class="block text-[11px] font-medium text-fg-subtle">
          Environment <span class="font-normal text-fg-subtle">(tier)</span>
          <select bind:value={stage} class="mt-1 w-full h-8 rounded-md border border-border bg-surface px-2 text-[12px] text-fg">
            <option value="">None</option>
            <option value="production">Production</option>
            <option value="staging">Staging</option>
            <option value="research">Research</option>
            <option value="sandbox">Sandbox</option>
          </select>
        </label>
        <label class="block text-[11px] font-medium text-fg-subtle">
          Region <span class="font-normal text-fg-subtle">(optional)</span>
          <input bind:value={region} class="mt-1 w-full h-8 rounded-md border border-border bg-surface px-2 text-[12px] text-fg" placeholder="us-east-1, nyc3, …" />
        </label>
      </div>

      <div class="block text-[11px] font-medium text-fg-subtle">
        Colour
        <div role="radiogroup" aria-label="Host colour" class="mt-1.5 flex items-center gap-2">
          <button
            type="button"
            role="radio"
            aria-checked={color === null}
            aria-label="No colour"
            title="No colour"
            onclick={() => (color = null)}
            class="inline-flex items-center justify-center w-6 h-6 rounded-full border border-border text-fg-subtle
                   {color === null ? 'ring-2 ring-offset-2 ring-offset-surface ring-accent/60' : ''}"
          >
            <Icon name="x" size={11} />
          </button>
          {#each PALETTE as hex (hex)}
            {@const selected = color === hex}
            <button
              type="button"
              role="radio"
              aria-checked={selected}
              aria-label={hex}
              title={hex}
              onclick={() => (color = hex)}
              class="inline-flex w-6 h-6 rounded-full transition-transform hover:-translate-y-0.5
                     {selected ? 'ring-2 ring-offset-2 ring-offset-surface' : ''}"
              style:background-color={hex}
              style:--tw-ring-color={hex}
            ></button>
          {/each}
        </div>
      </div>

      <label class="block text-[11px] font-medium text-fg-subtle">
        Notes <span class="font-normal text-fg-subtle">(optional)</span>
        <textarea bind:value={notes} rows="2" class="mt-1 w-full rounded-md border border-border bg-surface px-2 py-1.5 text-[12px] text-fg" placeholder="Anything worth remembering about this host"></textarea>
      </label>

      <div class="flex items-center gap-2 pt-1">
        <button
          type="submit"
          disabled={busy}
          class="flex-1 inline-flex items-center justify-center gap-1.5 h-9 rounded-md
                 text-[12px] font-medium bg-accent text-on-accent hover:brightness-110
                 disabled:opacity-50"
        >
          <Icon name={editing ? "check" : "plus"} size={12} />
          {editing ? "Save changes" : "Add host"}
        </button>
        <button
          type="button"
          onclick={oncancel}
          class="inline-flex items-center justify-center gap-1.5 h-9 px-4 rounded-md
                 text-[12px] font-medium border border-border text-fg-muted hover:text-fg hover:bg-surface-2"
        >
          Cancel
        </button>
      </div>
    </form>
  </div>
</section>
