<!--
  StopAllButton — universal kill switch (docs/UX_DESIGN.md §5.1).

  The product's single most important reliability promise. Three visual
  states:
    - idle       : red circle-stop icon; click → confirm
    - confirming : expands to "Stop N running?" with confirm/cancel,
                   4s timeout reverts to idle
    - in-progress: spinner; disabled

  Keyboard shortcut: ⇧⌘. enters the confirming state (one keystroke is
  enough; the confirm is the safety net).
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { errorBus } from "$lib/stores/errors.svelte";
  import { safeInvoke } from "$lib/ipc";
  import { projects } from "$lib/stores/projects.svelte";
  import Icon from "$lib/components/atoms/Icon.svelte";
  import type { StopAllReport } from "$lib/types/stopAll";

  type State = "idle" | "confirming" | "running";
  let state = $state<State>("idle");
  let confirmTimer: ReturnType<typeof setTimeout> | null = null;

  const runningCount = $derived(
    projects.value.filter(
      (p) => p.status === "running" || p.status === "starting",
    ).length,
  );

  function clearConfirmTimer() {
    if (confirmTimer !== null) {
      clearTimeout(confirmTimer);
      confirmTimer = null;
    }
  }

  function enterConfirming() {
    if (state !== "idle") return;
    if (runningCount === 0) {
      // Nothing to do — silently surface a quiet hint.
      errorBus.push({
        code: "NOTHING_TO_STOP",
        category: "lifecycle",
        whatHappened: "Nothing to stop.",
        whyItMatters: "No projects are currently running.",
        whoCausedIt: "user",
        actions: [],
      });
      return;
    }
    state = "confirming";
    confirmTimer = setTimeout(() => {
      if (state === "confirming") state = "idle";
    }, 4_000);
  }

  function cancel() {
    state = "idle";
    clearConfirmTimer();
  }

  async function commit() {
    clearConfirmTimer();
    state = "running";
    try {
      const report = await safeInvoke<StopAllReport>("stop_all");
      reportSummary(report);
    } catch {
      // safeInvoke already pushed the toast (e.g. SIDECAR_DOWN).
    } finally {
      state = "idle";
    }
  }

  function reportSummary(report: StopAllReport) {
    const total = report.stopped + report.failed;
    if (total === 0) {
      // Race: between the prompt and the call, all projects finished
      // stopping. Don't bother the user with a "nothing happened" toast.
      return;
    }

    if (report.failed === 0) {
      errorBus.push({
        code: "STOP_ALL_OK",
        category: "lifecycle",
        whatHappened: `Stopped ${report.stopped} project${report.stopped === 1 ? "" : "s"}.`,
        whyItMatters: "All running projects were brought down cleanly.",
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });
      return;
    }

    const failureList = report.results
      .filter((r) => !r.ok)
      .map((r) => `${r.id}: ${r.error ?? "unknown error"}`)
      .join("\n");
    errorBus.push({
      code: "STOP_ALL_PARTIAL",
      category: "project-error",
      whatHappened: `Stopped ${report.stopped} of ${total}. ${report.failed} failed.`,
      whyItMatters:
        "Some projects didn't stop cleanly. Their state may not match the table.",
      whoCausedIt: "system",
      actions: [],
      details: failureList,
    });
  }

  // Keyboard shortcut ⇧⌘. — single binding for the universal kill switch.
  function handleKey(e: KeyboardEvent) {
    if (
      e.key === "." &&
      e.shiftKey &&
      (e.metaKey || e.ctrlKey) &&
      state === "idle"
    ) {
      e.preventDefault();
      enterConfirming();
    } else if (e.key === "Escape" && state === "confirming") {
      cancel();
    }
  }

  onMount(() => {
    return () => clearConfirmTimer();
  });
</script>

<svelte:window onkeydown={handleKey} />

{#if state === "confirming"}
  <div
    class="flex items-center gap-1.5 h-9 px-2 rounded-lg border border-status-crashed/40 bg-status-crashed/10"
  >
    <span class="text-[12px] text-status-crashed pr-1">
      Stop {runningCount} running?
    </span>
    <button
      type="button"
      onclick={commit}
      class="inline-flex items-center justify-center w-7 h-7 rounded-md text-status-crashed hover:bg-status-crashed/20 transition-colors"
      aria-label="Confirm stop all"
    >
      <Icon name="check" size={14} />
    </button>
    <button
      type="button"
      onclick={cancel}
      class="inline-flex items-center justify-center w-7 h-7 rounded-md text-fg-muted hover:bg-surface-2 transition-colors"
      aria-label="Cancel stop all"
    >
      <Icon name="x" size={14} />
    </button>
  </div>
{:else}
  <!--
    Idle / running state — primary destructive button. The redesign
    promotes this from the previous icon-only treatment to a filled
    pill so it reads as a peer to the "+ Add Project" action. The
    button stays filled even when nothing is running (just dimmed +
    disabled) so the top bar's visual rhythm doesn't shift as
    projects spin up.
  -->
  <button
    type="button"
    onclick={enterConfirming}
    disabled={state === "running" || runningCount === 0}
    title={runningCount === 0
      ? "Nothing to stop"
      : `Stop all ${runningCount} running projects (⇧⌘.)`}
    aria-label="Stop all running projects"
    class="inline-flex items-center gap-1.5 h-9 px-3 rounded-lg
           text-[13px] font-medium tracking-tight
           bg-status-crashed text-on-accent
           shadow-sm hover:brightness-110 active:brightness-95
           transition disabled:opacity-50 disabled:cursor-not-allowed
           focus-visible:outline-none focus-visible:ring-2
           focus-visible:ring-status-crashed/40"
  >
    {#if state === "running"}
      <Icon name="refresh-cw" size={13} class="animate-spin" />
      Stopping…
    {:else}
      <Icon name="square" size={11} class="fill-current" />
      Stop All
    {/if}
  </button>
{/if}
