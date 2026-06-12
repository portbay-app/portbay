<!--
  EmbeddingsPlayground — embed text with an installed Ollama embedding model
  (`ollama_embed` → `/api/embed`) and show the vector dimension, a value
  sparkline, and the cosine similarity between two inputs. A real, useful
  demo of what an embedding model does, not a placeholder.
-->
<script lang="ts">
  import { invokeQuiet, normalise } from "$lib/ipc";
  import Icon from "$lib/components/atoms/Icon.svelte";
  import type { OllamaEmbedResult } from "$lib/types/ai";

  interface Props {
    /** Installed model names from the AI overview (parent owns the list). */
    models: string[];
    running: boolean;
  }
  let { models, running }: Props = $props();

  // Prefer embedding-named models, but allow any installed model — Ollama
  // returns an embedding for most models regardless.
  const ordered = $derived(
    [...models].sort((a, b) => Number(b.includes("embed")) - Number(a.includes("embed"))),
  );
  let model = $state<string>("");
  $effect(() => {
    if (!model && ordered.length) model = ordered[0];
  });

  let inputA = $state("");
  let inputB = $state("");
  let busy = $state(false);
  let error = $state<string | null>(null);
  let result = $state<OllamaEmbedResult | null>(null);

  const vectorA = $derived(result?.embeddings?.[0] ?? null);
  const vectorB = $derived(result?.embeddings?.[1] ?? null);

  function cosine(a: number[], b: number[]): number | null {
    if (!a?.length || !b?.length || a.length !== b.length) return null;
    let dot = 0;
    let na = 0;
    let nb = 0;
    for (let i = 0; i < a.length; i++) {
      dot += a[i] * b[i];
      na += a[i] * a[i];
      nb += b[i] * b[i];
    }
    const denom = Math.sqrt(na) * Math.sqrt(nb);
    return denom === 0 ? null : dot / denom;
  }
  const similarity = $derived(vectorA && vectorB ? cosine(vectorA, vectorB) : null);

  /** A compact <svg> sparkline of the first 64 vector values, centred on 0. */
  function sparkPath(vec: number[], width = 320, height = 36): string {
    const slice = vec.slice(0, 64);
    if (!slice.length) return "";
    const max = Math.max(...slice.map((v) => Math.abs(v))) || 1;
    const step = width / Math.max(1, slice.length - 1);
    return slice
      .map((v, i) => `${i === 0 ? "M" : "L"}${(i * step).toFixed(1)},${(height / 2 - (v / max) * (height / 2 - 2)).toFixed(1)}`)
      .join(" ");
  }

  async function embed() {
    if (!model) return;
    error = null;
    busy = true;
    result = null;
    try {
      result = await invokeQuiet<OllamaEmbedResult>("ollama_embed", {
        model,
        input: [inputA, inputB],
      });
    } catch (e) {
      error = normalise(e).whatHappened;
    } finally {
      busy = false;
    }
  }
</script>

