<!--
  ImagegenPlayground — generate images on-device with an installed diffusion
  model (FLUX / SD3 via the portbay-imagegen DiffusionKit sidecar). Prompt,
  negative prompt, steps, guidance, size, and seed, with per-step progress
  and a small session gallery. Mirrors the TtsPlayground load→download→use
  shape; generation streams progress over a Channel.
-->
<script lang="ts">
  import { Channel } from "@tauri-apps/api/core";
  import { invokeQuiet, normalise } from "$lib/ipc";
  import { imagegenDownload as sharedDownload } from "$lib/stores/imagegenDownloads.svelte";
  import Icon from "$lib/components/atoms/Icon.svelte";
  import ModelMark from "$lib/components/atoms/ModelMark.svelte";
  import type {
    ImagegenOverview,
    ImageCatalogModel,
    ImagegenGenerateEvent,
    ImagePlaygroundStatus,
    SttDownloadEvent,
  } from "$lib/types/ai";

  interface Props {
    /** Jump to Models → Image generation to download a model. */
    onManageModels: () => void;
  }
  let { onManageModels }: Props = $props();

  // Apple Image Playground is selected in the model dropdown like any other
  // engine, under this synthetic id. Generation runs in-process (no separate
  // window) via the system ImageCreator.
  const APPLE_ID = "apple:image-playground";

  let info = $state<ImagegenOverview | null>(null);
  let modelId = $state<string>("");
  const isApple = $derived(modelId === APPLE_ID);
  const model = $derived<ImageCatalogModel | null>(
    isApple ? null : (info?.catalog.find((m) => m.id === modelId) ?? info?.catalog[0] ?? null),
  );
  const installedIds = $derived(new Set(info?.installed.map((m) => m.id) ?? []));
  const installedModels = $derived(info?.catalog.filter((m) => installedIds.has(m.id)) ?? []);
  const installed = $derived(!!model && installedIds.has(model.id));

  let prompt = $state("");
  let negative = $state("");
  let steps = $state<number | null>(null);
  let guidance = $state<number | null>(null);
  let size = $state<number | null>(null);
  let seed = $state<string>("");

  let generating = $state(false);
  let genFraction = $state(0);
  let genStep = $state(0);
  let genTotal = $state(0);
  // Id of the in-flight generation, so Cancel can kill its sidecar process.
  let generateId: string | null = null;
  let downloading = $state(false);
  let downloadPct = $state(0);
  let error = $state<string | null>(null);
  // Newest-first session gallery of data URLs (not persisted — a test surface).
  let gallery = $state<string[]>([]);
  const current = $derived(gallery[0] ?? null);

  // Apple Image Playground — a system generator (Apple Intelligence). Runs
  // in-process, so prompt + result stay inside PortBay; no model download.
  let appleStatus = $state<ImagePlaygroundStatus | null>(null);
  let appleBusy = $state(false);
  // Generation reported that macOS hasn't downloaded Apple's image model yet —
  // swap the prompt UI for a download affordance (PortBay can't fetch Apple's
  // model itself; the system Image Playground app's first launch does).
  let appleModelMissing = $state(false);

  async function load() {
    info = await invokeQuiet<ImagegenOverview>("imagegen_overview");
    try {
      appleStatus = await invokeQuiet<ImagePlaygroundStatus>("imageplayground_check");
    } catch {
      appleStatus = { available: false };
    }
    // Default selection: an installed on-device model, else Apple (no download),
    // else the first catalog entry.
    if (!modelId && installedModels.length) modelId = installedModels[0].id;
    else if (!modelId && appleStatus?.available) modelId = APPLE_ID;
    else if (!modelId && info?.catalog.length) modelId = info.catalog[0].id;
  }
  $effect(() => {
    load();
  });

  async function generateApple() {
    if (appleBusy) return;
    error = null;
    appleBusy = true;
    try {
      const b64 = await invokeQuiet<string | null>("imageplayground_generate", {
        prompt: prompt.trim() || null,
      });
      // null = generation was cancelled.
      if (b64) gallery = [`data:image/png;base64,${b64}`, ...gallery].slice(0, 12);
    } catch (e) {
      const message = normalise(e).whatHappened;
      // ImageCreator refuses background callers with a raw framework code —
      // translate it for humans.
      if (message.includes("model_not_downloaded")) {
        appleModelMissing = true;
      } else {
        error = message.includes("backgroundCreationForbidden")
          ? "Apple Image Playground needs PortBay to be the active (frontmost) app — click into PortBay and try again."
          : message;
      }
    } finally {
      appleBusy = false;
    }
  }

  async function openImagePlaygroundApp() {
    try {
      await invokeQuiet<void>("imageplayground_open_app");
    } catch (e) {
      error = normalise(e).whatHappened;
    }
  }

  function fmtSize(bytes: number): string {
    return `${(bytes / 1_000_000_000).toFixed(1)} GB`;
  }

  async function download() {
    if (!model) return;
    // Cross-surface guard: the Models tab may already be downloading against
    // the same sidecar — concurrent downloads to it are undefined.
    if (sharedDownload.active) {
      error = "Another image-model download is already running — check the Models tab.";
      return;
    }
    sharedDownload.active = true;
    error = null;
    downloading = true;
    downloadPct = 0;
    const channel = new Channel<SttDownloadEvent>();
    channel.onmessage = (event) => {
      if (event.kind === "progress") downloadPct = Math.round(event.fraction * 100);
      else if (event.kind === "done") {
        downloading = false;
        sharedDownload.active = false;
        if (!event.success && !event.cancelled) error = event.error ?? "Download failed";
        load();
      }
    };
    try {
      await invokeQuiet<void>("imagegen_download_model", {
        model: model.id,
        downloadId: `imagegen-${model.id}-${Date.now()}`,
        onEvent: channel,
      });
    } catch (e) {
      downloading = false;
      sharedDownload.active = false;
      error = normalise(e).whatHappened;
    }
  }

  // A user-initiated cancel surfaces as an error message backend-side —
  // don't render it as a failure.
  function isCancelled(message: string): boolean {
    return message.toLowerCase().includes("cancelled");
  }

  async function generate() {
    if (!model || !prompt.trim()) return;
    error = null;
    generating = true;
    genFraction = 0;
    genStep = 0;
    genTotal = steps ?? model.defaultSteps;
    const id = (generateId = crypto.randomUUID());
    const channel = new Channel<ImagegenGenerateEvent>();
    channel.onmessage = (event) => {
      if (event.kind === "progress") {
        genFraction = event.fraction;
        genStep = event.step;
        genTotal = event.totalSteps;
      } else if (event.kind === "done") {
        gallery = [`data:image/png;base64,${event.imageBase64}`, ...gallery].slice(0, 12);
        generating = false;
      } else if (event.kind === "error") {
        if (!isCancelled(event.message)) error = event.message;
        generating = false;
      }
    };
    try {
      await invokeQuiet<void>("imagegen_generate", {
        model: model.id,
        generateId: id,
        prompt: prompt.trim(),
        negativePrompt: negative.trim() || null,
        steps: steps ?? null,
        guidance: guidance ?? null,
        size: size ?? null,
        seed: seed.trim() ? Number(seed.trim()) : null,
        onEvent: channel,
      });
    } catch (e) {
      const message = normalise(e).whatHappened;
      if (!isCancelled(message)) error = message;
      generating = false;
    } finally {
      if (generateId === id) generateId = null;
    }
  }

  async function cancelGenerate() {
    if (!generateId) return;
    try {
      await invokeQuiet<void>("imagegen_cancel_generate", { generateId });
    } catch {
      // Best-effort: the generation may have just finished.
    }
  }
