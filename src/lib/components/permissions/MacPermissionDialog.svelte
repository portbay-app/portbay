<!--
  MacPermissionDialog — Codex-style macOS permission request sheet.

  Shows the app icon with a drag affordance (matches System Settings
  drag-to-grant UX) plus an "Open System Settings" button that navigates
  directly to the relevant Privacy pane via URL scheme.

  Usage:
    <MacPermissionDialog kind="accessibility" bind:open onClose={close} />
-->
<script lang="ts">
  import { safeInvoke } from "$lib/ipc";

  interface Props {
    open?: boolean;
    kind: "accessibility" | "screen-recording" | "full-disk-access";
    onClose?: () => void;
  }
  let { open = false, kind, onClose }: Props = $props();

  const CONFIGS = {
    accessibility: {
      title: "Accessibility Access Required",
      subtitle: "PortBay needs permission to control your computer",
      description:
        "Allow PortBay to use Accessibility features to automate setup tasks.",
      settingsName: "Accessibility",
    },
    "screen-recording": {
      title: "Screen Recording Required",
      subtitle: "PortBay needs permission to record the screen",
      description:
        "Allow PortBay to capture your screen for project monitoring.",
      settingsName: "Screen Recording",
    },
    "full-disk-access": {
      title: "Full Disk Access Required",
      subtitle: "PortBay needs permission to access all files",
      description:
        "Allow PortBay to read and write files across your filesystem.",
      settingsName: "Full Disk Access",
    },
  } as const;

  const config = $derived(CONFIGS[kind]);

  async function openSettings() {
    await safeInvoke("open_privacy_settings", { kind });
    onClose?.();
  }

  function handleDragStart(e: DragEvent) {
    // Native file drag isn't available in Tauri — open the pane instead,
    // which is the primary action the user actually needs.
    e.preventDefault();
    void openSettings();
  }

  function handleBackdropClick(e: MouseEvent) {
    if (e.target === e.currentTarget) onClose?.();
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") onClose?.();
  }
</script>

<svelte:window onkeydown={handleKeydown} />

{#if open}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div
    class="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm"
    role="dialog"
    aria-modal="true"
    aria-label={config.title}
    tabindex="-1"
    onclick={handleBackdropClick}
  >
    <div
      class="w-[420px] rounded-2xl border border-border bg-surface shadow-2xl p-6 flex flex-col gap-5"
    >
      <!-- Header -->
      <div class="flex flex-col items-center text-center gap-1.5">
        <h2 class="text-[15px] font-semibold text-fg">{config.title}</h2>
        <p class="text-[12px] text-fg-muted">{config.subtitle}</p>
      </div>

      <!-- Description -->
      <p class="text-[12px] text-fg-muted leading-relaxed">{config.description}</p>

      <!-- Drag affordance -->
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
      <div
        draggable={true}
        ondragstart={handleDragStart}
        role="img"
        aria-label="Drag PortBay into {config.settingsName} settings"
        class="flex cursor-grab items-center gap-3 rounded-xl border border-border
               bg-surface-2 p-3 active:cursor-grabbing select-none"
      >
        <div class="flex h-16 w-16 shrink-0 items-center justify-center">
          <img
            src="/icon.png"
            alt=""
            aria-hidden="true"
            class="h-14 w-14 object-contain rounded-[18px]"
            draggable={false}
          />
        </div>
        <p class="text-[12px] text-fg-muted leading-relaxed">
          If <strong class="text-fg font-medium">PortBay</strong> doesn't appear
          in the list, drag this icon into
          <strong class="text-fg font-medium">{config.settingsName}</strong> settings
        </p>
      </div>

      <!-- Actions -->
      <div class="flex gap-2 justify-end">
        {#if onClose}
          <button
            type="button"
            onclick={onClose}
            class="px-4 py-2 rounded-md text-[12px] text-fg-muted hover:text-fg
                   border border-border hover:bg-surface-2 transition-colors"
          >
            Not now
          </button>
        {/if}
        <button
          type="button"
          onclick={openSettings}
          class="px-4 py-2 rounded-md text-[12px] font-medium text-on-accent
                 bg-accent hover:bg-accent-hover transition-colors"
        >
          Open System Settings
        </button>
      </div>
    </div>
  </div>
{/if}
