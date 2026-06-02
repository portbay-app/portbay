<!--
  MacPermissionDialog — polished macOS permission request sheet.

  PortBay's real privileged step is installing its hosts/DNS helper, which
  triggers one admin-password prompt and then appears under System Settings ›
  General › Login Items › "Allow in the Background". This sheet is shown
  *before* that step so the request is explained and user-initiated (never a
  surprise prompt at launch), with an animated hint of the Login-Items toggle
  the user ends up approving.

  It also handles true TCC kinds (accessibility / screen-recording /
  full-disk-access) for completeness — those use the System Settings privacy
  panes and a drag affordance instead of the install action.

  Usage (helper / DNS):
    <MacPermissionDialog bind:open kind="login-items"
      onConfirm={() => dns.setupLocalDns()} onClose={close} />
-->
<script lang="ts">
  import { safeInvoke } from "$lib/ipc";
  import { startDrag } from "@crabnebula/tauri-plugin-drag";

  type PermissionKind =
    | "login-items"
    | "accessibility"
    | "screen-recording"
    | "full-disk-access";

  interface Props {
    open?: boolean;
    kind?: PermissionKind;
    /** Runs the privileged action (e.g. install the helper). When set, the
        primary button calls this; otherwise it opens System Settings. */
    onConfirm?: () => void | Promise<void>;
    onClose?: () => void;
  }
  let { open = false, kind = "login-items", onConfirm, onClose }: Props =
    $props();

  interface KindConfig {
    title: string;
    subtitle: string;
    description: string;
    /** Pane name shown in copy + opened by the secondary action. */
    settingsName: string;
    /** kind passed to `open_privacy_settings`. */
    settingsKind: string;
    /** "toggle" → animate the Login-Items switch; "drag" → animate dropping
        the icon into a privacy list. */
    gesture: "toggle" | "drag";
    primaryLabel: string;
  }

  const CONFIGS: Record<PermissionKind, KindConfig> = {
    "login-items": {
      title: "Set up local DNS for your projects",
      subtitle: "One-time approval",
      description:
        "PortBay installs a small privileged helper so hostnames like myapp.test resolve to this Mac. macOS will ask for your password once, then list PortBay under Login Items › Allow in the Background.",
      settingsName: "Login Items",
      settingsKind: "login-items",
      gesture: "toggle",
      primaryLabel: "Install helper",
    },
    accessibility: {
      title: "Accessibility access required",
      subtitle: "Grant in System Settings",
      description:
        "Allow PortBay to use Accessibility features. If PortBay isn't in the list, drag the icon below into the Accessibility list.",
      settingsName: "Accessibility",
      settingsKind: "accessibility",
      gesture: "drag",
      primaryLabel: "Open System Settings",
    },
    "screen-recording": {
      title: "Screen recording required",
      subtitle: "Grant in System Settings",
      description:
        "Allow PortBay to capture your screen. If PortBay isn't in the list, drag the icon below into the Screen Recording list.",
      settingsName: "Screen Recording",
      settingsKind: "screen-recording",
      gesture: "drag",
      primaryLabel: "Open System Settings",
    },
    "full-disk-access": {
      title: "Full Disk Access required",
      subtitle: "Grant in System Settings",
      description:
        "Allow PortBay to read and write files across your filesystem. If PortBay isn't in the list, drag the icon below into the Full Disk Access list.",
      settingsName: "Full Disk Access",
      settingsKind: "full-disk-access",
      gesture: "drag",
      primaryLabel: "Open System Settings",
    },
  };

  const config = $derived(CONFIGS[kind]);
  let busy = $state(false);

  // Native drag-to-grant: for the privacy "drag" gestures the app icon below is
  // a real OS drag source — dragging it out and dropping it into the System
  // Settings privacy list adds PortBay (the macOS gesture that grants access).
  // The backend hands us the path to drag (the .app bundle) and a cursor icon;
  // `tauri-plugin-drag` begins the AppKit dragging session a webview can't.
  interface DragPayload {
    bundlePath: string;
    iconPath: string;
  }
  let dragPayload = $state<DragPayload | null>(null);
  // Set once the user has dropped PortBay into the list. macOS only applies a
  // newly-granted Accessibility permission after the app relaunches, so instead
  // of just closing we switch to a "relaunch to finish" step.
  let dropped = $state(false);

  $effect(() => {
    if (open && config.gesture === "drag") {
      dropped = false; // reset each time the sheet opens
      if (!dragPayload) {
        safeInvoke<DragPayload>("permission_drag_payload")
          .then((p) => (dragPayload = p))
          .catch(() => {});
      }
    }
  });

  async function relaunch() {
    await safeInvoke("relaunch_app").catch(() => {});
  }

  // MUST stay synchronous: the native drag session has to begin within the
  // dragstart event tick. Awaiting anything first (e.g. fetching the payload)
  // detaches it from the gesture and the OS never receives the file — which is
  // why an earlier async version "looked" like it dragged but added nothing.
  // The payload is prefetched by the $effect above when the sheet opens.
  function onIconDragStart(e: DragEvent) {
    if (config.gesture !== "drag" || !dragPayload) return;
    e.preventDefault(); // suppress the webview's own image drag
    // The onEvent callback reports the drag outcome: once the user drops the
    // tile onto a target ("Dropped" — i.e. into the Settings list), the gesture
    // is done, so dismiss the sheet. A "Cancelled" release leaves it open.
    void startDrag(
      { item: [dragPayload.bundlePath], icon: dragPayload.iconPath },
      (payload) => {
        if (payload.result === "Dropped") dropped = true;
      },
    );
  }

  async function openSettings() {
    await safeInvoke("open_privacy_settings", { kind: config.settingsKind });
  }

  async function primary() {
    if (onConfirm) {
      busy = true;
      try {
        await onConfirm();
        onClose?.();
      } finally {
        busy = false;
      }
    } else {
      // Open the privacy pane but KEEP this sheet open: the drag-to-grant flow
      // needs the user to come back and drag the tile into the list, then retry
      // — closing here would force them to trigger the whole flow again.
      await openSettings();
    }
  }

  function onBackdrop(e: MouseEvent) {
    if (e.target === e.currentTarget) onClose?.();
  }
  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") onClose?.();
  }