</script>

<section id="image" class="w-full">
  <div class="grid gap-4 xl:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
    <!-- Controls -->
    <div class="min-w-0 rounded-lg border border-border bg-surface p-4">
      <div class="flex flex-wrap items-center justify-between gap-2">
        <div class="flex items-center gap-2">
          <Icon name="image" size={14} class="text-fg-muted" />
          <h2 class="text-[14px] font-semibold text-fg">Image generation</h2>
        </div>
        {#if (info && info.catalog.length > 0) || appleStatus?.available}
          <label class="flex items-center gap-2 text-[11px] text-fg-subtle">
            Model
            <select class="rounded-md border border-border bg-bg px-2 py-1.5 text-[12px] text-fg" bind:value={modelId} disabled={generating || appleBusy}>
              {#if appleStatus?.available}
                <option value={APPLE_ID}>Apple Image Playground</option>
              {/if}
              {#each info?.catalog ?? [] as m (m.id)}
                <option value={m.id}>{m.displayName}{installedIds.has(m.id) ? "" : " (not installed)"}</option>
              {/each}
            </select>
          </label>
        {/if}
      </div>

      {#if isApple && appleModelMissing}
        <div class="mt-3 space-y-3">
          <div class="flex items-center gap-2">
            <ModelMark family="apple" size={18} class="shrink-0" />
            <p class="text-[12px] text-fg-muted">
              macOS hasn't downloaded <span class="font-medium text-fg">Apple's image model</span> yet. Open Image Playground to start the system download, then try again here.
            </p>
          </div>
          <div class="flex flex-wrap gap-2">
            <button class="rounded-md bg-accent px-3 py-1.5 text-[12px] font-semibold text-on-accent" onclick={openImagePlaygroundApp}>
              <Icon name="download" size={12} class="inline mr-1" />
              Open Image Playground
            </button>
            <button
              class="rounded-md border border-border px-3 py-1.5 text-[12px] text-fg hover:bg-surface-2 disabled:opacity-50"
              disabled={appleBusy}
              onclick={() => {
                appleModelMissing = false;
                if (prompt.trim()) generateApple();
              }}
            >
              Try again
            </button>
          </div>
        </div>
      {:else if isApple}
        <div class="mt-3 space-y-3">
          <div class="flex items-center gap-2">
            <ModelMark family="apple" size={18} class="shrink-0" />
            <p class="text-[11px] text-fg-subtle">Generated on this Mac with Apple Intelligence — no model download.</p>
          </div>
          <label class="block">
            <span class="text-[11px] text-fg-subtle">Prompt</span>
            <textarea
              bind:value={prompt}
              rows="3"
              placeholder="A golden retriever puppy wearing a tiny chef hat…"
              class="mt-1 w-full resize-y rounded-md border border-border bg-bg px-2.5 py-1.5 text-[12px] text-fg focus:outline-none focus:border-accent/60"
            ></textarea>
          </label>
          <div class="flex items-center gap-2">
            <button
              class="inline-flex items-center gap-1.5 rounded-md bg-accent px-3 py-1.5 text-[12px] font-semibold text-on-accent disabled:opacity-50"
              disabled={appleBusy || !prompt.trim()}
              onclick={generateApple}
            >
              <Icon name={appleBusy ? "loader-circle" : "sparkles"} size={13} class={appleBusy ? "animate-spin" : ""} />
              {appleBusy ? "Generating…" : "Generate"}
            </button>
            <span class="text-[10.5px] text-fg-subtle">Apple Intelligence · on-device</span>
          </div>
        </div>
      {:else if !info}
        <p class="mt-3 text-[12px] text-fg-subtle">Checking the image engine…</p>
      {:else if !info.status.available}
        <div class="mt-3 rounded-md border border-border bg-surface-2/40 px-3 py-2.5 text-[12px] leading-relaxed text-fg-muted">
          {info.status.reason === "requires_macos_14"
            ? "On-device image generation needs macOS 14 or newer."
            : info.status.reason === "sidecar_missing"
              ? "On-device image generation isn't available in this build. Reinstalling PortBay should restore it."
              : "Local image generation is macOS-only."}
        </div>
      {:else if info.catalog.length === 0}
        <p class="mt-3 text-[12px] text-fg-muted">No image models in the catalog yet.</p>
      {:else if !installed}
        <div class="mt-3 space-y-3">
          <p class="text-[12px] text-fg-muted">
            Download <span class="font-medium text-fg">{model?.displayName}</span> to generate images entirely on this Mac.
          </p>
          {#if downloading}
            <div>
              <div class="h-1.5 w-full overflow-hidden rounded-full bg-surface-2">
                <div class="h-full bg-accent transition-all" style="width: {downloadPct}%"></div>
              </div>
              <p class="mt-1.5 text-[11px] text-fg-subtle">Downloading model… {downloadPct}%</p>
            </div>
          {:else}
            <div class="flex flex-wrap gap-2">
              <button class="rounded-md bg-accent px-3 py-1.5 text-[12px] font-semibold text-on-accent" onclick={download}>
                <Icon name="download" size={12} class="inline mr-1" />
                Download ({model ? fmtSize(model.approxSizeBytes) : ""})
              </button>
              <button class="rounded-md border border-border px-3 py-1.5 text-[12px] text-fg hover:bg-surface-2" onclick={onManageModels}>
                Manage models
              </button>
            </div>
          {/if}
        </div>
      {:else}
        <div class="mt-3 space-y-3">
          <label class="block">
            <span class="text-[11px] text-fg-subtle">Prompt</span>
            <textarea
              bind:value={prompt}
              rows="3"
              placeholder="A serene harbor at golden hour, boats moored, soft fog…"
              class="mt-1 w-full resize-y rounded-md border border-border bg-bg px-2.5 py-1.5 text-[12px] text-fg focus:outline-none focus:border-accent/60"
            ></textarea>
          </label>
          <label class="block">
            <span class="text-[11px] text-fg-subtle">Negative prompt <span class="text-fg-subtle/70">(optional)</span></span>
            <input
              bind:value={negative}
              placeholder="blurry, watermark, extra fingers…"
              class="mt-1 w-full rounded-md border border-border bg-bg px-2.5 py-1.5 text-[12px] text-fg focus:outline-none focus:border-accent/60"
            />
          </label>

          <div class="grid grid-cols-2 gap-2 sm:grid-cols-4">
            {#each [
              { label: "Steps", value: () => steps, set: (v: number | null) => (steps = v), step: "1", placeholder: String(model?.defaultSteps ?? 4) },
              { label: "Guidance", value: () => guidance, set: (v: number | null) => (guidance = v), step: "0.1", placeholder: "auto" },
              { label: "Size", value: () => size, set: (v: number | null) => (size = v), step: "64", placeholder: String(model?.defaultSize ?? 1024) },
            ] as knob (knob.label)}
              <label class="block">
                <span class="text-[10.5px] uppercase tracking-wide text-fg-subtle">{knob.label}</span>
                <input
                  type="number"
                  step={knob.step}
                  value={knob.value() ?? ""}
                  placeholder={knob.placeholder}
                  oninput={(e) => knob.set(e.currentTarget.value === "" ? null : Number(e.currentTarget.value))}
                  class="mt-1 w-full rounded-md border border-border bg-bg px-2 py-1.5 font-mono text-[12px] text-fg focus:outline-none focus:border-accent/60"
                />
              </label>
            {/each}
            <label class="block">
              <span class="text-[10.5px] uppercase tracking-wide text-fg-subtle">Seed</span>
              <input
                bind:value={seed}
                inputmode="numeric"
                placeholder="random"
                class="mt-1 w-full rounded-md border border-border bg-bg px-2 py-1.5 font-mono text-[12px] text-fg focus:outline-none focus:border-accent/60"
              />
            </label>
          </div>

          <div class="flex items-center gap-2">
            <button
              class="inline-flex items-center gap-1.5 rounded-md bg-accent px-3 py-1.5 text-[12px] font-semibold text-on-accent disabled:opacity-50"
              disabled={generating || !prompt.trim()}
              onclick={generate}
            >
              <Icon name={generating ? "loader-circle" : "sparkles"} size={13} class={generating ? "animate-spin" : ""} />
              {generating ? "Generating…" : "Generate"}
            </button>
            {#if generating}
              <button
                type="button"
                class="rounded-md border border-border px-3 py-1.5 text-[12px] text-fg hover:bg-surface-2"
                onclick={cancelGenerate}
              >
                Cancel
              </button>
            {/if}
            <span class="text-[10.5px] text-fg-subtle">{model?.speedNote}</span>
          </div>
        </div>
      {/if}

      {#if error}
        <p class="mt-3 text-[11px] text-status-unhealthy">{error}</p>
      {/if}
    </div>

    <!-- Canvas + gallery -->
    <div class="min-w-0 rounded-lg border border-border bg-surface p-4">
      <h3 class="text-[13px] font-semibold text-fg">Output</h3>
      <div class="mt-3 grid aspect-square w-full place-items-center overflow-hidden rounded-md border border-border bg-bg">
        {#if generating || appleBusy}
          <div class="w-full px-8 text-center">
            <Icon name="loader-circle" size={24} class="mx-auto animate-spin text-accent" />
            {#if generating}
              <div class="mx-auto mt-3 h-1.5 max-w-[200px] overflow-hidden rounded-full bg-surface-2">
                <div class="h-full bg-accent transition-all" style={`width:${Math.round(genFraction * 100)}%`}></div>
              </div>
              <p class="mt-2 text-[11px] text-fg-subtle">
                Diffusing… {genTotal ? `step ${genStep}/${genTotal}` : ""}
              </p>
            {:else}
              <p class="mt-2 text-[11px] text-fg-subtle">Generating with Apple Intelligence…</p>
            {/if}
          </div>
        {:else if current}
          <img src={current} alt="Generated result" class="h-full w-full object-contain" />
        {:else}
          <div class="px-8 text-center">
            <Icon name="image" size={24} class="mx-auto text-fg-subtle" />
            <p class="mt-2 text-[12px] text-fg-subtle">Generated images appear here.</p>
          </div>
        {/if}
      </div>

      {#if gallery.length > 1}
        <div class="mt-3 flex gap-2 overflow-x-auto">
          {#each gallery as img, i (img)}
            <button
              type="button"
              class="h-14 w-14 shrink-0 overflow-hidden rounded-md border {i === 0 ? 'border-accent' : 'border-border'}"
              onclick={() => (gallery = [img, ...gallery.filter((g) => g !== img)])}
            >
              <img src={img} alt="" class="h-full w-full object-cover" />
            </button>
          {/each}
        </div>
      {/if}

      {#if current}
        <div class="mt-2 flex items-center gap-3">
          <a href={current} download="portbay-image.png" class="text-[11px] text-accent hover:underline">Download PNG</a>
          <span class="text-[10.5px] text-fg-subtle">{gallery.length} this session</span>
        </div>
      {/if}
    </div>
  </div>
</section>
