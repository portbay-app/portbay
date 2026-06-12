<!--
  DictationRewriteChip — inline status for the Smart Dictation rewrite layer.

  Renders nothing while idle, so surfaces can keep it mounted next to their
  mic button unconditionally. Four states, mirroring the controller:
    • rewriting — spinner + cancel (keeps the raw transcript already typed)
    • done      — confirmation + Undo (reverts to the raw transcript, or for
                  a voice edit, to the pre-edit original)
    • kept-raw  — quiet note that the rewrite didn't happen (auto-dismisses)
    • restored  — voice edit failed, OR an explicit action was a genuine
                  no-op (already clean); the original text stands
    • unavailable — an explicit Writing Tools action couldn't run at all
                  (provider unreachable / no model / empty output), distinct
                  from "Kept original" so the failure is debuggable
-->

<script lang="ts">
  import { fade } from "svelte/transition";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import type { DictationRewriter } from "$lib/dictation/rewriter.svelte";

  let { rewriter }: { rewriter: DictationRewriter } = $props();
</script>

{#if rewriter.phase !== "idle"}
  <span
    transition:fade={{ duration: 120 }}
    class="inline-flex h-6 shrink-0 items-center gap-1.5 rounded-full border px-2 text-[11px]
      {rewriter.phase === 'done'
      ? 'border-accent/40 bg-accent/10 text-accent'
      : 'border-border bg-surface-2 text-fg-muted'}"
    role="status"
    aria-live="polite"
  >
    {#if rewriter.phase === "rewriting"}
      <Icon name="refresh-cw" size={11} class="animate-spin" />
      <span class="whitespace-nowrap">Polishing…</span>
      <button
        type="button"
        onclick={() => rewriter.cancel()}
        title="Keep the words as spoken"
        aria-label="Cancel rewrite and keep the raw transcript"
        class="grid h-4 w-4 place-items-center rounded-full text-fg-subtle hover:bg-surface hover:text-fg"
      >
        <Icon name="x" size={10} />
      </button>
    {:else if rewriter.phase === "done"}
      <Icon name="circle-check" size={11} />
      <span class="whitespace-nowrap">Polished</span>
      <button
        type="button"
        onclick={() => rewriter.undo()}
        title="Revert to the words as spoken"
        class="inline-flex items-center gap-0.5 rounded-full px-1 font-medium hover:bg-accent/15"
      >
        <Icon name="rotate-ccw" size={10} /> Undo
      </button>
    {:else if rewriter.phase === "unavailable"}
      <Icon name="circle-alert" size={11} />
      <span
        class="whitespace-nowrap"
        title="The rewrite model didn't return anything — check AI → Speech-to-Text (is your provider reachable and a model selected?)."
        >Rewrite unavailable</span
      >
    {:else if rewriter.phase === "restored"}
      <span class="whitespace-nowrap">Kept original</span>
    {:else}
      <span class="whitespace-nowrap">Kept as spoken</span>
    {/if}
  </span>
{/if}