</script>

<svelte:window onkeydown={onKeydown} />

{#if open}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div
    class="fixed inset-0 z-50 flex items-center justify-center bg-black/55 backdrop-blur-sm"
    role="dialog"
    aria-modal="true"
    aria-label={config.title}
    tabindex="-1"
    onclick={onBackdrop}
  >
    <div
      class="perm-card w-[440px] rounded-2xl border border-border bg-surface
             shadow-2xl p-6 flex flex-col items-center gap-5 text-center"
    >
      {#if config.gesture === "toggle"}
        <!-- Toggle gesture (DNS helper / Login Items): icon → flow → switch. -->
        <div class="relative flex w-full items-center justify-center gap-4 py-3">
          <div class="perm-app relative">
            <span class="perm-glow"></span>
            <img src="/icon.png" alt="" aria-hidden="true" class="relative h-16 w-16 rounded-[16px]" />
          </div>
          <div class="perm-flow" aria-hidden="true"><span></span><span></span><span></span></div>
          <div class="perm-settings">
            <img src="/icon.png" alt="" aria-hidden="true" class="h-5 w-5 rounded-[5px]" />
            <span class="perm-settings-label">PortBay</span>
            <span class="perm-toggle"><span class="perm-knob"></span></span>
          </div>
        </div>

        <div class="flex flex-col gap-1">
          <h2 class="text-[15px] font-semibold text-fg">{config.title}</h2>
          <p class="text-[11.5px] uppercase tracking-wide text-accent">{config.subtitle}</p>
        </div>
        <p class="text-[12px] text-fg-muted leading-relaxed">{config.description}</p>
      {:else if dropped}
        <!-- Dropped into the list. macOS only applies a newly-granted
             permission after the app relaunches, so guide the user to restart. -->
        <div class="perm-app relative py-2">
          <span class="perm-glow"></span>
          <img src="/icon.png" alt="" aria-hidden="true" class="relative h-16 w-16 rounded-[18px]" />
        </div>
        <div class="flex flex-col gap-1">
          <h2 class="text-[15px] font-semibold text-fg">Almost done — relaunch PortBay</h2>
          <p class="text-[11.5px] uppercase tracking-wide text-accent">One last step</p>
        </div>
        <p class="text-[12px] text-fg-muted leading-relaxed">
          PortBay was added to {config.settingsName}. macOS only applies a new
          permission after the app restarts — make sure PortBay is switched
          <span class="font-medium text-fg">on</span> in the list, then relaunch
          to start using voice-to-text.
        </p>
      {:else}
        <!-- Drag-to-grant gesture: open the privacy pane, then physically drag
             the PortBay tile into the list (the macOS gesture that adds it). -->
        <!-- Animated hero: PortBay logo → flowing dots → the privacy list it
             drops into (the blinking dashed row is the drop target). -->
        <div class="relative flex w-full items-center justify-center gap-4 py-3">
          <div class="perm-app relative">
            <span class="perm-glow"></span>
            <img src="/icon.png" alt="" aria-hidden="true" class="relative h-16 w-16 rounded-[16px]" />
          </div>
          <div class="perm-flow" aria-hidden="true"><span></span><span></span><span></span></div>
          <div class="perm-list" aria-hidden="true">
            <span class="perm-row perm-row-drop"></span>
            <span class="perm-row"></span>
            <span class="perm-row"></span>
          </div>
        </div>

        <div class="flex flex-col gap-1">
          <h2 class="text-[15px] font-semibold text-fg">{config.title}</h2>
          <p class="text-[11.5px] uppercase tracking-wide text-accent">{config.subtitle}</p>
        </div>

        <!-- Numbered steps -->
        <ol class="w-full space-y-2 text-left">
          {#each [`Open System Settings → ${config.settingsName}.`, `Drag the PortBay tile below into the ${config.settingsName} list.`, "Switch PortBay on in the list."] as step, i (i)}
            <li class="flex items-start gap-2.5">
              <span class="mt-px flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-accent/15 text-[11px] font-semibold text-accent">{i + 1}</span>
              <span class="text-[12px] text-fg-muted leading-relaxed">{step}</span>
            </li>
          {/each}
        </ol>

        <!-- The real OS drag source: drag this tile into the privacy list. -->
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div
          class="perm-drag-chip group flex w-full cursor-grab items-center gap-3 rounded-xl border border-border bg-surface-2 p-2.5 transition-colors hover:border-accent/60 active:cursor-grabbing"
          draggable="true"
          ondragstart={onIconDragStart}
          title={`Drag into ${config.settingsName}`}
          aria-label={`Drag PortBay into ${config.settingsName} settings`}
        >
          <img src="/icon.png" alt="" aria-hidden="true" draggable="false" class="h-11 w-11 shrink-0 rounded-[12px]" />
          <div class="flex min-w-0 flex-1 flex-col text-left">
            <span class="truncate text-[12.5px] font-medium text-fg">Drag PortBay into {config.settingsName}</span>
            <span class="text-[11px] text-fg-subtle">Drop it onto the list to grant access</span>
          </div>
          <span class="perm-grip" aria-hidden="true">
            <svg width="14" height="14" viewBox="0 0 14 14" fill="currentColor"><circle cx="4" cy="3" r="1.3"/><circle cx="10" cy="3" r="1.3"/><circle cx="4" cy="7" r="1.3"/><circle cx="10" cy="7" r="1.3"/><circle cx="4" cy="11" r="1.3"/><circle cx="10" cy="11" r="1.3"/></svg>
          </span>
        </div>
      {/if}

      <!-- Actions -->
      <div class="mt-1 flex w-full flex-col gap-2">
        {#if dropped}
          <button
            type="button"
            onclick={relaunch}
            class="h-9 w-full rounded-lg text-[12.5px] font-medium text-on-accent
                   bg-accent hover:bg-accent-hover transition-colors"
          >
            Relaunch PortBay
          </button>
          <div class="flex items-center justify-center gap-4">
            {#if onClose}
              <button
                type="button"
                onclick={onClose}
                class="text-[11.5px] text-fg-subtle hover:text-fg transition-colors"
              >
                Later
              </button>
            {/if}
          </div>
        {:else}
        <button
          type="button"
          disabled={busy}
          onclick={primary}
          class="h-9 w-full rounded-lg text-[12.5px] font-medium text-on-accent
                 bg-accent hover:bg-accent-hover transition-colors disabled:opacity-60"
        >
          {busy ? "Working…" : config.primaryLabel}
        </button>
        <div class="flex items-center justify-center gap-4">
          {#if onConfirm}
            <button
              type="button"
              onclick={openSettings}
              class="text-[11.5px] text-fg-muted hover:text-fg transition-colors"
            >
              Open {config.settingsName}
            </button>
          {/if}
          {#if onClose}
            <button
              type="button"
              onclick={onClose}
              class="text-[11.5px] text-fg-subtle hover:text-fg transition-colors"
            >
              Not now
            </button>
          {/if}
        </div>
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  .perm-card {
    animation: perm-pop 220ms cubic-bezier(0.22, 1, 0.36, 1);
  }
  @keyframes perm-pop {
    from { opacity: 0; transform: translateY(8px) scale(0.98); }
    to { opacity: 1; transform: translateY(0) scale(1); }
  }

  /* App icon — gentle float + soft accent glow pulse. */
  .perm-app { animation: perm-float 3s ease-in-out infinite; }
  @keyframes perm-float {
    0%, 100% { transform: translateY(0); }
    50% { transform: translateY(-5px); }
  }
  .perm-glow {
    position: absolute;
    inset: -10px;
    border-radius: 22px;
    background: radial-gradient(circle, var(--color-accent) 0%, transparent 68%);
    opacity: 0.35;
    filter: blur(8px);
    animation: perm-pulse 3s ease-in-out infinite;
  }
  @keyframes perm-pulse {
    0%, 100% { opacity: 0.22; transform: scale(0.95); }
    50% { opacity: 0.45; transform: scale(1.08); }
  }

  /* Flowing dots from the icon toward the destination. */
  .perm-flow { display: flex; gap: 6px; }
  .perm-flow span {
    width: 6px; height: 6px; border-radius: 9999px;
    background: var(--color-accent);
    opacity: 0.25;
    animation: perm-dot 1.4s ease-in-out infinite;
  }
  .perm-flow span:nth-child(2) { animation-delay: 0.2s; }
  .perm-flow span:nth-child(3) { animation-delay: 0.4s; }
  @keyframes perm-dot {
    0%, 100% { opacity: 0.2; transform: scale(0.85); }
    50% { opacity: 1; transform: scale(1.1); }
  }

  /* Login Items row + toggle that animates on. */
  .perm-settings {
    display: flex; align-items: center; gap: 8px;
    padding: 8px 10px; border-radius: 10px;
    background: var(--color-surface-2);
    border: 1px solid var(--color-border);
    width: 150px;
  }
  .perm-settings-label { font-size: 11px; color: var(--color-fg); flex: 1; text-align: left; }
  .perm-toggle {
    position: relative; width: 30px; height: 18px; border-radius: 9999px;
    background: var(--color-fg-subtle);
    animation: perm-track 2.6s ease-in-out infinite;
  }
  .perm-knob {
    position: absolute; top: 2px; left: 2px;
    width: 14px; height: 14px; border-radius: 9999px; background: #fff;
    animation: perm-slide 2.6s ease-in-out infinite;
  }
  @keyframes perm-track {
    0%, 35% { background: var(--color-fg-subtle); }
    55%, 100% { background: var(--color-status-running); }
  }
  @keyframes perm-slide {
    0%, 35% { left: 2px; }
    55%, 100% { left: 14px; }
  }

  /* Privacy list (hero destination) with a blinking dashed drop row. */
  .perm-list {
    display: flex; flex-direction: column; gap: 6px;
    padding: 8px; border-radius: 10px; width: 120px;
    background: var(--color-surface-2); border: 1px solid var(--color-border);
  }
  .perm-row { height: 8px; border-radius: 4px; background: var(--color-border-strong); }
  .perm-row-drop {
    background: transparent;
    border: 1.5px dashed var(--color-accent);
    animation: perm-blink 1.6s ease-in-out infinite;
  }
  @keyframes perm-blink {
    0%, 100% { opacity: 0.4; }
    50% { opacity: 1; }
  }

  /* Draggable tile: a soft accent ring breathes to signal it's grabbable, and
     the grip dots nudge sideways to hint "drag me out". */
  .perm-drag-chip {
    box-shadow: 0 0 0 0 rgba(0, 0, 0, 0);
    animation: perm-chip 2.4s ease-in-out infinite;
  }
  .perm-drag-chip:hover { animation-play-state: paused; }
  @keyframes perm-chip {
    0%, 100% { box-shadow: 0 0 0 0 color-mix(in srgb, var(--color-accent) 40%, transparent); }
    50% { box-shadow: 0 0 0 4px color-mix(in srgb, var(--color-accent) 0%, transparent); }
  }
  .perm-grip { animation: perm-grip-nudge 2.4s ease-in-out infinite; }
  @keyframes perm-grip-nudge {
    0%, 100% { transform: translateX(0); opacity: 0.5; }
    50% { transform: translateX(3px); opacity: 0.9; }
  }

  @media (prefers-reduced-motion: reduce) {
    .perm-card, .perm-app, .perm-glow, .perm-flow span,
    .perm-toggle, .perm-knob, .perm-drag-chip, .perm-grip, .perm-row-drop { animation: none; }
    .perm-toggle { background: var(--color-status-running); }
    .perm-knob { left: 14px; }
  }
</style>
