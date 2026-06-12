<script lang="ts">
  import { Channel } from "@tauri-apps/api/core";
  import { invokeQuiet, normalise } from "$lib/ipc";
  import Icon from "$lib/components/atoms/Icon.svelte";
  import type { TtsOverview, TtsCatalogModel, SttDownloadEvent } from "$lib/types/ai";

  let info = $state<TtsOverview | null>(null);
  let model = $derived<TtsCatalogModel | null>(info?.catalog?.[0] ?? null);
  let installed = $derived(
    !!model && (info?.installed?.some((m) => m.id === model!.id) ?? false),
  );

  let voice = $state<string>("");
  let text = $state("");
  let speaking = $state(false);
  let downloading = $state(false);
  let downloadPct = $state(0);
  let error = $state<string | null>(null);
  let audio: HTMLAudioElement | null = null;
  // Last synthesized clip, kept so it can be replayed and exported as .wav.
  let lastWav = $state<string | null>(null);
  // What the last clip was synthesized from. The primary button reads
  // "Replay" only while text + voice still match — editing either one
  // flips it back to "Speak" (the clip no longer represents the inputs).
  let lastText = $state("");
  let lastVoice = $state("");
  const canReplay = $derived(
    lastWav !== null && text.trim() === lastText && voice === lastVoice,
  );

  // Group the voices for the picker: af_/am_ = American, bf_/bm_ = British;
  // f/m in the second slot = female/male. Keeps a 28-entry list scannable.
  const voiceGroups = $derived.by(() => {
    const groups: { label: string; voices: { id: string; label: string }[] }[] = [
      { label: "American · female", voices: [] },
      { label: "American · male", voices: [] },
      { label: "British · female", voices: [] },
      { label: "British · male", voices: [] },
    ];
    for (const v of model?.voices ?? []) {
      const idx =
        v.id.startsWith("af_") ? 0 : v.id.startsWith("am_") ? 1 : v.id.startsWith("bf_") ? 2 : 3;
      groups[idx].voices.push(v);
    }
    return groups.filter((g) => g.voices.length > 0);
  });

  async function load() {
    info = await invokeQuiet<TtsOverview>("tts_overview");
    if (model && !voice) voice = model.defaultVoice ?? model.voices[0]?.id ?? "";
  }

  $effect(() => {
    load();
  });

  function fmtSize(bytes: number): string {
    return `${(bytes / 1_000_000_000).toFixed(1)} GB`.replace("0.4 GB", "~360 MB");
  }

  async function download() {
    if (!model) return;
    error = null;
    downloading = true;
    downloadPct = 0;
    const channel = new Channel<SttDownloadEvent>();
    channel.onmessage = (event) => {
      if (event.kind === "progress") downloadPct = Math.round(event.fraction * 100);
      else if (event.kind === "done") {
        downloading = false;
        if (!event.success && !event.cancelled) error = event.error ?? "Download failed";
        load();
      }
    };
    try {
      await invokeQuiet<void>("tts_download_model", {
        model: model.id,
        // Unique per attempt (the main page's pattern) — a constant id would
        // make cancel target the wrong registry entry on retry.
        downloadId: `tts-${Date.now()}-${Math.random().toString(16).slice(2)}`,
        onEvent: channel,
      });
    } catch (e) {
      downloading = false;
      error = normalise(e).whatHappened;
    }
  }

  async function speak() {
    if (!model || !text.trim()) return;
    error = null;
    speaking = true;
    try {
      const wav = await invokeQuiet<string>("tts_speak", {
        model: model.id,
        text: text.trim(),
        voice: voice || null,
      });
      lastWav = wav;
      lastText = text.trim();
      lastVoice = voice;
      audio?.pause();
      audio = new Audio(`data:audio/wav;base64,${wav}`);
      await audio.play();
    } catch (e) {
      error = normalise(e).whatHappened;
    } finally {
      speaking = false;
    }
  }

  /** Replay the cached clip without re-synthesizing. */
  function replay() {
    if (!lastWav) return;
    if (!audio) audio = new Audio(`data:audio/wav;base64,${lastWav}`);
    audio.currentTime = 0;
    void audio.play();
  }

  /** Export the last synthesized clip as a .wav download. */
  function exportWav() {
    if (!lastWav) return;
    const bytes = Uint8Array.from(atob(lastWav), (c) => c.charCodeAt(0));
    const url = URL.createObjectURL(new Blob([bytes], { type: "audio/wav" }));
    const a = document.createElement("a");
    a.href = url;
    a.download = `portbay-${voice || "voice"}-${Date.now()}.wav`;
    a.click();
    URL.revokeObjectURL(url);
  }
