<!--
  SttPlayground — record from the mic and transcribe on-device with an
  installed Whisper/Parakeet model. Reuses the SAME capture flow the global
  dictation feature uses (`stt_start_capture` / `stt_stop_capture` + the
  `stt://partial` / `stt://level` events emitted by the sidecar), so there's
  no new backend op — just a focused test surface for the transcription model.
-->
<script lang="ts">
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { invokeQuiet, safeInvoke, normalise } from "$lib/ipc";
  import Icon from "$lib/components/atoms/Icon.svelte";
  import ModelMark from "$lib/components/atoms/ModelMark.svelte";
  import { MacPermissionDialog } from "$lib/components/permissions";
  import type { SttOverview } from "$lib/types/ai";

  let info = $state<SttOverview | null>(null);
  let model = $state<string>("");
  // Installed STT models, matched to catalog entries for display.
  const installed = $derived(
    (info?.installed ?? []).map((m) => ({
      id: m.id,
      engine: m.engine,
      label: info?.catalog.find((c) => c.id === m.id)?.displayName ?? m.id,
    })),
  );

  let recording = $state(false);
  let listening = $state(false);
  let partial = $state("");
  let final = $state("");
  let level = $state(0);
  let error = $state<string | null>(null);
  let copied = $state(false);

  // Mic permission pre-flight: surface the TCC state BEFORE the first
  // capture instead of failing mid-session. "not_determined" gets a prompt
  // CTA; denied/restricted gets the System Settings flow.
  const micPermission = $derived(info?.micPermission ?? "unknown");
  const micDenied = $derived(micPermission === "denied" || micPermission === "restricted");
  let micRequesting = $state(false);
  let showMicDialog = $state(false);

  async function requestMic() {
    micRequesting = true;
    try {
      const granted = await invokeQuiet<boolean>("stt_request_mic_access");
      if (!granted) showMicDialog = true;
      await load();
    } finally {
      micRequesting = false;
    }
  }

  /** A capture failure that is really the mic permission — route it to the
   * actionable card rather than dead-end red text. */
  function isMicDeniedError(message: string): boolean {
    return /microphone/i.test(message) && /denied|access/i.test(message);
  }

  async function load() {
    info = await invokeQuiet<SttOverview>("stt_overview");
    if (!model && installed.length) model = installed[0].id;
  }

  $effect(() => {
    load();
    // The listen() calls resolve asynchronously — if this effect is cleaned
    // up before they land (fast tab switches), the late-resolving listeners
    // must be unsubscribed immediately or they leak for the app session.
    let cancelled = false;
    let unlisten: UnlistenFn[] = [];
    void Promise.all([
      listen("dictation://listening", () => {
        listening = true;
      }),
      listen<{ text: string }>("stt://partial", (e) => {
        partial = e.payload.text ?? "";
      }),
      listen<{ rms: number }>("stt://level", (e) => {
        level = Math.min(1, (e.payload.rms ?? 0) * 4);
      }),
      listen("dictation://ended", () => {
        listening = false;
        level = 0;
      }),
    ]).then((fns) => {
      if (cancelled) for (const u of fns) u();
      else unlisten = fns;
    });
    return () => {
      cancelled = true;
      for (const u of unlisten) u();
      unlisten = [];
      if (recording) void safeInvoke("stt_cancel_capture").catch(() => {});
    };
  });

  async function start() {
    if (!model) return;
    error = null;
    partial = "";
    final = "";
    recording = true;
    listening = false;
    try {
      await invokeQuiet<void>("stt_start_capture", { model });
    } catch (e) {
      recording = false;
      error = normalise(e).whatHappened;
    }
  }

  async function stop() {
    if (!recording) return;
    try {
      const text = await invokeQuiet<string>("stt_stop_capture");
      final = (text ?? "").trim();
      partial = "";
    } catch (e) {
      error = normalise(e).whatHappened;
    } finally {
      recording = false;
      listening = false;
      level = 0;
    }
  }

  function copy() {
    if (!final) return;
    void navigator.clipboard.writeText(final);
    copied = true;
    setTimeout(() => (copied = false), 1500);
  }
</script>

