<!--
  FeedbackPrompt — a quiet, dismissible bottom-right card that appears once when
  an install-age milestone is reached (24h → feedback, 1 week → review/star).
  Universal: reaches anonymous users the lifecycle emails can't. Mounted once on
  the main window; renders nothing until a prompt is due.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { fly } from "svelte/transition";
  import { openUrl } from "@tauri-apps/plugin-opener";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import LighthouseLogo from "$lib/components/atoms/LighthouseLogo.svelte";
  import { lifecycle, type LifecyclePrompt } from "$lib/stores/lifecycle.svelte";

  const FEEDBACK_URL = "https://github.com/portbay-app/portbay/discussions/new?category=feedback";
  const REVIEW_URL = "https://github.com/portbay-app/portbay";

  // Don't surface instantly on launch — let the app settle first.
  let armed = $state(false);
  onMount(() => {
    lifecycle.evaluate();
    const t = setTimeout(() => (armed = true), 4000);
    return () => clearTimeout(t);
  });

  const due = $derived(armed ? lifecycle.due : null);

  const copy: Record<LifecyclePrompt, { title: string; body: string; cta: string; icon: "sparkles" | "star"; url: string }> = {
    feedback24h: {
      title: "How's your first day?",
      body: "What's working well — and what got in your way? Even one sentence shapes what we build next.",
      cta: "Share feedback",
      icon: "sparkles",
      url: FEEDBACK_URL,
    },
    review1week: {
      title: "A week in — enjoying PortBay?",
      body: "If it's saving you time, a star on GitHub helps other developers find it. Takes ten seconds.",
      cta: "Star on GitHub",
      icon: "star",
      url: REVIEW_URL,
    },
  };

  const current = $derived(due ? copy[due] : null);

  function act() {
    if (!due) return;
    void openUrl(copy[due].url);
    lifecycle.complete(due);
  }

  function dismiss() {
    if (due) lifecycle.complete(due);
  }
</script>

{#if current && due}
  <div
    class="fixed bottom-4 right-4 z-[55] w-[340px] rounded-2xl bg-bg border border-border
           shadow-2xl overflow-hidden"
    role="dialog"
    aria-label={current.title}
    transition:fly={{ y: 16, duration: 220 }}
  >
    <div class="p-4">
      <div class="flex items-start gap-3">
        <LighthouseLogo size={28} />
        <div class="min-w-0 flex-1">
          <div class="flex items-start justify-between gap-2">
            <h3 class="text-[13.5px] font-semibold text-fg leading-snug">{current.title}</h3>
            <button
              type="button"
              onclick={dismiss}
              aria-label="Dismiss"
              class="-mr-1 -mt-0.5 shrink-0 grid place-items-center w-6 h-6 rounded-md text-fg-subtle hover:text-fg hover:bg-surface-2 transition-colors"
            >
              <Icon name="x" size={14} />
            </button>
          </div>
          <p class="mt-1 text-[12px] leading-relaxed text-fg-muted">{current.body}</p>
          <div class="mt-3 flex items-center gap-2">
            <button
              type="button"
              onclick={act}
              class="inline-flex items-center gap-1.5 h-8 px-3 rounded-lg bg-accent text-on-accent text-[12.5px] font-semibold hover:brightness-110 active:brightness-95 transition shadow-sm"
            >
              <Icon name={current.icon} size={13} />
              {current.cta}
            </button>
            <button
              type="button"
              onclick={dismiss}
              class="h-8 px-2.5 rounded-lg text-[12.5px] font-medium text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
            >
              Maybe later
            </button>
          </div>
        </div>
      </div>
    </div>
  </div>
{/if}
