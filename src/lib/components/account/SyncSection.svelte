<!--
  SyncSection — multi-device sync (Pro), end-to-end encrypted. Matches the other
  Settings <section> cards. Recovery-key based: the key is generated/held by the
  Rust+keychain layer; this UI only shows it once to copy and accepts a paste to
  set up another device. Gated on the `sync` entitlement.
-->
<script lang="ts">
  import { onMount } from "svelte";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import { entitlements } from "$lib/stores/entitlements.svelte";
  import { account } from "$lib/stores/account.svelte";
  import { sync } from "$lib/stores/sync.svelte";
  import { confirmDialog } from "$lib/stores/confirm.svelte";
  import { errorBus } from "$lib/stores/errors.svelte";

  type Mode = "idle" | "show-key" | "paste";
  let mode = $state<Mode>("idle");
  let recoveryKey = $state("");
  let pasteValue = $state("");

  const isPro = $derived(entitlements.allows("sync"));
  /** License device-activation cap (Pro = 2). */
  const maxDevices = $derived(entitlements.maxDevices);

  onMount(() => {
    if (isPro) void sync.load();
  });

  function toast(message: string, severity: "success" | "info" = "success") {
    errorBus.push({
      code: "SYNC",
      category: "account-sync",
      whatHappened: message,
      whyItMatters: "",
      whoCausedIt: "user",
      severity,
      actions: [],
    });
  }

  async function setup() {
    recoveryKey = await sync.enable();
    // Register this device against the 2-device license cap (idempotent).
    try {
      await sync.activate();
    } catch {
      /* device-limit or transient — safeInvoke toasted; the key is still generated */
    }
    mode = "show-key";
  }

  async function showKey() {
    const k = await sync.getRecoveryKey();
    if (k) {
      recoveryKey = k;
      mode = "show-key";
    }
  }

  async function copyKey() {
    try {
      await navigator.clipboard.writeText(recoveryKey);
      toast("Recovery key copied. Store it somewhere safe.");
    } catch {
      toast("Couldn't access the clipboard — select and copy the key manually.", "info");
    }
  }

  async function submitPaste() {
    const key = pasteValue.trim();
    if (!key) return;
    await sync.setRecoveryKey(key);
    pasteValue = "";
    mode = "idle";
    // Activate this device against the 2-device license cap before pulling. A
    // blocked 3rd device gets the DEVICE_LIMIT_REACHED toast and stops here.
    try {
      await sync.activate();
    } catch {
      return; // safeInvoke toasted (device limit or transient)
    }
    // Pull the existing data for this account onto the new device.
    try {
      const pulled = await sync.pull();
      toast(pulled ? "This device is connected — your projects were pulled in." : "This device is connected to sync.");
    } catch {
      /* safeInvoke toasted */
    }
  }

  async function syncNow() {
    const out = await sync.push(false);
    if (out.result === "conflict") {
      const choice = await confirmDialog.open({
        title: "Newer data in the cloud",
        message:
          "Another device has synced more recently. Pull that version in, or overwrite it with this device's projects?",
        actions: [
          { label: "Pull cloud version", value: "pull", tone: "primary" },
          { label: "Overwrite with this device", value: "overwrite", tone: "destructive" },
        ],
      });
      if (choice === "pull") {
        await sync.pull();
        toast("Pulled the latest from the cloud.");
      } else if (choice === "overwrite") {
        await sync.push(true);
        toast("This device's projects are now the synced version.");
      }
      return;
    }
    toast("Synced to the cloud.");
  }

  async function pullNow() {
    const pulled = await sync.pull();
    toast(pulled ? "Pulled the latest from the cloud." : "Nothing to pull yet — sync this device first.", pulled ? "success" : "info");
  }

  async function turnOff() {
    const ok = await confirmDialog.open({
      title: "Turn off sync on this device?",
      message:
        "This device will stop syncing and forget its recovery key. Your cloud data and other devices are untouched. You'll need the recovery key to reconnect.",
      actions: [{ label: "Turn off sync here", value: "off", tone: "destructive" }],
    });
    if (ok === "off") {
      await sync.disable();
      mode = "idle";
      toast("Sync turned off on this device.");
    }
  }

  async function revoke(id: string, name: string) {
    const ok = await confirmDialog.open({
      title: `Revoke "${name}"?`,
      message: "That device will no longer be able to sync until it reconnects with the recovery key.",
      actions: [{ label: "Revoke device", value: "revoke", tone: "destructive" }],
    });
    if (ok === "revoke") {
      await sync.revokeDevice(id);
      toast("Device revoked.");
    }
  }
