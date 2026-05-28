<!--
  CrashReportCard — the proactive "send this crash in one click" surface.

  Driven entirely by the `crashSurface` store. Two presentations from one card:

    • mode "crash" — a crash from a previous session (e.g. a Rust panic that
      took the app down). Shown as a centred modal on launch, because the app
      genuinely failed and the report is worth the user's attention.

    • mode "live"  — a JS error caught while the app is still running. Shown as
      a quiet bottom-right card so it doesn't interrupt; the store guarantees
      one prompt per distinct fault, ever.

  Both share the inner content: a one-click "Send report" (the click is the
  consent — no telemetry opt-in needed), then a Bun-style "Crash Report Sent"
  confirmation with a collapsible stack trace.
-->
<script lang="ts">
  import { fade, fly } from "svelte/transition";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import { openUrl } from "$lib/security/openUrl";
  import { crashSurface } from "$lib/stores/crashSurface.svelte";
  import type { CrashKind } from "$lib/types/telemetry";

  const ISSUES_URL = "https://github.com/portbay-app/portbay/issues/new";

  let showTrace = $state(false);

  const report = $derived(crashSurface.report);
  const mode = $derived(crashSurface.mode);
  const phase = $derived(crashSurface.phase);
  const sent = $derived(phase === "sent");

  function kindLabel(kind: CrashKind): string {
    switch (kind) {
      case "rust_panic":
        return "an internal crash";
      case "js_unhandled_rejection":
        return "an unhandled error";
      default:
        return "an unexpected error";
    }
  }

  const title = $derived(
    sent
      ? "Crash Report Sent"
      : mode === "crash"
        ? "PortBay crashed last session"
        : "PortBay hit a problem",
  );

  const subtitle = $derived(
    sent
      ? "We've sent a redacted crash report to the PortBay team."
      : mode === "crash"
        ? "It looks like PortBay closed unexpectedly. Sending the report helps us fix it."
        : `PortBay ran into ${report ? kindLabel(report.kind) : "an error"}. The app is still running — you can keep working.`,
  );

  function onKeydown(event: KeyboardEvent) {
    if (event.key === "Escape" && crashSurface.isOpen) crashSurface.dismiss();
  }
</script>

<svelte:window onkeydown={onKeydown} />

