<!--
  DeployStepsView — the live run output shared by the ad-hoc Run/Deploy pane
  and the project deploy section. Renders one card per step: status icon,
  command, live elapsed / final duration + exit code, and the streamed output
  (ANSI-rendered, stick-to-bottom autoscroll, incremental conversion so a
  chatty build doesn't re-render its whole history per chunk).

  Cards expand while running and on failure, collapse once a step succeeds;
  a manual toggle always wins over the automatic state. The summary line
  (all-ok / stopped-at / cancelled + total time) lives here too so both hosts
  stay consistent.
-->
<script lang="ts">
  import Icon from "$lib/components/atoms/Icon.svelte";
  import { AnsiAppender } from "$lib/components/logs/ansi";
  import { formatDuration, summarize, type LiveStep, type LiveStepStatus } from "$lib/deployLive";

  let { steps }: { steps: LiveStep[] } = $props();

  const summary = $derived(summarize(steps));

  // Incremental ANSI → HTML, one appender per step index. `push` only converts
  // the appended suffix and returns cached HTML otherwise, so calling it from
  // a $derived is cheap and idempotent. A new run (fresh steps array) starts
  // its outputs back at "" which resets the appenders.
  const appenders: AnsiAppender[] = [];
  function appender(i: number): AnsiAppender {
    while (appenders.length <= i) appenders.push(new AnsiAppender());
    return appenders[i];
  }
  const htmls = $derived(steps.map((s, i) => appender(i).push(s.output)));

  // Manual expand/collapse overrides, reset whenever a new run replaces the
  // steps array.
  let overrides = $state<Record<number, boolean>>({});
  let lastSteps: LiveStep[] | undefined;
  $effect(() => {
    if (steps !== lastSteps) {
      lastSteps = steps;
      overrides = {};
    }
  });
  function isOpen(i: number, status: LiveStepStatus): boolean {
    return overrides[i] ?? (status === "running" || status === "failed" || status === "cancelled");
  }

  // 500 ms ticker for the running step's live elapsed readout.
  let now = $state(Date.now());
  $effect(() => {
    if (!summary.running) return;
    const t = setInterval(() => (now = Date.now()), 500);
    return () => clearInterval(t);
  });

  /** Stick-to-bottom autoscroll: follows new output unless the user scrolled
      up, re-engages when they return to the bottom. */
  function autoscroll(node: HTMLElement, _dep: number) {
    let stick = true;
    const onScroll = () => {
      stick = node.scrollTop + node.clientHeight >= node.scrollHeight - 8;
    };
    node.addEventListener("scroll", onScroll);
    node.scrollTop = node.scrollHeight;
    return {
      update(_d: number) {
        if (stick) node.scrollTop = node.scrollHeight;
      },
      destroy() {
        node.removeEventListener("scroll", onScroll);
      },
    };
  }

  function copyOutput(step: LiveStep) {
    // Strip ANSI escapes from the copied text (same CSI matcher as the logs).
    // eslint-disable-next-line no-control-regex
    void navigator.clipboard.writeText(step.output.replace(/\[[0-9;?]*[A-Za-z]/g, ""));
  }

  const STATUS_ICON: Record<LiveStepStatus, { name: "circle-dot" | "refresh-cw" | "circle-check" | "circle-alert" | "ban" | "minus"; class: string }> = {
    queued: { name: "circle-dot", class: "text-fg-subtle" },
    running: { name: "refresh-cw", class: "animate-spin text-accent" },
    ok: { name: "circle-check", class: "text-status-running" },
    failed: { name: "circle-alert", class: "text-status-crashed" },
    cancelled: { name: "ban", class: "text-fg-muted" },
    skipped: { name: "minus", class: "text-fg-subtle" },
  };

  function headerClass(status: LiveStepStatus): string {
    switch (status) {
      case "ok":
        return "bg-status-running/10";
      case "failed":
        return "bg-status-crashed/10";
      case "running":
        return "bg-accent/10";
      default:
        return "bg-surface-2/40";
    }
  }

  function timing(step: LiveStep): string {
    if (step.status === "running" && step.startedAt !== null) {
      return formatDuration(Math.max(0, now - step.startedAt));
    }
    if (step.durationMs !== null) return formatDuration(step.durationMs);
    return "";
  }
</script>

<div class="space-y-2">
  {#each steps as step, i (i)}
    {@const open = isOpen(i, step.status)}
    {@const hasOutput = step.output !== ""}
    <div
      class="overflow-hidden rounded-md border border-border/70
             {step.status === 'queued' || step.status === 'skipped' ? 'opacity-60' : ''}"
    >
      <div class="flex items-center gap-2 px-3 py-1.5 text-[12px] {headerClass(step.status)}">
        <Icon
          name={STATUS_ICON[step.status].name}
          size={13}
          class={STATUS_ICON[step.status].class}
        />
        <code class="flex-1 truncate font-mono text-fg" title={step.command}>{step.command}</code>
        {#if timing(step)}
          <span class="shrink-0 font-mono text-[11px] tabular-nums text-fg-subtle">{timing(step)}</span>
        {/if}
        {#if step.exitCode !== null && step.exitCode !== 0 && step.status !== "cancelled"}
          <span class="shrink-0 font-mono text-[11px] text-status-crashed">exit {step.exitCode}</span>
        {:else if step.status === "skipped"}
          <span class="shrink-0 text-[11px] text-fg-subtle">skipped</span>
        {:else if step.status === "cancelled"}
          <span class="shrink-0 text-[11px] text-fg-muted">cancelled</span>
        {/if}
        {#if hasOutput}
          <button
            type="button"
            onclick={() => copyOutput(step)}
            class="shrink-0 rounded p-0.5 text-fg-subtle hover:bg-surface-2 hover:text-fg"
            aria-label="Copy output"
            title="Copy output"
          >
            <Icon name="copy" size={12} />
          </button>
          <button
            type="button"
            onclick={() => (overrides[i] = !open)}
            class="shrink-0 rounded p-0.5 text-fg-subtle hover:bg-surface-2 hover:text-fg"
            aria-label={open ? "Collapse output" : "Expand output"}
          >
            <Icon name={open ? "chevron-down" : "chevron-right"} size={12} />
          </button>
        {/if}
      </div>
      {#if hasOutput && open}
        <pre
          use:autoscroll={step.output.length}
          class="max-h-56 overflow-auto whitespace-pre-wrap break-words bg-surface-2/50 px-3 py-2 font-mono text-[11px] leading-relaxed text-fg">{@html htmls[i]}</pre>
      {/if}
    </div>
  {/each}

  {#if steps.length > 0 && !summary.running}
    {#if summary.allOk}
      <p class="text-[12px] font-medium text-status-running">
        All steps succeeded{summary.totalMs > 0 ? ` in ${formatDuration(summary.totalMs)}` : ""}.
      </p>
    {:else if summary.cancelled}
      <p class="text-[12px] font-medium text-fg-muted">Run cancelled.</p>
    {:else if summary.failedAt !== -1}
      <p class="text-[12px] font-medium text-status-crashed">
        Stopped at step {summary.failedAt + 1} (non-zero exit). Later steps were skipped.
      </p>
    {/if}
  {/if}
</div>