</script>

<section class="bg-surface border border-border rounded-2xl p-5 grid grid-cols-[180px,1fr] gap-x-6">
  <div class="flex items-start gap-2.5">
    <span class="inline-flex items-center justify-center w-8 h-8 rounded-lg bg-fg-muted/10 text-fg-muted">
      <Icon name="cloud" size={15} />
    </span>
    <div class="pt-1">
      <span class="text-[14px] font-semibold text-fg">Sync</span>
      <span class="block text-[11px] text-fg-subtle mt-0.5">Pro</span>
    </div>
  </div>

  <div class="space-y-4">
    {#if !isPro}
      <!-- upsell -->
      <div class="flex items-start gap-3">
        <p class="text-[13px] leading-relaxed text-fg-muted flex-1">
          Keep your projects in sync across every Mac, end-to-end encrypted. Multi-device sync is part of
          <span class="text-fg font-medium">PortBay Pro</span>.
        </p>
        <button
          type="button"
          onclick={() => account.open({ intent: "pro" })}
          class="shrink-0 inline-flex items-center gap-1.5 h-9 px-4 rounded-lg bg-accent text-on-accent text-[13px] font-semibold hover:brightness-110 transition shadow-sm"
        >
          <Icon name="sparkles" size={13} /> Upgrade
        </button>
      </div>
    {:else if !sync.state.enabled && mode !== "paste"}
      <!-- not set up -->
      <p class="text-[13px] leading-relaxed text-fg-muted">
        Set up sync on this device. We'll generate a <span class="text-fg">recovery key</span> that encrypts your
        projects before they leave your Mac — we can't read them. Save the key to add other devices.
      </p>
      <div class="flex flex-wrap items-center gap-2">
        <button
          type="button"
          onclick={setup}
          disabled={sync.busy}
          class="inline-flex items-center gap-1.5 h-9 px-4 rounded-lg bg-accent text-on-accent text-[13px] font-semibold hover:brightness-110 transition shadow-sm disabled:opacity-60"
        >
          <Icon name="cloud" size={13} /> Set up sync
        </button>
        <button
          type="button"
          onclick={() => (mode = "paste")}
          class="inline-flex items-center gap-1.5 h-9 px-3 rounded-lg text-[13px] font-medium text-fg-muted hover:text-fg hover:bg-surface-2 transition"
        >
          Add this device with a key
        </button>
      </div>
    {/if}

    {#if isPro && mode === "show-key"}
      <!-- reveal the recovery key -->
      <div class="rounded-xl border border-border bg-surface-2/50 p-4">
        <div class="flex items-center gap-2 text-[12px] font-semibold text-fg mb-2">
          <Icon name="lock" size={13} /> Your recovery key
        </div>
        <code class="block text-[12px] text-fg break-all bg-bg border border-border rounded-lg p-2.5 select-all">{recoveryKey}</code>
        <p class="mt-2 text-[11.5px] leading-relaxed text-fg-muted">
          Save this somewhere safe (a password manager). It's the only way to read your synced data or add another
          device — <span class="text-fg">we can't recover it for you</span>.
        </p>
        <div class="mt-3 flex items-center gap-2">
          <button
            type="button"
            onclick={copyKey}
            class="inline-flex items-center gap-1.5 h-8 px-3 rounded-lg bg-accent text-on-accent text-[12.5px] font-semibold hover:brightness-110 transition"
          >
            <Icon name="file-text" size={12} /> Copy key
          </button>
          <button
            type="button"
            onclick={() => (mode = "idle")}
            class="h-8 px-3 rounded-lg text-[12.5px] font-medium text-fg-muted hover:text-fg hover:bg-surface-2 transition"
          >
            I've saved it
          </button>
        </div>
      </div>
    {/if}

    {#if isPro && mode === "paste"}
      <!-- paste a key from another device -->
      <div class="rounded-xl border border-border bg-surface-2/50 p-4">
        <label for="recovery-paste" class="block text-[12px] font-semibold text-fg mb-2">
          Paste the recovery key from your other device
        </label>
        <input
          id="recovery-paste"
          type="text"
          bind:value={pasteValue}
          placeholder="Recovery key"
          class="w-full h-10 px-3 rounded-lg bg-bg border border-border text-[13px] text-fg placeholder:text-fg-subtle focus:outline-none focus:border-accent focus:ring-1 focus:ring-accent/40 transition font-mono"
        />
        <div class="mt-3 flex items-center gap-2">
          <button
            type="button"
            onclick={submitPaste}
            disabled={!pasteValue.trim() || sync.busy}
            class="inline-flex items-center gap-1.5 h-8 px-3 rounded-lg bg-accent text-on-accent text-[12.5px] font-semibold hover:brightness-110 transition disabled:opacity-50"
          >
            Connect this device
          </button>
          <button
            type="button"
            onclick={() => { mode = "idle"; pasteValue = ""; }}
            class="h-8 px-3 rounded-lg text-[12.5px] font-medium text-fg-muted hover:text-fg hover:bg-surface-2 transition"
          >
            Cancel
          </button>
        </div>
      </div>
    {/if}

    {#if isPro && sync.state.enabled && mode === "idle"}
      <!-- active sync controls -->
      <div class="flex items-center justify-between gap-3">
        <div class="flex items-center gap-2 text-[12.5px] text-status-running">
          <Icon name="circle-check" size={14} />
          <span>Sync is on for this device{sync.state.last_version > 0 ? ` · v${sync.state.last_version}` : ""}.</span>
        </div>
      </div>
      <div class="flex flex-wrap items-center gap-2">
        <button
          type="button"
          onclick={syncNow}
          disabled={sync.busy}
          class="inline-flex items-center gap-1.5 h-9 px-4 rounded-lg bg-accent text-on-accent text-[13px] font-semibold hover:brightness-110 transition shadow-sm disabled:opacity-60"
        >
          <Icon name="refresh-cw" size={13} /> Sync now
        </button>
        <button
          type="button"
          onclick={pullNow}
          disabled={sync.busy}
          class="inline-flex items-center gap-1.5 h-9 px-3 rounded-lg border border-border text-[13px] font-medium text-fg hover:bg-surface-2 transition disabled:opacity-60"
        >
          <Icon name="chevron-down" size={13} /> Pull from cloud
        </button>
        <button
          type="button"
          onclick={showKey}
          class="inline-flex items-center gap-1.5 h-9 px-3 rounded-lg text-[13px] font-medium text-fg-muted hover:text-fg hover:bg-surface-2 transition"
        >
          <Icon name="lock" size={13} /> Show recovery key
        </button>
      </div>

      <!-- devices -->
      {#if sync.devices.length > 0}
        <div class="border-t border-border/60 pt-3">
          <div class="flex items-center justify-between mb-2">
            <span class="text-[12px] font-medium text-fg-muted">Devices</span>
            <span class="text-[11.5px] text-fg-subtle">Active on {sync.devices.length} of {maxDevices}</span>
          </div>
          <ul class="space-y-1.5">
            {#each sync.devices as d (d.id)}
              <li class="flex items-center justify-between gap-3 text-[12.5px]">
                <span class="text-fg truncate">{d.name} <span class="text-fg-subtle">· {d.platform}</span></span>
                <button
                  type="button"
                  onclick={() => revoke(d.id, d.name)}
                  class="shrink-0 h-7 px-2.5 rounded-md text-[12px] font-medium text-fg-muted hover:text-status-crashed hover:bg-surface-2 transition"
                >
                  Revoke
                </button>
              </li>
            {/each}
          </ul>
        </div>
      {/if}

      <div class="border-t border-border/60 pt-3">
        <button
          type="button"
          onclick={turnOff}
          class="inline-flex items-center gap-1.5 h-8 px-3 rounded-lg text-[12.5px] font-medium text-fg-muted hover:text-status-crashed hover:bg-surface-2 transition"
        >
          <Icon name="x" size={13} /> Turn off sync on this device
        </button>
      </div>
    {/if}
  </div>
</section>