<section id="stt" class="w-full">
  <div class="rounded-lg border border-border bg-surface p-4">
    <div class="flex flex-wrap items-center justify-between gap-2">
      <div class="flex items-center gap-2">
        <Icon name="mic" size={14} class="text-fg-muted" />
        <h2 class="text-[14px] font-semibold text-fg">Speech to Text</h2>
      </div>
      <label class="flex items-center gap-2 text-[11px] text-fg-subtle">
        Model
        <select class="rounded-md border border-border bg-bg px-2 py-1.5 text-[12px] text-fg" bind:value={model} disabled={recording}>
          {#if installed.length === 0}
            <option value="">No models installed</option>
          {/if}
          {#each installed as m (m.id)}
            <option value={m.id}>{m.label}</option>
          {/each}
        </select>
      </label>
    </div>

    {#if installed.length === 0}
      <p class="mt-3 text-[12px] text-fg-muted">
        No transcription models installed. Download a Whisper or Parakeet model under
        <span class="font-medium text-fg">Models → Speech-to-Text</span> to test it here.
      </p>
    {:else}
      <p class="mt-1 text-[11px] text-fg-subtle">
        Records from the microphone and transcribes entirely on this Mac — audio never leaves the machine.
      </p>

      {#if micDenied}
        <div class="mt-3 flex flex-wrap items-center justify-between gap-3 rounded-md border border-status-unhealthy/40 bg-status-unhealthy/5 px-3 py-2.5">
          <div class="min-w-0">
            <p class="text-[12px] font-medium text-fg">Microphone access is off for PortBay</p>
            <p class="mt-0.5 text-[11px] text-fg-muted">
              Switch PortBay on under Privacy &amp; Security › Microphone to record here and to dictate anywhere.
            </p>
          </div>
          <button
            type="button"
            class="shrink-0 rounded-md border border-border px-2.5 py-1.5 text-[11px] text-fg hover:bg-surface-2"
            onclick={() => (showMicDialog = true)}
          >
            Open Privacy Settings…
          </button>
        </div>
      {:else if micPermission === "not_determined"}
        <div class="mt-3 flex flex-wrap items-center justify-between gap-3 rounded-md border border-border bg-surface-2/40 px-3 py-2.5">
          <p class="min-w-0 text-[11px] text-fg-muted">
            macOS will ask for microphone access on first use — grant it now so your first dictation isn't interrupted.
          </p>
          <button
            type="button"
            class="shrink-0 rounded-md border border-accent/40 px-2.5 py-1.5 text-[11px] text-accent hover:bg-accent/10 disabled:opacity-50"
            disabled={micRequesting}
            onclick={requestMic}
          >
            {micRequesting ? "Waiting for macOS…" : "Enable microphone"}
          </button>
        </div>
      {/if}

      <div class="mt-3 flex items-center gap-3">
        {#if recording}
          <button
            class="inline-flex items-center gap-1.5 rounded-md border border-status-unhealthy/50 px-3 py-1.5 text-[12px] font-semibold text-status-unhealthy hover:bg-status-unhealthy/10"
            onclick={stop}
          >
            <Icon name="square" size={13} /> Stop & transcribe
          </button>
          <span class="inline-flex items-center gap-1.5 text-[11px] text-fg-subtle">
            <span class="h-2 w-2 rounded-full {listening ? 'bg-status-running animate-pulse' : 'bg-fg-subtle'}"></span>
            {listening ? "Listening…" : "Starting…"}
          </span>
          <!-- Live level meter from the sidecar's RMS events. -->
          <span class="h-1.5 flex-1 overflow-hidden rounded-full bg-bg">
            <span class="block h-full bg-accent transition-[width] duration-75" style={`width:${Math.round(level * 100)}%`}></span>
          </span>
        {:else}
          <button
            class="inline-flex items-center gap-1.5 rounded-md bg-accent px-3 py-1.5 text-[12px] font-semibold text-on-accent disabled:opacity-50"
            disabled={!model || micDenied}
            onclick={start}
          >
            <Icon name="mic" size={13} /> Record
          </button>
          {#if model}
            {@const eng = installed.find((m) => m.id === model)?.engine}
            {#if eng}<ModelMark family={eng} size={18} class="shrink-0" />{/if}
          {/if}
        {/if}
      </div>

      <!-- Transcript: live partial while recording, final after stop. -->
      <div class="mt-3 min-h-[120px] rounded-md border border-border bg-bg px-3 py-2.5">
        {#if final}
          <p class="whitespace-pre-wrap text-[13px] leading-relaxed text-fg">{final}</p>
        {:else if partial}
          <p class="whitespace-pre-wrap text-[13px] leading-relaxed text-fg-muted">{partial}<span class="inline-block h-3.5 w-[2px] translate-y-0.5 animate-pulse bg-accent"></span></p>
        {:else if recording}
          <p class="text-[12px] text-fg-subtle">Speak — your words appear here as the model hears them.</p>
        {:else}
          <p class="text-[12px] text-fg-subtle">Press Record and start talking.</p>
        {/if}
      </div>

      {#if final}
        <div class="mt-2 flex items-center gap-3">
          <button class="text-[11px] text-accent hover:underline" onclick={copy}>{copied ? "Copied" : "Copy transcript"}</button>
          <span class="text-[10.5px] text-fg-subtle">{final.split(/\s+/).filter(Boolean).length} words</span>
        </div>
      {/if}
    {/if}

    {#if error}
      {#if isMicDeniedError(error)}
        <div class="mt-3 flex flex-wrap items-center justify-between gap-3 rounded-md border border-status-unhealthy/40 bg-status-unhealthy/5 px-3 py-2.5">
          <p class="min-w-0 text-[11px] text-fg-muted">
            macOS blocked the microphone for PortBay — switch it on under Privacy &amp; Security › Microphone, then try again.
          </p>
          <button
            type="button"
            class="shrink-0 rounded-md border border-border px-2.5 py-1.5 text-[11px] text-fg hover:bg-surface-2"
            onclick={() => (showMicDialog = true)}
          >
            Open Privacy Settings…
          </button>
        </div>
      {:else}
        <p class="mt-3 text-[11px] text-status-unhealthy">{error}</p>
      {/if}
    {/if}
  </div>
</section>

<MacPermissionDialog
  open={showMicDialog}
  kind="microphone"
  checkGranted={async () =>
    (await invokeQuiet<SttOverview>("stt_overview")).micPermission === "authorized"}
  onClose={() => {
    showMicDialog = false;
    void load();
  }}
/>
