<!--
  NotificationsPanel — anchored popover triggered from the topbar bell.

  Lists the last 50 envelopes the error bus has seen, newest first.
  Click outside or hit Escape to close. "Mark all read" zeroes the
  unread badge without losing history; "Clear" wipes the store.

  Severity-aware: errors render in status-crashed, warnings in
  status-unhealthy, info/success in fg-muted. The dot mirrors the
  toast colour so the two surfaces feel like the same system.
-->
<script lang="ts">
  import Icon, { type IconName } from "$lib/components/atoms/Icon.svelte";
  import { notifications } from "$lib/stores/notifications.svelte";
  import { activity, type ActivityKind } from "$lib/stores/activity.svelte";
  import type { CommandError } from "$lib/types/error";

  interface Props {
    open: boolean;
    onclose: () => void;
  }
  let { open, onclose }: Props = $props();

  let panelEl: HTMLDivElement | undefined = $state();

  function onWindowKey(e: KeyboardEvent) {
    if (open && e.key === "Escape") onclose();
  }

  function onWindowClick(e: MouseEvent) {
    if (!open || !panelEl) return;
    const target = e.target as Node | null;
    if (target && !panelEl.contains(target)) onclose();
  }

  // Mark visible items read when the panel opens. We schedule rather
  // than mark inline so the badge transition is visible to the user.
  $effect(() => {
    if (!open) return;
    const handle = setTimeout(() => {
      void notifications.markAllRead();
      void activity.markAllRead();
    }, 600);
    return () => clearTimeout(handle);
  });

  function markAllRead() {
    void notifications.markAllRead();
    void activity.markAllRead();
  }
  function clearAll() {
    notifications.clear();
    void activity.clear();
  }

  type Tone = "error" | "warn" | "info" | "success";

  // Agent-activity presentation: tone, circle icon, and the verb that joins
  // "<agent> … <card>". Comments read as info; blocks as errors; warnings warn.
  const activityTone: Record<ActivityKind, Tone> = {
    done: "success",
    comment: "info",
    blocked: "error",
    warning: "warn",
    learning: "info",
  };
  const activityIcon: Record<ActivityKind, IconName> = {
    done: "circle-check",
    comment: "bot",
    blocked: "circle-alert",
    warning: "circle-alert",
    learning: "sparkles",
  };
  const activityVerb: Record<ActivityKind, string> = {
    done: "finished",
    comment: "commented on",
    blocked: "blocked",
    warning: "flagged",
    learning: "recorded a learning",
  };

  function severityTone(e: CommandError): Tone {
    if (e.severity === "success") return "success";
    if (e.severity === "info") return "info";
    if (e.severity === "warning") return "warn";
    if (e.whoCausedIt === "system" && e.actions.length === 0 && !e.severity) {
      return "info";
    }
    return "error";
  }

  function relativeTime(ts: number): string {
    const delta = Date.now() - ts;
    if (delta < 60_000) return "just now";
    const mins = Math.floor(delta / 60_000);
    if (mins < 60) return `${mins}m ago`;
    const hrs = Math.floor(mins / 60);
    if (hrs < 24) return `${hrs}h ago`;
    const days = Math.floor(hrs / 24);
    return `${days}d ago`;
  }

  // Full per-tone class strings — pre-composed so Tailwind's JIT can
  // see the literal `bg-status-crashed/15` etc. tokens at build time.
  const toneIconWrap: Record<Tone, string> = {
    error: "bg-status-crashed/15 text-status-crashed",
    warn: "bg-status-unhealthy/15 text-status-unhealthy",
    info: "bg-accent/15 text-accent",
    success: "bg-status-running/15 text-status-running",
  };

  function iconFor(tone: Tone): "circle-alert" | "info" | "circle-check" {
    if (tone === "error" || tone === "warn") return "circle-alert";
    if (tone === "success") return "circle-check";
    return "info";
  }
</script>

<svelte:window onkeydown={onWindowKey} onclick={onWindowClick} />