{#snippet content()}
  {#if report}
    <div class="p-5">
      {#if sent}
        <!-- Confirmation (Bun-style) -->
        <div class="flex flex-col items-center text-center">
          <div
            class="grid place-items-center w-12 h-12 rounded-full bg-green-500 text-white"
          >
            <Icon name="check" size={22} />
          </div>
          <h3 class="mt-3 text-[16px] font-semibold text-fg">{title}</h3>
          <p class="mt-1 text-[12.5px] leading-relaxed text-fg-muted max-w-sm">
            {subtitle}
          </p>
        </div>

        <div class="mt-4 rounded-lg border border-accent/30 bg-accent/5 p-3">
          <div class="flex items-center gap-1.5 text-accent">
            <Icon name="shield" size={13} />
            <span class="text-[12px] font-semibold">About this report</span>
          </div>
          <p class="mt-1 text-[11.5px] leading-relaxed text-fg-muted">
            PortBay is open source. The report includes only the error message,
            a sanitised stack trace, your OS, and the app version — never your
            projects, file paths, or credentials.
          </p>
        </div>

        <div class="mt-3 rounded-lg border border-border bg-bg/60 p-3">
          <p class="text-[11px] uppercase tracking-wide text-fg-subtle">
            What you can do
          </p>
          <ol class="mt-2 space-y-2">
            <li class="flex items-start gap-2.5">
              <span
                class="grid place-items-center shrink-0 w-5 h-5 rounded-full bg-surface-2 text-[10px] text-fg-muted"
                >1</span
              >
              <span class="text-[12px] leading-snug text-fg"
                >Check for app updates — a fix may already be available.</span
              >
            </li>
            <li class="flex items-start gap-2.5">
              <span
                class="grid place-items-center shrink-0 w-5 h-5 rounded-full bg-surface-2 text-[10px] text-fg-muted"
                >2</span
              >
              <span class="text-[12px] leading-snug text-fg">
                <button
                  type="button"
                  class="text-accent hover:underline"
                  onclick={() => void openUrl(ISSUES_URL)}>Open an issue on GitHub</button
                >
                with steps to reproduce so we can fix it faster.
              </span>
            </li>
          </ol>
        </div>
      {:else}
        <!-- Offer to send -->
        <div class="flex items-start gap-3">
          <div
            class="grid place-items-center shrink-0 w-9 h-9 rounded-full bg-red-500/15 text-red-400"
          >
            <Icon name="circle-alert" size={18} />
          </div>
          <div class="min-w-0 flex-1">
            <div class="flex items-start justify-between gap-2">
              <h3 class="text-[14px] font-semibold text-fg leading-snug">{title}</h3>
              <button
                type="button"
                onclick={() => crashSurface.dismiss()}
                aria-label="Dismiss"
                class="-mr-1 -mt-0.5 shrink-0 grid place-items-center w-6 h-6 rounded-md text-fg-subtle hover:text-fg hover:bg-surface-2 transition-colors"
              >
                <Icon name="x" size={14} />
              </button>
            </div>
            <p class="mt-1 text-[12px] leading-relaxed text-fg-muted">{subtitle}</p>
          </div>
        </div>

        <p
          class="mt-3 rounded-md border border-border bg-bg/60 px-2.5 py-2 text-[11.5px] text-fg-muted break-words"
        >
          {report.message}
        </p>

        <p class="mt-2 text-[11px] leading-relaxed text-fg-subtle">
          The report is redacted — only the error, a sanitised stack trace, your
          OS, and the app version. No projects, paths, or credentials.
        </p>
      {/if}

      <!-- Stack trace expander (shared by both states) -->
      {#if report.backtrace}
        <div class="mt-3 rounded-md border border-border overflow-hidden">
          <button
            type="button"
            onclick={() => (showTrace = !showTrace)}
            class="flex w-full items-center gap-1.5 px-3 py-2 text-[12px] text-fg-muted hover:bg-surface-2 transition-colors"
          >
            <Icon name={showTrace ? "chevron-down" : "chevron-right"} size={13} />
            View stack trace
          </button>
          {#if showTrace}
            <pre
              class="max-h-44 overflow-auto border-t border-border bg-bg/80 px-3 py-2 text-[10.5px] leading-relaxed text-fg-muted whitespace-pre-wrap break-words">{report.backtrace}</pre>
          {/if}
        </div>
      {/if}

      <!-- Actions -->
      {#if sent}
        <div class="mt-4 flex flex-col items-center gap-1">
          <button
            type="button"
            onclick={() => crashSurface.dismiss()}
            class="h-8 px-4 rounded-lg bg-accent text-on-accent text-[12.5px] font-semibold hover:brightness-110 active:brightness-95 transition shadow-sm"
          >
            Close
          </button>
          <p class="mt-1 text-[11px] text-fg-subtle">
            Thank you for helping improve PortBay.
          </p>
        </div>
      {:else}
        {#if phase === "error"}
          <p class="mt-3 text-[11.5px] text-red-400">
            Couldn't send the report. Check your connection and try again.
          </p>
        {/if}
        <div class="mt-4 flex items-center gap-2">
          <button
            type="button"
            onclick={() => void crashSurface.send()}
            disabled={phase === "sending"}
            class="inline-flex items-center gap-1.5 h-8 px-3 rounded-lg bg-accent text-on-accent text-[12.5px] font-semibold hover:brightness-110 active:brightness-95 transition shadow-sm disabled:opacity-60"
          >
            {phase === "sending"
              ? "Sending…"
              : phase === "error"
                ? "Try again"
                : "Send report"}
          </button>
          <button
            type="button"
            onclick={() => void crashSurface.discard()}
            disabled={phase === "sending"}
            class="h-8 px-2.5 rounded-lg text-[12.5px] font-medium text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors disabled:opacity-60"
          >
            Discard
          </button>
        </div>
      {/if}
    </div>
  {/if}
{/snippet}

{#if crashSurface.isOpen}
  {#if mode === "crash"}
    <!-- Centred modal: the app actually crashed, so it earns the spotlight. -->
    <div
      class="fixed inset-0 z-[60] grid place-items-center bg-black/40 backdrop-blur-[1px] p-4"
      transition:fade={{ duration: 150 }}
    >
      <div
        class="w-[440px] max-w-full rounded-2xl bg-bg border border-border shadow-2xl overflow-hidden"
        role="dialog"
        aria-modal="true"
        aria-label={title}
        transition:fly={{ y: 12, duration: 200 }}
      >
        {@render content()}
      </div>
    </div>
  {:else}
    <!-- Quiet bottom-right card for a recoverable live error. -->
    <div
      class="fixed bottom-4 right-4 z-[55] w-[360px] max-w-full rounded-2xl bg-bg border border-border shadow-2xl overflow-hidden"
      role="dialog"
      aria-label={title}
      transition:fly={{ y: 16, duration: 220 }}
    >
      {@render content()}
    </div>
  {/if}
{/if}
