<!--
  ConfirmDialog — the single in-app confirmation modal, driven by the
  `confirmDialog` store. Mounted once at the layout root.

  Unlike native confirm(), it presents any number of explicit, labeled
  actions plus a Cancel, so destructive flows never overload OK/Cancel.
  Cancel / Escape / backdrop click all resolve the promise as `null`;
  focus opens on the Cancel button (the safe default) and is trapped
  while open, then restored to the previously-focused element on close.
-->
<script lang="ts">
  import Icon from "$lib/components/atoms/Icon.svelte";
  import { confirmDialog } from "$lib/stores/confirm.svelte";
  import type { ConfirmTone } from "$lib/stores/confirm.svelte";

  let dialogEl = $state<HTMLDivElement | null>(null);
  let cancelButton = $state<HTMLButtonElement | null>(null);
  let lastFocused: HTMLElement | null = null;

  // When the dialog opens, remember what had focus and move focus to the
  // Cancel button. When it closes, restore focus.
  $effect(() => {
    if (confirmDialog.isOpen) {
      lastFocused = document.activeElement as HTMLElement | null;
      queueMicrotask(() => cancelButton?.focus());
    } else if (lastFocused) {
      lastFocused.focus();
      lastFocused = null;
    }
  });

  function actionClass(tone: ConfirmTone | undefined): string {
    if (tone === "destructive") {
      return "text-on-accent bg-status-crashed hover:brightness-110 active:brightness-95 shadow-sm";
    }
    if (tone === "primary") {
      return "text-on-accent bg-accent hover:brightness-110 active:brightness-95 shadow-sm";
    }
    return "text-fg border border-border hover:bg-surface-2";
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      confirmDialog.cancel();
      return;
    }
    if (e.key !== "Tab") return;
    // Trap focus within the dialog.
    const focusables = dialogEl?.querySelectorAll<HTMLElement>(
      'button:not([disabled]), [href], input, [tabindex]:not([tabindex="-1"])',
    );
    if (!focusables || focusables.length === 0) return;
    const first = focusables[0];
    const last = focusables[focusables.length - 1];
    const active = document.activeElement;
    if (e.shiftKey && active === first) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && active === last) {
      e.preventDefault();
      first.focus();
    }
  }

  const paragraphs = $derived(confirmDialog.message.split("\n\n"));
</script>

<svelte:window onkeydown={confirmDialog.isOpen ? onKeydown : undefined} />

{#if confirmDialog.isOpen}
  <div
    class="fixed inset-0 z-[60] bg-black/40 backdrop-blur-sm"
    onclick={() => confirmDialog.cancel()}
    role="presentation"
  ></div>
  <div
    bind:this={dialogEl}
    role="dialog"
    aria-modal="true"
    aria-labelledby="confirm-title"
    aria-describedby="confirm-body"
    class="fixed left-1/2 top-1/2 z-[61] w-[min(440px,calc(100vw-2rem))]
           -translate-x-1/2 -translate-y-1/2 rounded-2xl bg-bg border border-border
           shadow-2xl flex flex-col overflow-hidden"
  >
    <div class="px-5 pt-5 pb-4">
      <div class="flex items-start gap-3">
        <span
          class="shrink-0 mt-0.5 grid place-items-center w-8 h-8 rounded-full
                 {confirmDialog.destructive
            ? 'bg-status-crashed/10 text-status-crashed'
            : 'bg-accent/10 text-accent'}"
        >
          <Icon name={confirmDialog.icon} size={16} />
        </span>
        <div class="min-w-0 flex-1">
          <h2 id="confirm-title" class="text-[15px] font-semibold text-fg tracking-tight">
            {confirmDialog.title}
          </h2>
          <div id="confirm-body" class="mt-1.5 space-y-2">
            {#each paragraphs as para}
              <p class="text-[12.5px] leading-relaxed text-fg-muted">{para}</p>
            {/each}
          </div>
        </div>
      </div>
    </div>

    <footer
      class="px-5 py-3.5 border-t border-border bg-surface/40 flex flex-wrap items-center justify-end gap-2"
    >
      <button
        bind:this={cancelButton}
        type="button"
        onclick={() => confirmDialog.cancel()}
        class="h-8 px-3.5 rounded-md text-[12px] font-medium text-fg-muted
               hover:text-fg hover:bg-surface-2 transition-colors"
      >
        {confirmDialog.cancelLabel}
      </button>
      {#each confirmDialog.actions as action (action.value)}
        <button
          type="button"
          onclick={() => confirmDialog.choose(action.value)}
          class="inline-flex items-center gap-1.5 h-8 px-3.5 rounded-md text-[12px]
                 font-medium transition {actionClass(action.tone)}"
        >
          {#if action.icon}
            <Icon name={action.icon} size={12} />
          {/if}
          {action.label}
        </button>
      {/each}
    </footer>
  </div>
{/if}
