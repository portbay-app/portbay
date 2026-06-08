<!--
  SmartDictationPanel — the Smart Dictation rewrite-layer controls, hosted on
  the AI page (moved out of Settings → AI Integrations, 2026-06-06).

  No mode picker and no push-to-talk toggle (2026-06-06, user decision):
  the rewrite is always on and scales itself — light cleanup for clean
  speech, restructure for rambling speech — and ⌘Z restores the raw
  transcript. Push-to-talk is simply part of how dictation works.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "$lib/components/atoms/Icon.svelte";
  import FnKey from "$lib/components/atoms/FnKey.svelte";
  import Popover from "$lib/components/atoms/Popover.svelte";
  import DictateAnywhereControls from "$lib/components/ai/DictateAnywhereControls.svelte";
  import { invokeQuiet, safeInvoke } from "$lib/ipc";
  import { openUrl } from "$lib/security/openUrl";
  import { preferences } from "$lib/stores/preferences.svelte";
  import { clearProviderLatch } from "$lib/dictation/rewriter.svelte";
  import type { DictationProviderStatus } from "$lib/dictation/types";
  import type { SttOverview } from "$lib/types/ai";

  /** Host page hook: jump to the AI page's "Speech to text" section (model
   * downloads live there, not here). */
  let { onManageSpeechModels }: { onManageSpeechModels?: () => void } = $props();

  const dict = $derived(preferences.value.dictation);

  /** Transcription engines — the upstream half of dictation (the rewrite
   * provider below is the downstream half). macOS stays the zero-setup
   * default; the local engine swaps the recognizer for a downloaded
   * Whisper/Parakeet model with live partial captions. */
  const STT_ENGINES: { id: string; label: string; detail: string }[] = [
    {
      id: "macos",
      label: "macOS Dictation",
      detail: "Built into macOS — types as you speak, nothing to download.",
    },
    {
      id: "local",
      label: "Local model",
      detail: "Whisper or Parakeet running on this Mac — your choice of model and quality.",
    },
  ];

  /** Local STT inventory (installed models + sidecar availability), probed
   * quietly — an unavailable engine is copy here, never a toast. */
  let sttInfo = $state<SttOverview | null>(null);
  async function refreshSttInfo() {
    try {
      sttInfo = await invokeQuiet<SttOverview>("stt_overview");
    } catch {
      sttInfo = null;
    }
  }
  const sttAvailable = $derived(sttInfo?.status.available === true);
  const sttInstalled = $derived(sttInfo?.installed ?? []);
  const sttModelName = (id: string): string =>
    sttInfo?.catalog.find((m) => m.id === id)?.displayName ?? id;

  /** Page the chosen model in now (fire-and-forget) — picking an engine or
   * model in Settings is the strongest "dictation is imminent" signal, and
   * the capture start otherwise pays the full CoreML load. */
  function prewarmSttModel(model: string) {
    if (!model) return;
    void safeInvoke("stt_prewarm", { model }).catch(() => {});
  }

  async function setSttEngine(engine: string) {
    if (engine === dict.sttEngine) return;
    // Switching to "local" with nothing picked: adopt the recommended (or
    // only) installed model so the switch works immediately; with nothing
    // installed the engine quietly stays macOS until a model is downloaded
    // (the copy below says so).
    let sttModel = dict.sttModel;
    if (engine === "local" && !sttModel) {
      const preferred =
        sttInstalled.find((m) => sttInfo?.catalog.find((c) => c.id === m.id)?.recommended) ??
        sttInstalled[0];
      sttModel = preferred?.id ?? "";
    }
    await preferences.update({ dictation: { ...dict, sttEngine: engine, sttModel } });
    if (engine === "local") prewarmSttModel(sttModel);
  }

  function setSttModel(id: string) {
    void preferences.update({ dictation: { ...dict, sttModel: id } });
    prewarmSttModel(id);
  }

  /** Recording-overlay position (the notch HUD / bottom pill both
   * local-engine surfaces share). Swaps the wrapper on the NEXT session —
   * the native window is placed per show. The waveform noise floor and
   * preview-tail length are no longer user-facing: the waveform
   * self-calibrates to the room, and the preview length keeps its backend
   * default (both prefs stay in the core, just without a knob). */
  const OVERLAY_POSITIONS: { id: string; label: string }[] = [
    { id: "notch", label: "Notch" },
    { id: "bottom", label: "Bottom" },
  ];

  async function setOverlayPosition(position: string) {
    if (position === dict.overlayPosition) return;
    await preferences.update({ dictation: { ...dict, overlayPosition: position } });
  }

  /** Recent system-wide dictations — the never-lose-a-dictation net (the
   * backend records every anywhere session; the tray's "Paste Last
   * Dictation" re-delivers the newest). Copy here is the recovery path for
   * older entries. */
  interface DictationHistoryEntry {
    id: number;
    atMs: number;
    text: string;
    /** Pre-polish transcript, present only when a rewrite changed the text. */
    raw?: string | null;
    appName?: string | null;
    inserted: boolean;
  }
  let history = $state<DictationHistoryEntry[]>([]);
  let copiedId = $state<number | null>(null);
  async function refreshHistory() {
    try {
      history = (await invokeQuiet<DictationHistoryEntry[]>("dictation_history_list")) ?? [];
    } catch {
      history = [];
    }
  }
  async function copyHistoryEntry(entry: DictationHistoryEntry) {
    try {
      await navigator.clipboard.writeText(entry.text);
      copiedId = entry.id;
      setTimeout(() => {
        if (copiedId === entry.id) copiedId = null;
      }, 1500);
    } catch {
      // Clipboard denial is visible enough (no "Copied" flash).
    }
  }
  async function clearHistory() {
    try {
      await safeInvoke("dictation_history_clear");
    } catch {
      return;
    }
    history = [];
  }
  const historyTime = (atMs: number): string => {
    const d = new Date(atMs);
    const sameDay = new Date().toDateString() === d.toDateString();
    return sameDay
      ? d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })
      : d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
  };

  /** Rewrite providers: Apple's on-device model is the zero-setup default;
   * Ollama stays for older Macs and model choice; Off keeps the raw
   * transcript untouched (the rewrite layer does nothing). */
  const DICTATION_PROVIDERS: { id: string; label: string; detail: string }[] = [
    {
      id: "apple",
      label: "Apple Intelligence",
      detail: "On-device, zero setup.",
    },
    {
      id: "ollama",
      label: "Ollama",
      detail: "Local server, your model.",
    },
    {
      id: "off",
      label: "Off",
      detail: "Raw text, no clean-up.",
    },
  ];

  /** Settings copy for the Apple provider's machine-readable reasons. */
  const AFM_REASON_COPY: Record<string, string> = {
    requires_macos_26: "Needs macOS 26 or newer — switch to Ollama, or update macOS.",
    device_not_eligible: "This Mac doesn't support Apple Intelligence — switch to Ollama.",
    apple_intelligence_not_enabled:
      "Turn on Apple Intelligence in System Settings → Apple Intelligence & Siri, then re-check.",
    model_not_ready: "The on-device model is still downloading. Try again in a few minutes.",
    sidecar_missing: "The bundled helper is missing — reinstall PortBay.",
    sidecar_failed: "The bundled helper didn't respond — reinstall PortBay.",
  };
  const afmReasonCopy = (reason: string | null | undefined): string =>
    (reason && AFM_REASON_COPY[reason]) || "Apple Intelligence isn't available right now.";

  let dictStatus = $state<DictationProviderStatus | null>(null);
  let dictChecking = $state(false);

  /** The model the jargon A/B (claudedocs/dictation-jargon-ab-2026-06-06.md)
   * crowned: fewer dropped clauses than the on-device 3B on every audience
   * tested, strictly better vocabulary application. Apple stays the
   * zero-setup default; this is the data-justified upgrade offer. */
  const RECOMMENDED_OLLAMA_MODEL = "qwen2.5:7b";

  /** Scout probe of the local Ollama while Apple is the active provider —
   * powers the "recommended for power users" affordance. Null until probed
   * (or when Apple isn't active; the affordance hides either way). */
  let ollamaScout = $state<DictationProviderStatus | null>(null);

  /** The installed model matching the recommendation, if any. */
  const recommendedInstalled = $derived(
    ollamaScout?.models.find((m) => m.startsWith(RECOMMENDED_OLLAMA_MODEL)) ?? null,
  );

  /** Probe the configured provider: liveness (+ model list for Ollama).
   * Off has nothing to probe — the rewrite layer is disabled. */
  async function checkDictationProvider() {
    if (dict.provider === "off") {
      dictStatus = null;
      ollamaScout = null;
      return;
    }
    dictChecking = true;
    try {
      dictStatus = await safeInvoke<DictationProviderStatus>("dictation_provider_status", {
        provider: { kind: dict.provider, endpoint: dict.endpoint, model: dict.model },
      });
      // Available again (e.g. Apple Intelligence finished downloading): re-arm
      // a provider the rewriter latched off after a structural failure.
      if (dictStatus?.reachable) clearProviderLatch();
    } catch {
      dictStatus = null;
    } finally {
      dictChecking = false;
    }
    // Scout the local Ollama when Apple is active so the upgrade affordance
    // can offer the stronger model. Quiet: a dead/absent Ollama just means
    // no affordance, never an error.
    if (dict.provider === "apple") {
      try {
        ollamaScout = await safeInvoke<DictationProviderStatus>("dictation_provider_status", {
          provider: { kind: "ollama", endpoint: dict.endpoint, model: "" },
        });
      } catch {
        ollamaScout = null;
      }
    }
  }

  /** Take the recommendation: switch to Ollama pinned to the recommended
   * model (auto-pick prefers the smaller 3b when both are installed — the
   * recommendation is specifically the 7b). */
  async function adoptRecommendedModel() {
    if (!recommendedInstalled) return;
    await preferences.update({
      dictation: { ...dict, provider: "ollama", model: recommendedInstalled },
    });
    dictStatus = null;
    void checkDictationProvider();
  }

  function setDictationModel(model: string) {
    void preferences.update({ dictation: { ...dict, model } });
  }

  async function setDictationProvider(provider: string) {
    if (provider === dict.provider) return;
    await preferences.update({ dictation: { ...dict, provider } });
    // A stale status from the other provider is worse than none.
    dictStatus = null;
    void checkDictationProvider();
  }

  /** Parse the comma-separated custom-terms input into a clean list and
   * persist it (same pattern as AdvancedPanel's extra dirs). Order kept —
   * the prompt takes the first 12; the backend re-trims for safety. */
  function saveDictationCustomTerms(raw: string): void {
    const seen = new Set<string>();
    const terms = raw
      .split(",")
      .map((s) => s.trim())
      .filter((s) => s.length > 0 && !seen.has(s.toLowerCase()) && !!seen.add(s.toLowerCase()));
    void preferences.update({ dictation: { ...dict, customTerms: terms } });
  }

  /* Forget everything dictation LEARNED — the per-context/per-project jargon
   * store the rewrite and recognizer build from accepted dictations. The
   * privacy reset; the custom terms above (hand-curated) are a separate list
   * and stay. Brief inline confirmation rather than a toast — it's a quiet,
   * reversible-by-re-learning action. */
  let vocabReset = $state(false);
  async function resetLearnedVocabulary() {
    await safeInvoke("dictation_reset_vocabulary");
    vocabReset = true;
    setTimeout(() => (vocabReset = false), 2000);
  }

  // The endpoint edits through a draft so half-typed URLs aren't persisted
  // (every control on this page otherwise writes through on change).
  let dictEndpointDraft = $state("");
  let dictEndpointDirty = $state(false);
  $effect(() => {
    if (!dictEndpointDirty) dictEndpointDraft = preferences.value.dictation.endpoint;
  });
  async function commitDictationEndpoint() {
    dictEndpointDirty = false;
    const next = dictEndpointDraft.trim().replace(/\/+$/, "") || "http://127.0.0.1:11434";
    if (next !== dict.endpoint) {
      await preferences.update({
        ai: { ...preferences.value.ai, endpoint: next },
        dictation: { ...dict, endpoint: next },
      });
      void checkDictationProvider();
    }
  }

  onMount(() => {
    // Settings load is async; probe the dictation provider once they land so
    // a dead setup is visible without a click (the rewrite is always on). The
    // root layout already loads preferences — only load if they haven't landed
    // yet, so revisiting the panel doesn't re-fetch.
    const ready = preferences.loaded ? Promise.resolve() : preferences.load();
    void ready.then(() => {
      void checkDictationProvider();
      void refreshSttInfo();
      void refreshHistory();
    });
  });
