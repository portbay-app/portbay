<!--
  /integrations — the hub for PortBay's optional capabilities.

  A card per tool. Each card is a doorway (Open → the tool's existing surface)
  plus, where the backend has a real switch, the tool's master on/off — the
  same preferences the Settings/AI panels bind, so the two surfaces can never
  disagree:
    · Dictation → dictation.anywhere         (system-wide Fn, arm flow mirrored
                                              from DictateAnywhereControls)
    · STT       → dictation.sttEngine        (local engine ↔ macOS dictation,
                                              adopt-a-model logic mirrored from
                                              SmartDictationPanel)
  TTS and Image Generation have no on/off in the backend — they are on-demand
  engines that only run while used — so their cards show real readiness
  (installed model counts from the engine overviews) instead of a fake switch.

  Sidebar arrangement lives in Settings → General (SidebarCustomizer); the
  cards keep a quick per-tool "Show in sidebar" pin.
-->
<script lang="ts">
  import { onMount } from "svelte";

  import Icon, { type IconName } from "$lib/components/atoms/Icon.svelte";
  import Toggle from "$lib/components/atoms/Toggle.svelte";
  import { invokeQuiet, safeInvoke } from "$lib/ipc";
  import { navOrder } from "$lib/stores/navOrder.svelte";
  import { preferences } from "$lib/stores/preferences.svelte";
  import type { DictationAnywhereStatus } from "$lib/dictation/types";
  import type { ImagegenOverview, SttOverview, TtsOverview } from "$lib/types/ai";

  interface Capability {
    id: string;
    icon: IconName;
    name: string;
    blurb: string;
    href: string;
    /** Nav item this card pins/unpins; absent = lives under the AI entry. */
    navId?: string;
  }

  // Blurbs for the AI modalities mirror the AI page's playground tab blurbs
  // so the same feature reads the same everywhere.
  const CAPABILITIES: Capability[] = [
    {
      id: "dictation",
      icon: "wand-sparkles",
      name: "Dictation",
      href: "/ai?view=dictation",
      blurb: "System-wide voice typing: hold Fn in any app, on-device transcription, AI polish.",
    },
    {
      id: "stt",
      icon: "mic",
      name: "Speech to Text",
      href: "/ai?playground=stt",
      blurb: "On-device transcription with Whisper or Parakeet — the engine behind dictation.",
    },
    {
      id: "tts",
      icon: "audio-lines",
      name: "Text to Speech",
      href: "/ai?playground=tts",
      blurb: "Synthesize natural speech on-device and play it back.",
    },
    {
      id: "image",
      icon: "image",
      name: "Image Generation",
      href: "/ai?playground=image",
      blurb: "Generate images on-device from a text prompt with a diffusion model.",
    },
    {
      id: "inspector",
      icon: "activity",
      name: "Request Inspector",
      href: "/inspector",
      navId: "inspector",
      blurb: "Live HTTP traffic through Caddy — method, status, latency, and the matched project.",
    },
    {
      id: "sandbox",
      icon: "package",
      name: "Sandbox",
      href: "/sandbox",
      navId: "sandbox",
      blurb: "Run projects under macOS Seatbelt isolation and see exactly what the sandbox blocked.",
    },
  ];

  const pinState = $derived(new Map(navOrder.allItems.map((it) => [it.id, it])));

  function isPinned(navId: string): boolean {
    return !(pinState.get(navId)?.hidden ?? false);
  }

  const dict = $derived(preferences.value.dictation);

  // ── Engine readiness (quiet probes — a missing sidecar just leaves the
  //    status line generic; nothing toasts on this page). ──────────────────
  let sttOv = $state<SttOverview | null>(null);
  let ttsOv = $state<TtsOverview | null>(null);
  let imgOv = $state<ImagegenOverview | null>(null);

  onMount(() => {
    void (async () => {
      try {
        sttOv = await invokeQuiet<SttOverview>("stt_overview");
      } catch {
        sttOv = null;
      }
      try {
        ttsOv = await invokeQuiet<TtsOverview>("tts_overview");
      } catch {
        ttsOv = null;
      }
      try {
        imgOv = await invokeQuiet<ImagegenOverview>("imagegen_overview");
      } catch {
        imgOv = null;
      }
    })();
  });

  function modelLine(
    ov: { status: { available: boolean }; installed: unknown[] } | null,
    noun: string,
  ): string | null {
    if (!ov) return null;
    if (!ov.status.available) return "Engine unavailable";
    const n = ov.installed.length;
    return n === 0 ? `No ${noun} downloaded yet` : `${n} ${noun}${n === 1 ? "" : "s"} installed`;
  }

  // ── Dictation (system-wide) — mirrors DictateAnywhereControls.setAnywhere:
  //    flipping on is the approval moment, so arm with prompt:true to fire
  //    macOS's own Accessibility dialog; if trust is still missing, point at
  //    the AI page where the full drag-to-grant flow lives. ────────────────
  const anywhereReady = $derived(dict.sttEngine === "local" && !!dict.sttModel);
  let anywhereHint = $state<string | null>(null);

  async function setAnywhere(next: boolean) {
    anywhereHint = null;
    await preferences.update({ dictation: { ...dict, anywhere: next } });
    if (!next) return;
    try {
      const status = await invokeQuiet<DictationAnywhereStatus>("dictation_anywhere_arm", {
        prompt: true,
      });
      if (status.supported && !status.trusted) {
        anywhereHint = "macOS Accessibility permission needed — finish setup in the AI page.";
      }
    } catch {
      // Probe failure isn't fatal; the AI page owns the full setup flow.
    }
  }

  // ── Local STT engine — mirrors SmartDictationPanel.setSttEngine, incl.
  //    adopting the recommended installed model and the prewarm. ───────────
  const localSttOn = $derived(dict.sttEngine === "local");
  let sttHint = $state<string | null>(null);

  async function setLocalStt(next: boolean) {
    sttHint = null;
    if (!next) {
      await preferences.update({ dictation: { ...dict, sttEngine: "macos" } });
      return;
    }
    let sttModel = dict.sttModel;
    if (!sttModel) {
      const installed = sttOv?.installed ?? [];
      const preferred =
        installed.find((m) => sttOv?.catalog.find((c) => c.id === m.id)?.recommended) ??
        installed[0];
      sttModel = preferred?.id ?? "";
    }
    if (!sttModel) {
      sttHint = "Download a speech model in the AI page first.";
      return;
    }
    await preferences.update({ dictation: { ...dict, sttEngine: "local", sttModel } });
    void safeInvoke("stt_prewarm", { model: sttModel }).catch(() => {});
  }
</script>

<div class="flex flex-col h-full min-h-0">
  <header class="px-6 pt-6 pb-3 shrink-0">
    <h1 class="text-lg font-semibold text-fg flex items-center gap-2">
      <Icon name="grid-2x2" size={18} />
      Integrations
    </h1>
    <p class="text-[12.5px] text-fg-subtle mt-0.5">
      PortBay's optional tools and capabilities — turn them on, open them, pin
      the ones you use to the sidebar.
    </p>
  </header>

  <div class="flex-1 min-h-0 overflow-y-auto px-6 pb-6 space-y-4">
    <div class="grid gap-3 md:grid-cols-2 2xl:grid-cols-3">
      {#each CAPABILITIES as cap (cap.id)}
        <div class="rounded-lg border border-border bg-surface p-4 flex flex-col gap-3">
          <div class="flex items-start gap-3">
            <span
              class="shrink-0 h-9 w-9 rounded-md bg-surface-2 text-fg-muted
                     flex items-center justify-center"
            >
              <Icon name={cap.icon} size={17} />
            </span>
            <div class="min-w-0 flex-1">
              <h3 class="text-[13px] font-medium text-fg leading-tight">
                {cap.name}
              </h3>
              <p class="text-[11.5px] text-fg-subtle mt-1 leading-snug">
                {cap.blurb}
              </p>
            </div>
            <!-- Master switch, where a real one exists. -->
            {#if cap.id === "dictation"}
              <Toggle
                checked={dict.anywhere}
                disabled={!anywhereReady}
                label="Enable system-wide dictation"
                onchange={(next) => void setAnywhere(next)}
              />
            {:else if cap.id === "stt"}
              <Toggle
                checked={localSttOn}
                label="Use the local speech engine"
                onchange={(next) => void setLocalStt(next)}
              />
            {/if}
          </div>

          <!-- Status / hint line — only real, store-backed state. -->
          {#if cap.id === "dictation"}
            {#if anywhereHint}
              <p class="text-[11px] text-status-warning leading-snug">
                {anywhereHint}
                <a href="/ai?view=dictation" class="text-accent hover:underline">Open setup</a>
              </p>
            {:else if !anywhereReady}
              <p class="text-[11px] text-fg-subtle leading-snug">
                Needs the local speech engine and a downloaded model —
                <a href="/ai?view=dictation" class="text-accent hover:underline">set up in AI</a>.
              </p>
            {:else if dict.anywhere}
              <p class="text-[11px] text-fg-subtle leading-snug">
                On — hold Fn in any app to dictate.
              </p>
            {/if}
          {:else if cap.id === "stt"}
            {#if sttHint}
              <p class="text-[11px] text-status-warning leading-snug">
                {sttHint}
                <a href="/ai?playground=stt" class="text-accent hover:underline">Open AI</a>
              </p>
            {:else if modelLine(sttOv, "model")}
              <p class="text-[11px] text-fg-subtle leading-snug">
                {modelLine(sttOv, "model")}
                {#if !localSttOn}
                  · off = macOS system dictation
                {/if}
              </p>
            {/if}
          {:else if cap.id === "tts"}
            {#if modelLine(ttsOv, "voice pack")}
              <p class="text-[11px] text-fg-subtle leading-snug">
                {modelLine(ttsOv, "voice pack")} · runs on demand
              </p>
            {/if}
          {:else if cap.id === "image"}
            {#if modelLine(imgOv, "model")}
              <p class="text-[11px] text-fg-subtle leading-snug">
                {modelLine(imgOv, "model")} · runs on demand
              </p>
            {/if}
          {/if}

          <div class="mt-auto flex items-center justify-between gap-2">
            <a
              href={cap.href}
              class="inline-flex items-center gap-1.5 px-2.5 py-1.5 rounded-md
                     text-[12px] text-fg-muted hover:text-fg bg-surface-2/60
                     hover:bg-surface-2 border border-border/60 transition-colors"
            >
              Open
              <Icon name="arrow-right" size={12} />
            </a>
            {#if cap.navId}
              <label class="flex items-center gap-2 text-[11.5px] text-fg-subtle">
                Show in sidebar
                <Toggle
                  checked={isPinned(cap.navId)}
                  label="Show {cap.name} in sidebar"
                  onchange={(next) => navOrder.setHidden(cap.navId!, !next)}
                />
              </label>
            {/if}
          </div>
        </div>
      {/each}
    </div>

    <p class="text-[11.5px] text-fg-subtle">
      Want to rearrange or hide more of the sidebar? The full manager lives in
      <a href="/settings?tab=general" class="text-accent hover:underline"
        >Settings → General → Sidebar</a
      >.
    </p>
  </div>
</div>
