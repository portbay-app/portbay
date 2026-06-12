<!--
  MobileDestinationPicker — the Xcode-style pre-run ritual, in the rail.

  A device chip (platform icon + name + boot dot) opens a grouped, searchable
  popover: Recent → Ready (booted/connected) → Simulators → Emulators →
  Physical devices. Selecting persists to `MobileRunConfig.device` via
  `update_project` (the backend re-stamps the launch script), so the row's
  plain Play runs the same destination.

  Enumeration happens on open (and on the refresh button), plus one scan on
  mount when a device is pinned — the chip must resolve the persisted id to
  the real device name ("Nour's iPhone", not the UDID) without the user ever
  opening the popover. Automatic stays fully passive: the backend shells out
  to simctl/adb/flutter for enumeration, so we never scan with nothing to
  resolve. A pinned destination that's no longer enumerable shows an explicit
  warning instead of being silently substituted (the Expo-Orbit bug the plan
  calls out).
-->
<script lang="ts">
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import Popover from "$lib/components/atoms/Popover.svelte";
  import StatusDot from "$lib/components/atoms/StatusDot.svelte";

  import { safeInvoke } from "$lib/ipc";
  import { mobileDeviceNames } from "$lib/stores/mobileDeviceNames.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import type { ProjectView } from "$lib/types/projects";
  import { groupTargets, type MobileRunTarget } from "$lib/types/mobile";

  interface Props {
    project: ProjectView;
  }
  let { project }: Props = $props();

  let targets = $state<MobileRunTarget[] | null>(null);
  let loading = $state(false);
  let query = $state("");
  let saving = $state(false);

  // ---- Android Wi-Fi pairing (QR + manual code) ----
  // Offered for kinds that can run on an Android device. The QR flow mirrors
  // Android Studio: phone scans → backend pairs + connects → re-enumerate.
  const androidCapable = $derived(
    project.type === "android" ||
      project.type === "flutter" ||
      project.type === "expo",
  );
  type PairView = "list" | "qr" | "manual";
  let pairView = $state<PairView>("list");
  let pairQr = $state<{ qrSvg: string; password: string } | null>(null);
  let pairStatus = $state<string | null>(null);
  let pairDone = $state(false);
  let pairManualHost = $state("");
  let pairManualCode = $state("");
  let pairBusy = $state(false);
  let unlistenPair: UnlistenFn | null = null;

  type PairEvent =
    | { stage: "pairing"; address: string }
    | { stage: "connecting" }
    | { stage: "connected"; serial: string }
    | { stage: "failed"; message: string };

  async function startQrPairing() {
    pairView = "qr";
    pairQr = null;
    pairDone = false;
    pairStatus = "Generating code…";
    try {
      unlistenPair ??= await listen<PairEvent>("portbay://adb-pair", (e) => {
        const ev = e.payload;
        if (ev.stage === "pairing") pairStatus = `Phone found (${ev.address}) — pairing…`;
        else if (ev.stage === "connecting") pairStatus = "Paired — connecting…";
        else if (ev.stage === "connected") {
          pairStatus = `Connected (${ev.serial}).`;
          pairDone = true;
          void refresh();
        } else {
          pairStatus = ev.message;
          pairDone = true;
        }
      });
      pairQr = await safeInvoke<{ qrSvg: string; password: string }>(
        "android_wifi_pair_start",
      );
      pairStatus = "Waiting for the phone to scan…";
    } catch {
      pairStatus = null;
      pairView = "list"; // toast already pushed
    }
  }

  async function submitManualPair() {
    if (pairBusy) return;
    pairBusy = true;
    pairStatus = "Pairing…";
    pairDone = false;
    try {
      const msg = await safeInvoke<string>("android_wifi_pair_manual", {
        hostPort: pairManualHost,
        code: pairManualCode,
      });
      pairStatus = msg;
      pairDone = true;
      void refresh();
    } catch {
      pairStatus = null; // toast already carries the error
    } finally {
      pairBusy = false;
    }
  }

  function closePairing() {
    pairView = "list";
    pairQr = null;
    pairStatus = null;
    pairDone = false;
  }

  $effect(() => {
    return () => {
      if (unlistenPair) {
        unlistenPair();
        unlistenPair = null;
      }
    };
  });

  const pinned = $derived(project.mobileRun?.device ?? null);

  // Pinned-but-gone: only judged against a *loaded* enumeration.
  const pinnedMissing = $derived(
    pinned !== null &&
      targets !== null &&
      !targets.some((t) => t.id === pinned),
  );

  const pinnedTarget = $derived(
    pinned === null ? null : (targets?.find((t) => t.id === pinned) ?? null),
  );

  const chipLabel = $derived(
    pinnedTarget?.name ?? mobileDeviceNames.label(pinned),
  );

  const recentsKey = $derived(`portbay:mobile-recents:${project.id}`);

  function loadRecents(): string[] {
    try {
      const raw = localStorage.getItem(recentsKey);
      const list = raw ? (JSON.parse(raw) as unknown) : [];
      return Array.isArray(list) ? list.filter((x) => typeof x === "string") : [];
    } catch {
      return [];
    }
  }

  function pushRecent(id: string) {
    const list = [id, ...loadRecents().filter((x) => x !== id)].slice(0, 5);
    try {
      localStorage.setItem(recentsKey, JSON.stringify(list));
    } catch {
      /* storage full / unavailable — recents are a nicety */
    }
  }

  async function refresh() {
    if (loading) return;
    loading = true;
    try {
      targets = await safeInvoke<MobileRunTarget[]>("list_mobile_run_targets", {
        id: project.id,
      });
      mobileDeviceNames.remember(targets);
    } catch {
      targets = targets ?? []; // toast already pushed
    } finally {
      loading = false;
    }
  }

  function onOpenChange(open: boolean) {
    // Stale-while-revalidate: every open kicks a re-scan, but the previous
    // enumeration stays on screen (the "Scanning devices…" placeholder only
    // gates the very first load) — enumeration can take seconds (flutter
    // device discovery), and a slightly stale list beats a spinner.
    if (open) void refresh();
    if (!open) {
      query = "";
      closePairing();
    }
  }

  async function pick(target: MobileRunTarget | null, close: () => void) {
    if (saving) return;
    saving = true;
    try {
      await safeInvoke("update_project", {
        id: project.id,
        patch: {
          mobileRun: {
            ...(project.mobileRun ?? {}),
            device: target?.id ?? null,
          },
        },
      });
      if (target) pushRecent(target.id);
      await projects.refresh();
      close();
    } catch {
      /* toast already pushed */
    } finally {
      saving = false;
    }
  }

  // Grouped view: recents first (intersection with the enumeration), then the
  // standard groups with already-shown recents removed.
  const grouped = $derived.by(() => {
    const all = targets ?? [];
    const recents = loadRecents()
      .map((id) => all.find((t) => t.id === id))
      .filter((t): t is MobileRunTarget => t !== undefined)
      .filter(
        (t) =>
          !query.trim() ||
          t.name.toLowerCase().includes(query.trim().toLowerCase()),
      );
    const recentIds = new Set(recents.map((t) => t.id));
    const rest = groupTargets(
      all.filter((t) => !recentIds.has(t.id)),
      query,
    );
    return { recents, rest };
  });

  let popoverOpen = $state(false);
  $effect(() => {
    onOpenChange(popoverOpen);
  });

  // Resolve the pinned id to its real name and connection state as soon as
  // the rail mounts. The `targets === null` guard makes this a single scan
  // per mount; once an enumeration (or a failed one) lands it never re-fires.
  $effect(() => {
    if (pinned !== null && targets === null && !loading) void refresh();
  });
