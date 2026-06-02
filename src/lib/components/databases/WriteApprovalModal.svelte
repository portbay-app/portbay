<!--
  WriteApprovalModal — safety gate for agent-issued database writes.

  Shows the FIRST item from dbApprovals.pending in a blocking modal.
  Deny is the default-focused action; Approve requires an explicit click.
  Tab focus is trapped within the dialog; Escape triggers Deny.
  Both buttons are disabled while a verdict is in flight (no double-submit).
-->
<script lang="ts">
  import { dbApprovals } from "$lib/stores/dbApprovals.svelte";

  let dialogEl = $state<HTMLDivElement | null>(null);
  let denyButton = $state<HTMLButtonElement | null>(null);
  let lastFocused: HTMLElement | null = null;
  let inFlight = $state(false);

  const item = $derived(dbApprovals.pending[0] ?? null);
  const total = $derived(dbApprovals.pending.length);

  // Move focus to Deny on open; restore on close.
  $effect(() => {
    if (item) {
      lastFocused = document.activeElement as HTMLElement | null;
      queueMicrotask(() => denyButton?.focus());
    } else if (lastFocused) {
      lastFocused.focus();
      lastFocused = null;
    }
  });

  function originLabel(origin: string): string {
    if (origin === "mcp-agent") return "AI agent";
    return origin;
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      void handleDeny();
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

  async function handleApprove() {
    if (!item || inFlight) return;
    inFlight = true;
    try {
      await dbApprovals.approve(item.id);
    } finally {
      inFlight = false;
    }
  }

  async function handleDeny() {
    if (!item || inFlight) return;
    inFlight = true;
    try {
      await dbApprovals.deny(item.id);
    } finally {
      inFlight = false;
    }
  }
</script>

<svelte:window onkeydown={item ? onKeydown : undefined} />

{#if item}
  <!-- Backdrop -->
  <div
    class="fixed inset-0 z-[60] bg-black/40 backdrop-blur-sm"
    role="presentation"
  ></div>

  <!-- Dialog -->
  <div
    bind:this={dialogEl}
    role="dialog"
    aria-modal="true"
    aria-labelledby="db-write-title"
    aria-describedby="db-write-body"
    class="fixed left-1/2 top-1/2 z-[61] w-[min(560px,calc(100vw-2rem))]
           -translate-x-1/2 -translate-y-1/2 rounded-2xl bg-bg border border-border
           shadow-2xl flex flex-col overflow-hidden"
  >
    <!-- Header -->
    <div class="px-5 pt-5 pb-4">
      <div class="flex items-start gap-3">
        <!-- Warning icon -->
        <span
          class="shrink-0 mt-0.5 grid place-items-center w-8 h-8 rounded-full
                 bg-amber-500/10 text-amber-400"
        >
          <svg
            xmlns="http://www.w3.org/2000/svg"
            width="16"
            height="16"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
          >
            <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
            <line x1="12" y1="9" x2="12" y2="13" />
            <line x1="12" y1="17" x2="12.01" y2="17" />
          </svg>
        </span>

        <div class="min-w-0 flex-1" id="db-write-body">
          <div class="flex items-center justify-between gap-2 flex-wrap">
            <h2
              id="db-write-title"
              class="text-[15px] font-semibold text-fg tracking-tight"
            >
              Approve database write?
            </h2>
            {#if total > 1}
              <span class="text-[11px] text-fg-muted tabular-nums">
                1 of {total} pending
              </span>
            {/if}
          </div>

          <!-- Instance + engine + origin -->
          <div class="mt-2 flex flex-wrap items-center gap-1.5 text-[12px]">
            <span class="font-mono text-fg">{item.instanceId}</span>
            <span
              class="inline-flex items-center px-1.5 py-0.5 rounded text-[10.5px] font-medium
                     bg-accent/10 text-accent uppercase tracking-wide"
            >
              {item.engine}
            </span>
            <span
              class="inline-flex items-center px-1.5 py-0.5 rounded text-[10.5px] font-medium
                     bg-amber-500/10 text-amber-400"
            >
              {originLabel(item.origin)}
            </span>
          </div>

          {#if item.schema}
            <p class="mt-1.5 text-[12px] text-fg-muted">
              Schema: <span class="font-mono text-fg">{item.schema}</span>
            </p>
          {/if}

          <!-- SQL — shown verbatim, never truncated -->
          <div class="mt-3">
            <p class="text-[11px] font-medium text-fg-muted uppercase tracking-wide mb-1">
              SQL
            </p>
            <pre
              class="w-full overflow-x-auto rounded-lg bg-surface-2 border border-border
                     px-3 py-2.5 text-[12px] font-mono text-fg leading-relaxed
                     whitespace-pre max-h-64 overflow-y-auto"
            >{item.sql}</pre>
          </div>
        </div>
      </div>
    </div>

    <!-- Footer -->
    <footer
      class="px-5 py-3.5 border-t border-border bg-surface/40 flex flex-wrap items-center justify-end gap-2"
    >
      <!-- Deny — default focus, safe action -->
      <button
        bind:this={denyButton}
        type="button"
        disabled={inFlight}
        onclick={handleDeny}
        class="h-8 px-3.5 rounded-md text-[12px] font-medium
               text-fg border border-border hover:bg-surface-2
               disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
      >
        Deny
      </button>

      <!-- Approve — secondary action, never auto-focused -->
      <button
        type="button"
        disabled={inFlight}
        onclick={handleApprove}
        class="h-8 px-3.5 rounded-md text-[12px] font-medium
               text-on-accent bg-accent hover:brightness-110 active:brightness-95
               shadow-sm disabled:opacity-50 disabled:cursor-not-allowed transition"
      >
        Approve
      </button>
    </footer>
  </div>
{/if}