</script>

<div class="rounded-lg border border-border bg-surface p-4">
  <h2 class="text-[14px] font-semibold text-fg">Speech-to-Text</h2>
  <p class="mt-1 text-[12px] text-fg-muted leading-relaxed">
    {#if dict.provider === "off"}
      Dictated speech is typed exactly as spoken — the rewrite layer is off. Hold
      <FnKey /> in a field to talk.
    {:else}
      Dictated speech is polished automatically — light touch-up when it's already
      clean, full restructure when it rambles. ⌘Z restores your exact words. Hold
      <FnKey /> in a field to talk; with text selected, your words become an edit
      instruction.
    {/if}
  </p>

  <div class="mt-4 space-y-4">
    <!-- Privacy posture — always visible so the trade is explicit. -->
    <p class="text-[11px] text-fg-subtle leading-relaxed">
      {#if dict.provider === "off"}
        {#if dict.sttEngine === "local"}
          Audio is captured and transcribed entirely on this Mac by the model you
          chose — nothing leaves the machine. With the rewrite layer off, your words
          are kept exactly as transcribed.
        {:else}
          Audio never leaves macOS dictation, and with the rewrite layer off your
          words are kept exactly as spoken — nothing is sent anywhere.
        {/if}
      {:else if dict.sttEngine === "local"}
        Audio is captured and transcribed entirely on this Mac by the model you
        chose — nothing leaves the machine. Only the resulting <em>text</em> is
        sent to the rewrite model below, and if the rewrite fails for any
        reason, your words stay exactly as transcribed.
      {:else}
        Audio never leaves macOS dictation. Only the dictated
        <em>text</em> is sent to the model below — local by default — and if the
        rewrite fails for any reason, your words stay exactly as spoken.
      {/if}
    </p>

    <!-- Transcription engine — the upstream half: who turns speech into
         text. The rewrite provider below is unchanged either way. -->
    <div>
      <h3 class="text-[12px] font-semibold text-fg">Transcription</h3>
      <div class="mt-2 grid grid-cols-2 gap-2 max-[640px]:grid-cols-1" role="radiogroup" aria-label="Transcription engine">
        {#each STT_ENGINES as engine (engine.id)}
          {@const active = dict.sttEngine === engine.id}
          {@const disabled = engine.id === "local" && sttInfo !== null && !sttAvailable}
          <button
            type="button"
            role="radio"
            aria-checked={active}
            {disabled}
            onclick={() => void setSttEngine(engine.id)}
            class="rounded-lg border p-3 text-left transition-colors disabled:opacity-50 disabled:cursor-not-allowed {active
              ? 'border-accent/60 bg-accent/[0.08]'
              : 'border-border hover:border-border-strong hover:bg-surface-2'}"
          >
            <span class="flex items-center gap-1.5 text-[12.5px] font-medium {active ? 'text-accent' : 'text-fg'}">
              <span class="w-2 h-2 rounded-full {active ? 'bg-accent' : 'bg-fg-muted/40'}"></span>
              {engine.label}
            </span>
            <span class="mt-1 block text-[11px] leading-relaxed text-fg-subtle">{engine.detail}</span>
          </button>
        {/each}
      </div>

      {#if sttInfo !== null && !sttAvailable}
        <p class="mt-2 text-[11px] text-fg-subtle">
          {sttInfo.status.reason === "requires_macos_14"
            ? "Local transcription needs macOS 14 or newer — dictation uses macOS Dictation on this Mac."
            : "The local speech-to-text engine isn't available — dictation uses macOS Dictation."}
        </p>
      {:else if dict.sttEngine === "local"}
        <div class="mt-2 flex items-center justify-between gap-3 rounded-lg border border-border px-3 py-2.5">
          <div class="min-w-0">
            <span class="text-[13px] text-fg">Speech model</span>
            <p class="text-[11px] text-fg-subtle mt-0.5">
              {#if sttInstalled.length === 0}
                No models downloaded yet — dictation stays on macOS Dictation until one is.
              {:else}
                Runs on the Neural Engine; streaming models show live captions while you talk.
              {/if}
            </p>
          </div>
          {#if sttInstalled.length === 0}
            <button
              type="button"
              class="shrink-0 h-8 px-2.5 rounded-md border border-accent/40 text-[12px] text-accent hover:bg-accent/10 transition-colors"
              onclick={() => onManageSpeechModels?.()}
            >
              Download a model
            </button>
          {:else}
            <Popover align="right" width="16rem">
              {#snippet trigger(toggle, open)}
                <button
                  type="button"
                  onclick={toggle}
                  aria-expanded={open}
                  class="h-8 w-56 shrink-0 inline-flex items-center gap-2 rounded-md bg-bg border border-border px-2.5 text-[12px] text-fg hover:border-accent/60 transition-colors"
                >
                  <Icon name="audio-lines" size={13} class="shrink-0 text-fg-subtle" />
                  <span class="truncate {dict.sttModel ? '' : 'text-fg-muted'}">
                    {dict.sttModel ? sttModelName(dict.sttModel) : "Choose a model"}
                  </span>
                  <Icon name="chevron-down" size={13} class="ml-auto shrink-0 text-fg-subtle" />
                </button>
              {/snippet}
              {#snippet children(close)}
                <div class="space-y-0.5 min-w-[15rem]">
                  {#each sttInstalled as m (m.id)}
                    <button
                      type="button"
                      onclick={() => { setSttModel(m.id); close(); }}
                      class="w-full text-left rounded px-2 py-1 text-[12px] hover:bg-surface-2 {dict.sttModel === m.id ? 'text-fg font-medium' : 'text-fg-muted'}"
                    >
                      {sttModelName(m.id)}
                    </button>
                  {/each}
                  <button
                    type="button"
                    onclick={() => { onManageSpeechModels?.(); close(); }}
                    class="w-full text-left rounded px-2 py-1 text-[12px] text-accent hover:bg-surface-2"
                  >
                    Manage models…
                  </button>
                </div>
              {/snippet}
            </Popover>
          {/if}
        </div>

        <!-- Dictate anywhere — system-wide push-to-talk: hold Fn in ANY app
             and the transcript is typed into it. The same controls (bound to
             the same preference) also live in Settings → General; this shared
             component owns the probing + Accessibility grant flow. -->
        <div class="mt-2">
          <DictateAnywhereControls onManageModels={onManageSpeechModels} />
        </div>

        <!-- Recording overlay — the HUD both local-engine surfaces share
             (in-app sessions and dictate-anywhere). Position only: the
             waveform self-calibrates to the room and the preview length
             keeps its backend default (no knobs to tune). -->
        <div class="mt-2 rounded-lg border border-border px-3">
          <div class="flex items-center justify-between gap-3 py-2.5">
            <div class="min-w-0">
              <span class="text-[13px] text-fg">Overlay position</span>
              <p class="text-[11px] text-fg-subtle mt-0.5">
                Where the recording overlay appears — out of the camera notch, or as a
                pill near the bottom of the screen (better on Macs without a notch).
              </p>
            </div>
            <div
              class="inline-flex shrink-0 rounded-md border border-border overflow-hidden"
              role="radiogroup"
              aria-label="Overlay position"
            >
              {#each OVERLAY_POSITIONS as p (p.id)}
                {@const active = dict.overlayPosition === p.id}
                <button
                  type="button"
                  role="radio"
                  aria-checked={active}
                  onclick={() => void setOverlayPosition(p.id)}
                  class="h-8 px-3 text-[12px] transition-colors {active
                    ? 'bg-accent/15 text-accent'
                    : 'text-fg-muted hover:text-fg hover:bg-surface-2'}"
                >
                  {p.label}
                </button>
              {/each}
            </div>
          </div>
        </div>

      {/if}
    </div>

    <!-- Provider — card pattern like every control here. Same heading +
         mt-2 structure as Transcription above (a negative margin here once
         pulled the cards up over the heading). -->
    <div>
      <h3 class="text-[12px] font-semibold text-fg">Rewrite model</h3>
      <div class="mt-2 grid grid-cols-3 gap-2 max-[480px]:grid-cols-1" role="radiogroup" aria-label="Rewrite provider">
        {#each DICTATION_PROVIDERS as p (p.id)}
          {@const active = dict.provider === p.id}
          <button
            type="button"
            role="radio"
            aria-checked={active}
            onclick={() => void setDictationProvider(p.id)}
            class="rounded-lg border px-2.5 py-2 text-left transition-colors {active
              ? 'border-accent/60 bg-accent/[0.08]'
              : 'border-border hover:border-border-strong hover:bg-surface-2'}"
          >
            <span class="flex items-center gap-1.5 text-[12px] font-medium {active ? 'text-accent' : 'text-fg'}">
              <span class="w-1.5 h-1.5 shrink-0 rounded-full {active ? 'bg-accent' : 'bg-fg-muted/40'}"></span>
              <span class="truncate">{p.label}</span>
            </span>
            <span class="mt-0.5 block text-[10.5px] leading-snug text-fg-subtle">{p.detail}</span>
          </button>
        {/each}
      </div>
    </div>

    <!-- Recommended-model affordance: a local Ollama was detected while
         Apple is active. Apple stays the zero-setup default; the offer is
         data-backed (jargon A/B 2026-06-06: the 7B drops fewer clauses
         and applies vocabulary strictly better than the on-device 3B). -->
    {#if dict.provider === "apple" && ollamaScout?.reachable}
      <div class="flex items-start gap-2.5 rounded-lg border border-accent/30 bg-accent/[0.06] px-3 py-2.5">
        <Icon name="sparkles" size={13} class="shrink-0 mt-0.5 text-accent" />
        <div class="min-w-0 flex-1">
          {#if recommendedInstalled}
            <p class="text-[12px] text-fg">
              Ollama is running with <span class="font-mono">{recommendedInstalled}</span> — recommended for power users.
            </p>
            <p class="text-[11px] text-fg-subtle mt-0.5">
              Keeps more detail in dense, correction-heavy speech than the built-in model. Same privacy: local only.
            </p>
          {:else}
            <p class="text-[12px] text-fg">
              Ollama detected — power users get better rewrites from <span class="font-mono">{RECOMMENDED_OLLAMA_MODEL}</span>.
            </p>
            <p class="text-[11px] text-fg-subtle mt-0.5">
              Run <code class="font-mono text-fg-muted">ollama pull {RECOMMENDED_OLLAMA_MODEL}</code> (one-time, ~4.7 GB), then re-check.
            </p>
          {/if}
        </div>
        {#if recommendedInstalled}
          <button
            type="button"
            onclick={() => void adoptRecommendedModel()}
            class="shrink-0 h-7 px-2.5 rounded-md border border-accent/40 text-[12px] text-accent hover:bg-accent/10 transition-colors"
          >
            Use it
          </button>
        {/if}
      </div>
    {/if}

    {#if dict.provider === "off"}
      <p class="rounded-lg border border-border bg-surface-2/40 px-3 py-2.5 text-[11px] text-fg-subtle leading-relaxed">
        The rewrite layer is off — dictation types exactly what you say. Pick
        Apple Intelligence or Ollama above to turn on automatic clean-up,
        custom terms, and voice editing.
      </p>
    {:else}
    <div class="divide-y divide-border/60 rounded-lg border border-border px-3">
      <!-- Provider status -->
      <div class="flex items-center justify-between gap-3 py-2.5">
        <div class="min-w-0">
          <span class="text-[13px] text-fg">
            {dict.provider === "apple" ? "Apple Intelligence" : "Ollama (local)"}
          </span>
          <p class="text-[11px] text-fg-subtle mt-0.5">
            {#if dictChecking}
              Checking…
            {:else if dictStatus?.reachable}
              {#if dict.provider === "apple"}
                Ready — rewrites run on this Mac; the transcript never leaves it.
              {:else}
                Running — {dictStatus.models.length} model{dictStatus.models.length === 1 ? "" : "s"} installed.
              {/if}
            {:else if dict.provider === "apple"}
              {afmReasonCopy(dictStatus?.reason)}
              Until then, dictation keeps your words as spoken.
            {:else}
              Not reachable. Install from
              <button type="button" class="text-accent hover:underline" onclick={() => openUrl("https://ollama.com")}>ollama.com</button>,
              or start the managed server from the Server home section.
              Until then, dictation keeps your words as spoken.
            {/if}
          </p>
        </div>
        <span class="inline-flex items-center gap-2 shrink-0">
          <span class="w-2 h-2 rounded-full {dictChecking ? 'bg-fg-muted/40' : dictStatus?.reachable ? 'bg-status-running' : 'bg-amber-400'}"></span>
          <button
            type="button"
            onclick={() => void checkDictationProvider()}
            disabled={dictChecking}
            class="inline-flex items-center gap-1.5 h-8 px-2.5 rounded-md border border-border text-[12px] text-fg-muted hover:text-fg hover:bg-surface-2 disabled:opacity-50 transition-colors"
          >
            <Icon name="refresh-cw" size={13} class={dictChecking ? "animate-spin" : ""} /> Check
          </button>
        </span>
      </div>

      {#if dict.provider === "ollama"}
      <!-- Model -->
      <div class="flex items-center justify-between gap-3 py-2.5">
        <div class="min-w-0">
          <span class="text-[13px] text-fg">Model</span>
          <p class="text-[11px] text-fg-subtle mt-0.5">
            Auto picks the smallest capable installed model — rewriting wants latency, not depth.
          </p>
        </div>
        <Popover align="right" width="14rem">
          {#snippet trigger(toggle, open)}
            <button
              type="button"
              onclick={toggle}
              aria-expanded={open}
              class="h-8 w-56 shrink-0 inline-flex items-center gap-2 rounded-md bg-bg border border-border px-2.5 text-[12px] text-fg hover:border-accent/60 transition-colors"
            >
              <Icon name="sparkles" size={13} class="shrink-0 text-fg-subtle" />
              <span class="truncate {dict.model ? '' : 'text-fg-muted'}">
                {dict.model || `Auto${dictStatus?.defaultModel ? ` (${dictStatus.defaultModel})` : ""}`}
              </span>
              <Icon name="chevron-down" size={13} class="ml-auto shrink-0 text-fg-subtle" />
            </button>
          {/snippet}
          {#snippet children(close)}
            <div class="space-y-0.5 min-w-[13rem]">
              <button
                type="button"
                onclick={() => { setDictationModel(""); close(); }}
                class="w-full text-left rounded px-2 py-1 text-[12px] flex items-center gap-2 hover:bg-surface-2 {!dict.model ? 'text-fg font-medium' : 'text-fg-muted'}"
              >
                <Icon name="sparkles" size={13} class="shrink-0 text-fg-subtle" />
                Auto{dictStatus?.defaultModel ? ` (${dictStatus.defaultModel})` : ""}
              </button>
              {#each dictStatus?.models ?? [] as m (m)}
                <button
                  type="button"
                  onclick={() => { setDictationModel(m); close(); }}
                  class="w-full text-left rounded px-2 py-1 text-[12px] font-mono hover:bg-surface-2 {dict.model === m ? 'text-fg font-medium' : 'text-fg-muted'}"
                >
                  {m}
                </button>
              {:else}
                <p class="px-2 py-1 text-[11px] text-fg-subtle">No models detected — pull one, then re-check.</p>
              {/each}
            </div>
          {/snippet}
        </Popover>
      </div>

      <!-- Endpoint -->
      <div class="flex items-center justify-between gap-3 py-2.5">
        <div class="min-w-0">
          <span class="text-[13px] text-fg">Endpoint</span>
          <p class="text-[11px] text-fg-subtle mt-0.5">
            Shared with the Ollama server configuration on this page.
          </p>
        </div>
        <input
          type="text"
          bind:value={dictEndpointDraft}
          oninput={() => (dictEndpointDirty = true)}
          onblur={() => void commitDictationEndpoint()}
          onkeydown={(e) => { if (e.key === "Enter") (e.currentTarget as HTMLInputElement).blur(); }}
          spellcheck="false"
          aria-label="Ollama endpoint URL"
          class="h-8 w-48 shrink-0 rounded-md bg-bg border border-border px-2.5 text-[12px] font-mono text-fg focus:outline-none focus:border-accent/60 transition-colors"
        />
      </div>
      {/if}

      <!-- Custom terms — both providers; the one lever for plain words
           and niche brands dictation garbles ("factor" → refactor,
           "shop if I" → Shopify) that no automatic source can supply. -->
      <div class="flex items-center justify-between gap-3 py-2.5">
        <div class="min-w-0">
          <span class="text-[13px] text-fg">Custom terms</span>
          <p class="text-[11px] text-fg-subtle mt-0.5">
            Words dictation gets wrong — names, brands, jargon. Comma-separated;
            the first 12 are used, only when something like them was spoken.
          </p>
        </div>
        <input
          type="text"
          value={dict.customTerms.join(", ")}
          onchange={(e) => saveDictationCustomTerms((e.currentTarget as HTMLInputElement).value)}
          onkeydown={(e) => { if (e.key === "Enter") (e.currentTarget as HTMLInputElement).blur(); }}
          placeholder="refactor, Tailwind, Shopify"
          spellcheck="false"
          aria-label="Custom dictation terms"
          class="h-8 w-56 shrink-0 rounded-md bg-bg border border-border px-2.5 text-[12px] text-fg focus:outline-none focus:border-accent/60 transition-colors"
        />
      </div>

      <!-- Reset learned vocabulary — the privacy control for the jargon the
           rewrite + recognizer picked up from accepted dictations. Distinct
           from the hand-curated custom terms above, which it leaves alone. -->
      <div class="flex items-center justify-between gap-3 py-2.5">
        <div class="min-w-0">
          <span class="text-[13px] text-fg">Learned vocabulary</span>
          <p class="text-[11px] text-fg-subtle mt-0.5">
            Jargon dictation picked up from your accepted rewrites. Resetting
            clears it everywhere; your custom terms above stay.
          </p>
        </div>
        <button
          type="button"
          class="h-8 px-3 shrink-0 rounded-md border border-border text-[12px] text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
          onclick={() => void resetLearnedVocabulary()}
        >
          {vocabReset ? "Reset ✓" : "Reset"}
        </button>
      </div>
    </div>
    {/if}

    <!-- Recent dictations — its own section at the end, after the transcription
         and rewrite settings: this is history/recovery, not a setting. The
         recovery net for anywhere sessions — a paste the target app ate (or
         that landed in the wrong window) is one Copy away instead of gone. The
         tray menu's "Paste Last Dictation" re-delivers the newest directly. -->
    {#if history.length > 0}
      <div>
        <div class="flex items-center justify-between gap-3">
          <h3 class="text-[12px] font-semibold text-fg">Recent dictations</h3>
          <button
            type="button"
            class="h-7 px-2.5 rounded-md border border-border text-[11px] text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors shrink-0"
            onclick={() => void clearHistory()}
          >
            Clear
          </button>
        </div>
        <p class="text-[11px] text-fg-subtle mt-1">
          The last {history.length === 1 ? "dictation" : `${history.length} dictations`} from
          other apps, kept on this Mac in case a paste goes missing.
        </p>
        <ul class="mt-2 rounded-lg border border-border divide-y divide-border/40 max-h-56 overflow-y-auto">
          {#each history as entry (entry.id)}
            <li class="flex items-center gap-3 px-3 py-2">
              <div class="min-w-0 flex-1">
                <p class="text-[12px] text-fg truncate" title={entry.text}>{entry.text}</p>
                <p class="text-[10.5px] text-fg-subtle mt-0.5">
                  {historyTime(entry.atMs)}{entry.appName ? ` · ${entry.appName}` : ""}{entry.inserted
                    ? ""
                    : " · paste failed"}
                </p>
              </div>
              <button
                type="button"
                class="h-7 px-2 rounded-md border border-border text-[11px] transition-colors shrink-0 {copiedId ===
                entry.id
                  ? 'text-accent border-accent/40'
                  : 'text-fg-muted hover:text-fg hover:bg-surface-2'}"
                onclick={() => void copyHistoryEntry(entry)}
              >
                {copiedId === entry.id ? "Copied" : "Copy"}
              </button>
            </li>
          {/each}
        </ul>
      </div>
    {/if}
  </div>
</div>