</script>

{#snippet targetRow(t: MobileRunTarget, close: () => void)}
  <!-- select-none: without it the macOS WebView shows a text-selection I-beam
       over the row labels instead of the button's pointer cursor. -->
  <button
    type="button"
    disabled={t.unsupportedReason !== undefined || saving}
    title={t.unsupportedReason ?? `Run on ${t.name}`}
    onclick={() => void pick(t, close)}
    class="w-full flex items-center gap-2 px-2 py-1.5 rounded-md text-left
           text-[12px] transition-colors select-none
           {t.unsupportedReason
      ? 'opacity-45 cursor-not-allowed'
      : 'cursor-pointer hover:bg-surface-2'}"
  >
    <StatusDot
      status={t.state === "booted" || t.state === "connected"
        ? "running"
        : "stopped"}
      size="sm"
    />
    <span class="text-fg truncate">{t.name}</span>
    {#if t.osVersion}
      <span class="text-fg-subtle text-[10.5px] shrink-0">{t.osVersion}</span>
    {/if}
    <span class="ml-auto shrink-0 inline-flex items-center gap-1">
      {#if t.unsupportedReason}
        <span class="text-[10px] text-fg-subtle">unavailable</span>
      {/if}
      {#if pinned === t.id}
        <Icon name="check" size={12} class="text-accent" />
      {/if}
    </span>
  </button>
{/snippet}

<!-- Panel width tracks the trigger (100% of the chip): the rail is an
     overflow-y-auto container, so a fixed-width panel wider than the rail gets
     viewport-shifted by Popover's place() and then clipped by the rail's
     overflow context. Matching the chip keeps it fully inside the rail. -->
<Popover title="Destination" align="left" width="100%" bind:open={popoverOpen}>
  {#snippet trigger(toggle, open)}
    <button
      type="button"
      onclick={toggle}
      title={pinnedTarget?.unsupportedReason ?? "Choose run destination"}
      class="w-full inline-flex items-center gap-2 px-3 py-2 rounded-lg
             bg-surface-2/60 hover:bg-surface-2 border transition-colors
             text-[12px]
             {pinnedMissing
        ? 'border-status-unhealthy/60'
        : pinnedTarget?.unsupportedReason
          ? 'border-amber-400/50'
          : open
            ? 'border-accent/50'
            : 'border-border/60'}"
    >
      <Icon name="smartphone" size={13} class="shrink-0 text-fg-muted" />
      <span class="truncate text-fg">{chipLabel}</span>
      {#if pinnedTarget?.osVersion}
        <span class="text-fg-subtle text-[10.5px] shrink-0">
          {pinnedTarget.osVersion}
        </span>
      {/if}
      {#if pinnedTarget && (pinnedTarget.state === "booted" || pinnedTarget.state === "connected")}
        <StatusDot status="running" size="sm" />
      {/if}
      <Icon
        name="chevron-down"
        size={12}
        class="ml-auto shrink-0 text-fg-subtle"
      />
    </button>
  {/snippet}

  {#snippet children(close)}
    {#if pairView === "qr"}
      <!-- QR pairing: phone scans → backend pairs + connects → list refresh. -->
      <div class="space-y-2">
        <p class="text-[11.5px] text-fg-muted leading-snug">
          On the phone: Developer options → <strong>Wireless debugging</strong>
          → <strong>Pair device with QR code</strong>.
        </p>
        <div
          class="flex items-center justify-center rounded-md bg-fg text-bg p-3"
        >
          {#if pairQr}
            <!-- eslint-disable-next-line svelte/no-at-html-tags — SVG is
                 generated by our own backend from a fixed-format payload. -->
            <div class="w-[180px] h-[180px] [&>svg]:w-full [&>svg]:h-full">
              {@html pairQr.qrSvg}
            </div>
          {:else}
            <Icon name="loader-circle" size={22} class="animate-spin my-16" />
          {/if}
        </div>
        {#if pairStatus}
          <p
            class="text-[11.5px] leading-snug {pairDone
              ? pairStatus.startsWith('Connected') || pairStatus.startsWith('Paired')
                ? 'text-status-running'
                : 'text-status-unhealthy'
              : 'text-fg-muted'}"
          >
            {pairStatus}
          </p>
        {/if}
        <div class="flex items-center justify-between">
          <button
            type="button"
            onclick={() => (pairView = "manual")}
            class="text-[11px] text-fg-subtle hover:text-fg underline-offset-2 hover:underline"
          >
            Use pairing code instead
          </button>
          <button
            type="button"
            onclick={closePairing}
            class="px-2.5 py-1 rounded-md border border-border text-[11.5px]
                   text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
          >
            {pairDone ? "Back to devices" : "Cancel"}
          </button>
        </div>
      </div>
    {:else if pairView === "manual"}
      <!-- Manual fallback: "Pair device with pairing code" on the phone. -->
      <div class="space-y-2">
        <p class="text-[11.5px] text-fg-muted leading-snug">
          On the phone: Wireless debugging →
          <strong>Pair device with pairing code</strong>, then copy the values
          here.
        </p>
        <label class="block text-[11px] text-fg-subtle">
          IP address &amp; port
          <input
            type="text"
            bind:value={pairManualHost}
            placeholder="192.168.1.7:37123"
            spellcheck="false"
            class="mt-1 w-full px-2.5 py-1.5 rounded-md bg-bg border border-border
                   focus:border-accent/60 outline-none text-[12px] text-fg font-mono"
          />
        </label>
        <label class="block text-[11px] text-fg-subtle">
          Pairing code
          <input
            type="text"
            bind:value={pairManualCode}
            placeholder="123456"
            spellcheck="false"
            class="mt-1 w-full px-2.5 py-1.5 rounded-md bg-bg border border-border
                   focus:border-accent/60 outline-none text-[12px] text-fg font-mono"
          />
        </label>
        {#if pairStatus}
          <p
            class="text-[11.5px] leading-snug {pairDone
              ? 'text-status-running'
              : 'text-fg-muted'}"
          >
            {pairStatus}
          </p>
        {/if}
        <div class="flex items-center justify-between">
          <button
            type="button"
            onclick={() => void startQrPairing()}
            class="text-[11px] text-fg-subtle hover:text-fg underline-offset-2 hover:underline"
          >
            Use QR code instead
          </button>
          <div class="flex items-center gap-1.5">
            <button
              type="button"
              onclick={closePairing}
              class="px-2.5 py-1 rounded-md border border-border text-[11.5px]
                     text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
            >
              {pairDone ? "Back to devices" : "Cancel"}
            </button>
            <button
              type="button"
              onclick={() => void submitManualPair()}
              disabled={pairBusy || !pairManualHost.trim() || !pairManualCode.trim()}
              class="px-2.5 py-1 rounded-md bg-accent text-on-accent text-[11.5px]
                     hover:brightness-110 disabled:opacity-50 transition"
            >
              {pairBusy ? "Pairing…" : "Pair"}
            </button>
          </div>
        </div>
      </div>
    {:else}
    <div class="space-y-2">
      <div class="flex items-center gap-1.5">
        <div class="relative flex-1">
          <Icon
            name="search"
            size={12}
            class="absolute left-2 top-1/2 -translate-y-1/2 text-fg-subtle"
          />
          <!-- svelte-ignore a11y_autofocus -->
          <input
            type="text"
            bind:value={query}
            placeholder="Search devices"
            autofocus
            spellcheck="false"
            class="w-full pl-7 pr-2 py-1.5 rounded-md bg-bg border border-border
                   focus:border-accent/60 outline-none text-[12px] text-fg"
          />
        </div>
        <button
          type="button"
          onclick={() => void refresh()}
          disabled={loading}
          title="Re-scan devices"
          aria-label="Re-scan devices"
          class="p-1.5 rounded-md text-fg-subtle hover:text-fg hover:bg-surface-2
                 disabled:opacity-50 transition-colors"
        >
          <Icon name="refresh-cw" size={12} class={loading ? "animate-spin" : ""} />
        </button>
      </div>

      {#if pinnedMissing}
        <p
          class="px-2 py-1.5 rounded-md bg-status-unhealthy/10 text-status-unhealthy
                 text-[11px] leading-snug"
        >
          The pinned destination is unavailable — pick another. PortBay never
          substitutes a device silently.
        </p>
      {:else if pinnedTarget?.unsupportedReason}
        <!-- Pinned device enumerates but can't run right now (e.g. a wired
             iPhone that's unplugged / off the network) — same message the
             grayed row carries, surfaced where the user is looking. -->
        <p
          class="px-2 py-1.5 rounded-md bg-amber-400/10 text-amber-400
                 text-[11px] leading-snug"
        >
          {pinnedTarget.unsupportedReason}
        </p>
      {/if}

      <div class="max-h-72 overflow-y-auto space-y-2 -mx-0.5 px-0.5 select-none">
        {#if loading && targets === null}
          <p class="px-2 py-3 text-[11.5px] text-fg-subtle">Scanning devices…</p>
        {:else if (targets ?? []).length === 0}
          <p class="px-2 py-3 text-[11.5px] text-fg-subtle">
            No destinations found — check the toolchain checks in the rail.
          </p>
        {:else}
          <!-- Automatic: clear the pin, run resolves a device itself. -->
          <button
            type="button"
            disabled={saving}
            onclick={() => void pick(null, close)}
            class="w-full flex items-center gap-2 px-2 py-1.5 rounded-md text-left
                   text-[12px] hover:bg-surface-2 transition-colors
                   select-none cursor-pointer"
          >
            <Icon name="zap" size={12} class="text-fg-subtle" />
            <span class="text-fg">Automatic</span>
            <span class="text-fg-subtle text-[10.5px]">last booted / first available</span>
            {#if pinned === null}
              <Icon name="check" size={12} class="ml-auto shrink-0 text-accent" />
            {/if}
          </button>

          {#if grouped.recents.length > 0}
            <div>
              <p class="px-2 pb-1 text-[10px] uppercase tracking-wide text-fg-subtle">
                Recent
              </p>
              {#each grouped.recents as t (t.id)}
                {@render targetRow(t, close)}
              {/each}
            </div>
          {/if}

          {#each grouped.rest as g (g.group)}
            <div>
              <p class="px-2 pb-1 text-[10px] uppercase tracking-wide text-fg-subtle">
                {g.group}
              </p>
              {#each g.targets as t (t.id)}
                {@render targetRow(t, close)}
              {/each}
            </div>
          {/each}
        {/if}
      </div>

      {#if androidCapable}
        <div class="pt-1.5 border-t border-border/60">
          <button
            type="button"
            onclick={() => void startQrPairing()}
            class="w-full flex items-center gap-2 px-2 py-1.5 rounded-md text-left
                   text-[11.5px] text-fg-muted hover:text-fg hover:bg-surface-2
                   transition-colors"
          >
            <Icon name="plus" size={12} />
            Pair Android device over Wi-Fi…
          </button>
        </div>
      {/if}
    </div>
    {/if}
  {/snippet}
</Popover>