<section id="embeddings" class="w-full">
  <div class="grid gap-4 xl:grid-cols-[minmax(0,1.4fr)_minmax(0,1fr)]">
    <div class="min-w-0 rounded-lg border border-border bg-surface p-4">
      <div class="flex flex-wrap items-center justify-between gap-2">
        <div class="flex items-center gap-2">
          <Icon name="layers" size={14} class="text-fg-muted" />
          <h2 class="text-[14px] font-semibold text-fg">Embeddings</h2>
        </div>
        <label class="flex items-center gap-2 text-[11px] text-fg-subtle">
          Model
          <select class="rounded-md border border-border bg-bg px-2 py-1.5 text-[12px] text-fg" bind:value={model}>
            {#if ordered.length === 0}
              <option value="">No models installed</option>
            {/if}
            {#each ordered as m}
              <option value={m}>{m}</option>
            {/each}
          </select>
        </label>
      </div>

      <p class="mt-1 text-[11px] text-fg-subtle">
        Turn text into a vector. Embed two inputs to see how similar the model thinks they are.
      </p>

      <label class="mt-3 block">
        <span class="text-[11px] text-fg-subtle">Input A</span>
        <textarea
          bind:value={inputA}
          rows="2"
          placeholder="A sentence to embed…"
          class="mt-1 w-full resize-y rounded-md border border-border bg-bg px-2.5 py-1.5 text-[12px] text-fg focus:outline-none focus:border-accent/60"
        ></textarea>
      </label>
      <label class="mt-2 block">
        <span class="text-[11px] text-fg-subtle">Input B <span class="text-fg-subtle/70">(optional — for similarity)</span></span>
        <textarea
          bind:value={inputB}
          rows="2"
          placeholder="Another sentence to compare…"
          class="mt-1 w-full resize-y rounded-md border border-border bg-bg px-2.5 py-1.5 text-[12px] text-fg focus:outline-none focus:border-accent/60"
        ></textarea>
      </label>

      <div class="mt-3 flex items-center gap-2">
        <button
          class="inline-flex items-center gap-1.5 rounded-md bg-accent px-3 py-1.5 text-[12px] font-semibold text-on-accent disabled:opacity-50"
          disabled={busy || !model || !running || !inputA.trim()}
          onclick={embed}
        >
          <Icon name={busy ? "loader-circle" : "sparkles"} size={13} class={busy ? "animate-spin" : ""} />
          {busy ? "Embedding…" : "Embed"}
        </button>
        {#if !running}
          <span class="text-[10.5px] text-status-warning">Start the server to embed.</span>
        {:else if ordered.length === 0}
          <span class="text-[10.5px] text-fg-subtle">Install an Ollama model first — any model can embed.</span>
        {/if}
      </div>

      {#if error}
        <p class="mt-3 text-[11px] text-status-unhealthy">{error}</p>
      {/if}
    </div>

    <div class="min-w-0 space-y-4">
      <div class="rounded-lg border border-border bg-surface p-4">
        <h3 class="text-[13px] font-semibold text-fg">Result</h3>
        {#if !result}
          <p class="mt-2 text-[12px] text-fg-subtle">Embed an input to see its vector dimension and shape.</p>
        {:else}
          <dl class="mt-3 grid grid-cols-2 gap-2">
            <div class="rounded-md border border-border bg-surface-2/40 px-3 py-2">
              <dt class="text-[10px] uppercase tracking-wide text-fg-subtle">Dimensions</dt>
              <dd class="mt-0.5 font-mono text-[13px] text-fg">{vectorA?.length ?? "—"}</dd>
            </div>
            <div class="rounded-md border border-border bg-surface-2/40 px-3 py-2">
              <dt class="text-[10px] uppercase tracking-wide text-fg-subtle">Cosine similarity</dt>
              <dd class="mt-0.5 font-mono text-[13px] text-fg">{similarity !== null ? similarity.toFixed(4) : "—"}</dd>
            </div>
          </dl>
          {#if similarity !== null}
            <div class="mt-2 h-1.5 overflow-hidden rounded-full bg-bg">
              <div class="h-full bg-accent transition-all" style={`width:${Math.round(Math.max(0, similarity) * 100)}%`}></div>
            </div>
            <p class="mt-1 text-[10.5px] text-fg-subtle">
              {similarity > 0.8 ? "Very similar" : similarity > 0.5 ? "Somewhat similar" : "Not very similar"} — 1.0 is identical.
            </p>
          {/if}
          {#if vectorA}
            <p class="mt-3 text-[10px] uppercase tracking-wide text-fg-subtle">Input A · first 64 values</p>
            <svg viewBox="0 0 320 36" class="mt-1 h-9 w-full" preserveAspectRatio="none">
              <line x1="0" y1="18" x2="320" y2="18" stroke="currentColor" class="text-border" stroke-width="0.5" />
              <path d={sparkPath(vectorA)} fill="none" stroke="currentColor" class="text-accent" stroke-width="1" />
            </svg>
          {/if}
        {/if}
      </div>
    </div>
  </div>
</section>