{#if open}
  <div
    bind:this={panelEl}
    class="absolute right-0 top-12 z-50 w-80 max-h-[28rem] flex flex-col
           rounded-xl border border-border bg-surface shadow-2xl
           overflow-hidden"
    role="dialog"
    aria-label="Notifications"
  >
    <header
      class="shrink-0 flex items-center justify-between px-4 py-2.5
             border-b border-border bg-surface-2/40"
    >
      <h2 class="text-[13px] font-semibold text-fg">Notifications</h2>
      <div class="flex items-center gap-1">
        <button
          type="button"
          onclick={markAllRead}
          title="Mark all read"
          class="text-[11px] text-fg-muted hover:text-fg px-1.5 py-0.5 rounded"
        >
          Mark read
        </button>
        <button
          type="button"
          onclick={clearAll}
          title="Clear history"
          class="text-[11px] text-fg-muted hover:text-status-crashed px-1.5 py-0.5 rounded"
        >
          Clear
        </button>
      </div>
    </header>

    {#if activity.value.length === 0 && notifications.value.length === 0}
      <p class="px-4 py-10 text-center text-[12px] text-fg-subtle">
        Nothing has happened yet. Agent activity, toasts, errors, and system
        events will appear here.
      </p>
    {:else}
      <div class="flex-1 min-h-0 overflow-y-auto">
        <!-- Agent activity: comments, blocks, and warnings from your agents,
             across every project. Click an item to open its card. -->
        {#if activity.value.length > 0}
          <div class="px-3 pt-2.5 pb-1 text-[10px] font-semibold uppercase tracking-wide text-fg-subtle">
            Agent activity
          </div>
          <ul class="divide-y divide-border/60">
            {#each activity.value as n (n.id)}
              {@const tone = activityTone[n.kind]}
              <li>
                <button
                  type="button"
                  onclick={() => {
                    void activity.open(n);
                    onclose();
                  }}
                  class="w-full flex gap-2.5 px-3 py-2.5 text-left hover:bg-surface-2/60 transition-colors"
                >
                  <span
                    class="mt-0.5 inline-flex items-center justify-center w-5 h-5 rounded-full {toneIconWrap[tone]}"
                  >
                    <Icon name={activityIcon[n.kind]} size={12} />
                  </span>
                  <div class="flex-1 min-w-0">
                    <p class="text-[12px] text-fg truncate">
                      <span class="font-medium">{n.agent ?? "Agent"}</span>
                      {activityVerb[n.kind]}
                      <span class="text-fg-muted">{n.cardTitle}</span>
                    </p>
                    {#if n.body}
                      <p class="text-[11px] text-fg-muted line-clamp-2">{n.body}</p>
                    {/if}
                    <p class="mt-1 text-[10px] text-fg-subtle tabular-nums truncate">
                      {n.projectName} · {relativeTime(n.createdMs)}
                    </p>
                  </div>
                  {#if !n.read}
                    <span class="mt-1.5 w-1.5 h-1.5 rounded-full bg-accent shrink-0" title="Unread"></span>
                  {/if}
                </button>
              </li>
            {/each}
          </ul>
        {/if}

        <!-- System: the toast/error history (command failures, status events). -->
        {#if notifications.value.length > 0}
          {#if activity.value.length > 0}
            <div class="px-3 pt-3 pb-1 text-[10px] font-semibold uppercase tracking-wide text-fg-subtle border-t border-border/60">
              System
            </div>
          {/if}
          <ul class="divide-y divide-border/60">
            {#each notifications.value as entry (entry.id)}
              {@const tone = severityTone(entry.envelope)}
              <li class="flex gap-2.5 px-3 py-2.5">
                <span
                  class="mt-0.5 inline-flex items-center justify-center w-5 h-5 rounded-full {toneIconWrap[tone]}"
                >
                  <Icon name={iconFor(tone)} size={12} />
                </span>
                <div class="flex-1 min-w-0">
                  <p class="text-[12px] text-fg truncate">
                    {entry.envelope.whatHappened}
                  </p>
                  {#if entry.envelope.whyItMatters}
                    <p class="text-[11px] text-fg-muted line-clamp-2">
                      {entry.envelope.whyItMatters}
                    </p>
                  {/if}
                  <p class="mt-1 text-[10px] text-fg-subtle tabular-nums">
                    {relativeTime(entry.receivedAt)}
                  </p>
                </div>
                {#if !entry.read}
                  <span
                    class="mt-1.5 w-1.5 h-1.5 rounded-full bg-accent shrink-0"
                    title="Unread"
                  ></span>
                {/if}
              </li>
            {/each}
          </ul>
        {/if}
      </div>
    {/if}
  </div>
{/if}