</script>

<section id="tts" class="w-full">
  <div class="rounded-lg border border-border bg-surface p-4">
    <div class="flex flex-wrap items-start justify-between gap-3">
      <div>
        <h2 class="text-[14px] font-semibold text-fg">Text to Speech</h2>
        <p class="mt-1 text-[11px] text-fg-subtle">
          {#if model}
            {model.displayName} · {model.languages} · on-device
          {:else}
            On-device speech synthesis.
          {/if}
        </p>
      </div>
      {#if model && !installed && !downloading}
        <button
          class="rounded-md border border-border px-3 py-1.5 text-[12px] text-fg hover:bg-surface-2"
          onclick={download}
        >
          <Icon name="download" size={12} class="inline mr-1" />
          Download voice model ({fmtSize(model.approxSizeBytes)})
        </button>
      {/if}
    </div>

    {#if !model}
      <p class="mt-3 text-[12px] text-fg-muted">No text-to-speech models in the catalog.</p>
    {:else if downloading}
      <div class="mt-4">
        <div class="h-1.5 w-full overflow-hidden rounded-full bg-surface-2">
          <div class="h-full bg-accent transition-all" style="width: {downloadPct}%"></div>
        </div>
        <p class="mt-1.5 text-[11px] text-fg-subtle">Downloading voice model… {downloadPct}%</p>
      </div>
    {:else if !installed}
      <p class="mt-3 text-[12px] text-fg-muted">
        Download the {model.displayName} model to synthesize speech on this Mac.
      </p>
    {:else}
      <div class="mt-4 space-y-3">
        <label class="block">
          <span class="text-[11px] text-fg-subtle">Voice</span>
          <select
            bind:value={voice}
            class="mt-1 w-full rounded-md border border-border bg-surface-2 px-2 py-1.5 text-[12px] text-fg"
          >
            {#each voiceGroups as g (g.label)}
              <optgroup label={g.label}>
                {#each g.voices as v (v.id)}
                  <option value={v.id}>{v.label}</option>
                {/each}
              </optgroup>
            {/each}
          </select>
          <span class="mt-1 block text-[10.5px] text-fg-subtle">
            {model.voices.length} voices on-device
          </span>
        </label>
        <label class="block">
          <span class="text-[11px] text-fg-subtle">Text</span>
          <textarea
            bind:value={text}
            rows="3"
            placeholder="Type something to hear it spoken…"
            class="mt-1 w-full resize-y rounded-md border border-border bg-surface-2 px-2 py-1.5 text-[12px] text-fg"
          ></textarea>
        </label>
        <div class="flex items-center gap-2">
          <button
            class="rounded-md bg-accent px-3 py-1.5 text-[12px] font-medium text-on-accent hover:opacity-90 disabled:bg-surface-2 disabled:text-fg-subtle disabled:cursor-not-allowed"
            disabled={speaking || !text.trim()}
            onclick={() => (canReplay ? replay() : speak())}
          >
            <Icon name={speaking ? "refresh-cw" : canReplay ? "play" : "audio-lines"} size={12} class="inline mr-1" />
            {speaking ? "Synthesizing…" : canReplay ? "Replay" : "Speak"}
          </button>
          {#if lastWav}
            <button
              class="rounded-md border border-border px-3 py-1.5 text-[12px] text-fg hover:bg-surface-2"
              onclick={exportWav}
            >
              <Icon name="download" size={12} class="inline mr-1" /> Export .wav
            </button>
          {/if}
        </div>
      </div>
    {/if}

    {#if error}
      <p class="mt-3 text-[11px] text-danger">{error}</p>
    {/if}
  </div>
</section>
