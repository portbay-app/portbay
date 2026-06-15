<!--
  DictateAnywhereControls — the system-wide dictation toggles ("Dictate
  anywhere on this Mac" + "Hands-free with a double-tap"), plus the
  Accessibility-grant flow they depend on.

  These are app-wide settings (a global Fn hotkey that types into ANY app), so
  they live in two places that bind the SAME preference: the AI page's
  Speech-to-Text panel (next to the local-model setup they need) and
  Settings → General. This component owns all the probing and the permission
  sheet so both hosts stay thin and never drift out of sync.

  Self-contained: it probes the STT inventory (is a local model installed?) and
  the anywhere status (platform support, Accessibility trust, live monitors) on
  mount. When the platform doesn't support it, it renders nothing.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import FnKey from "$lib/components/atoms/FnKey.svelte";
  import Toggle from "$lib/components/atoms/Toggle.svelte";
  import { MacPermissionDialog } from "$lib/components/permissions";
  import { grantPromptSession } from "$lib/components/permissions/grant-prompt-session";
  import { invokeQuiet } from "$lib/ipc";
  import { preferences, type AppContextRule } from "$lib/stores/preferences.svelte";
  import type { DictationAnywhereStatus } from "$lib/dictation/types";
  import type { SttOverview } from "$lib/types/ai";

  /** A running app for the per-app context picker (backend `AppInfo`). */
  interface AppInfo {
    name: string;
    bundleId: string;
    iconDataUrl?: string | null;
  }

  /** The 7 rewrite contexts the per-app map can assign, with friendly labels.
   * Wire values match `RewriteContext` (snake_case) on the backend. */
  const CONTEXTS: { value: string; label: string }[] = [
    { value: "general_note", label: "Notes / prose" },
    { value: "todo_task", label: "To-do / task" },
    { value: "agent_prompt", label: "AI prompt" },
    { value: "terminal_command", label: "Terminal command" },
    { value: "git_commit", label: "Git commit" },
    { value: "deploy_note", label: "Deploy note" },
    { value: "bug_report", label: "Bug report" },
  ];
  const contextLabel = (value: string) =>
    CONTEXTS.find((c) => c.value === value)?.label ?? value;

  /** Optional "set up a local speech model" hook — shown when the feature is
   * enabled-capable but no model is installed yet. In Settings this jumps to
   * the AI page; on the AI page itself it's omitted (the picker is right
   * there). */
  /** `bordered` wraps the rows in a rounded card — the right look on the AI
   * page where neighbours are cards too. Settings → General lays its rows out
   * flush in a `divide-y` section, so it opts out. */
  let {
    onManageModels,
    bordered = true,
  }: { onManageModels?: () => void; bordered?: boolean } = $props();

  const dict = $derived(preferences.value.dictation);

  /** A local model must be installed for dictate-anywhere to do anything (it
   * captures + transcribes on-device). Probed quietly. */
  let sttInfo = $state<SttOverview | null>(null);
  const hasLocalModel = $derived((sttInfo?.installed.length ?? 0) > 0);

  /** Only streaming engines emit the End-of-Utterance signal — for other
   * models the auto-stop can never fire, so its toggle is disabled with an
   * explanation instead of silently doing nothing. */
  const selectedSupportsEou = $derived(
    sttInfo?.catalog.find((m) => m.id === dict.sttModel)?.streaming ?? false,
  );

  /** Mic permission pre-flight (rides the same stt_overview probe). The
   * capture runs in the sidecar but TCC attributes it to PortBay, so this is
   * the effective state — surfacing it here means the grant happens before
   * the first Fn-hold, not mid-session. */
  const micPermission = $derived(sttInfo?.micPermission ?? "unknown");
  const micDenied = $derived(micPermission === "denied" || micPermission === "restricted");
  let micRequesting = $state(false);
  let showMicDialog = $state(false);

  async function requestMic() {
    micRequesting = true;
    try {
      const granted = await invokeQuiet<boolean>("stt_request_mic_access");
      if (!granted) showMicDialog = true;
      await refreshSttInfo();
    } finally {
      micRequesting = false;
    }
  }

  async function refreshSttInfo() {
    try {
      sttInfo = await invokeQuiet<SttOverview>("stt_overview");
    } catch {
      sttInfo = null;
    }
  }

  /** Platform support + Accessibility trust + live global monitors. `null`
   * means "not answered yet" — distinct from `{supported:false}` (genuinely
   * unsupported, render nothing). `probeFailed` separates a failed/errored
   * probe from the initial loading null so we can offer a retry instead of
   * silently hiding the whole section. */
  let anywhereStatus = $state<DictationAnywhereStatus | null>(null);
  let probeFailed = $state(false);
  async function refreshAnywhereStatus(arm = false, prompt = false) {
    try {
      anywhereStatus = await invokeQuiet<DictationAnywhereStatus>(
        arm ? "dictation_anywhere_arm" : "dictation_anywhere_status",
        arm ? { prompt } : undefined,
      );
      probeFailed = false;
    } catch {
      anywhereStatus = null;
      probeFailed = true;
    }
  }

  /** The drag-to-grant Accessibility sheet (same flow the DNS helper uses).
   * The permission gates the global Fn hotkey and typing into other apps —
   * the mic grant covers the speech engine itself. */
  let showAccessibilityDialog = $state(false);
  /** First-success beat: right after the feature comes fully live (toggle on
   * + trust present), invite an immediate real try — the smoke test that
   * catches a half-configured setup before the user walks away. */
  let showTryItHint = $state(false);

  async function setAnywhere(next: boolean) {
    await preferences.update({ dictation: { ...dict, anywhere: next } });
    if (next) {
      // Turning it on IS the approval moment. `prompt: true` fires macOS's own
      // Accessibility dialog (registering PortBay in the list); if trust is
      // still missing after, the drag-to-grant sheet walks the user through
      // the switch. The arm also (re)installs the hotkey monitors once trust
      // appears.
      await refreshAnywhereStatus(true, true);
      if (anywhereStatus?.supported && !anywhereStatus.trusted) {
        showAccessibilityDialog = true;
      } else if (anywhereStatus?.trusted) {
        showTryItHint = true;
      }
    } else {
      showTryItHint = false;
    }
  }

  async function setAnywhereDoubleTap(next: boolean) {
    await preferences.update({ dictation: { ...dict, anywhereDoubleTap: next } });
  }

  async function setAnywhereAutoStop(next: boolean) {
    await preferences.update({ dictation: { ...dict, anywhereAutoStop: next } });
  }

  async function setAnywhereTapToggle(next: boolean) {
    await preferences.update({ dictation: { ...dict, anywhereTapToggle: next } });
  }

  async function setAnywhereCancelKey(code: number) {
    await preferences.update({ dictation: { ...dict, anywhereCancelKey: code } });
  }

  /** Echo a cue choice out loud, like macOS's alert-sound list — the only
   * way to know what "Glass" sounds like is to hear it. Fire-and-forget;
   * "None" (empty) stays silent. */
  function previewCue(sound: string, volume: number) {
    if (!sound) return;
    void invokeQuiet("dictation_preview_cue", { sound, volume });
  }

  async function setAnywhereCueSound(sound: string) {
    previewCue(sound, dict.anywhereCueVolume);
    await preferences.update({ dictation: { ...dict, anywhereCueSound: sound } });
  }

  async function setAnywhereCueVolume(volume: number) {
    previewCue(dict.anywhereCueSound, volume);
    await preferences.update({ dictation: { ...dict, anywhereCueVolume: volume } });
  }

  /** The cancel-key choices: Esc plus the F-keys that no app's text engine
   * eats (vim/games swallow Esc; F13–F15 are free on full-size keyboards). */
  const CANCEL_KEYS: { code: number; label: string }[] = [
    { code: 53, label: "Esc" },
    { code: 105, label: "F13" },
    { code: 107, label: "F14" },
    { code: 113, label: "F15" },
  ];

  /** Start-cue choices — bare names from /System/Library/Sounds. */
  const CUE_SOUNDS: { value: string; label: string }[] = [
    { value: "Tink", label: "Tink" },
    { value: "Pop", label: "Pop" },
    { value: "Glass", label: "Glass" },
    { value: "Morse", label: "Morse" },
    { value: "", label: "None" },
  ];

  async function setAnywherePolish(next: boolean) {
    await preferences.update({ dictation: { ...dict, anywherePolish: next } });
    if (next) void loadApps();
  }

  // --- Per-app formatting overrides (Gap 4) --------------------------------
  /** The frontmost app's bundle id → rewrite context. Unlisted apps fall back
   * to the built-in default (terminals → Terminal command, else Notes). */
  const appContexts = $derived<AppContextRule[]>(dict.anywhereAppContexts ?? []);
  /** Running apps for the picker, loaded on demand. */
  let runningApps = $state<AppInfo[]>([]);
  const appInfo = (bundleId: string): AppInfo | undefined =>
    runningApps.find((a) => a.bundleId === bundleId);
  /** Apps not already mapped — the "add" picker's options. */
  const addableApps = $derived(
    runningApps.filter((a) => !appContexts.some((r) => r.bundleId === a.bundleId)),
  );

  async function loadApps() {
    try {
      runningApps = (await invokeQuiet<AppInfo[]>("dictation_list_apps")) ?? [];
    } catch {
      runningApps = [];
    }
  }

  async function addAppRule(bundleId: string) {
    if (!bundleId || appContexts.some((r) => r.bundleId === bundleId)) return;
    const next = [...appContexts, { bundleId, context: "general_note" }];
    await preferences.update({ dictation: { ...dict, anywhereAppContexts: next } });
  }

  async function setRuleContext(bundleId: string, context: string) {
    const next = appContexts.map((r) => (r.bundleId === bundleId ? { ...r, context } : r));
    await preferences.update({ dictation: { ...dict, anywhereAppContexts: next } });
  }

  async function removeAppRule(bundleId: string) {
    const next = appContexts.filter((r) => r.bundleId !== bundleId);
    await preferences.update({ dictation: { ...dict, anywhereAppContexts: next } });
  }

  // --- Site favicons in the notch (Automation consent) ---------------------
  /** One running scriptable browser + PortBay's Automation consent for it
   * (backend `BrowserConsent`). Empty = no scriptable browser running, and
   * the row hides — nothing to enable against. */
  interface BrowserConsent {
    name: string;
    bundleId: string;
    consent: "granted" | "not_determined" | "denied" | "not_running";
  }
  let browserConsents = $state<BrowserConsent[]>([]);
  let consentRequesting = $state(false);
  const consentPending = $derived(
    browserConsents.filter((b) => b.consent === "not_determined"),
  );
  const consentGranted = $derived(browserConsents.filter((b) => b.consent === "granted"));
  const consentDenied = $derived(browserConsents.filter((b) => b.consent === "denied"));

  async function refreshBrowserConsents() {
    try {
      browserConsents = (await invokeQuiet<BrowserConsent[]>("dictation_favicon_consent")) ?? [];
    } catch {
      browserConsents = [];
    }
  }

  /** The explicit opt-in: fires macOS's own Automation dialog for each
   * running browser that hasn't answered yet. This button is the ONLY thing
   * that prompts — dictation itself never does (no consent = the notch just
   * keeps the browser's app icon). */
  async function requestBrowserConsents() {
    consentRequesting = true;
    try {
      browserConsents =
        (await invokeQuiet<BrowserConsent[]>("dictation_favicon_consent_request")) ?? [];
    } catch {
      await refreshBrowserConsents();
    } finally {
      consentRequesting = false;
    }
  }

  /** When the feature is on and a local model exists but Accessibility is
   * still missing, open the drag-to-grant sheet. This covers the production
   * case the toggle's own prompt can't: `anywhere` was already enabled (synced
   * prefs / a prior install), so the user never re-flips the toggle and the
   * global Fn monitor stays silently uninstalled. We can't trigger off the Fn
   * key itself — macOS withholds global key events from an untrusted process,
   * so the key is unobservable until the grant exists — so the next-best
   * moment is whenever this panel is seen or the app regains focus. */
  function maybePromptForGrant() {
    if (
      preferences.value.dictation.anywhere &&
      hasLocalModel &&
      anywhereStatus?.supported &&
      !anywhereStatus.trusted
    ) {
      // Mark the session so the root layout's boot check (which covers the
      // "user never opens this panel" path) doesn't stack a second sheet.
      grantPromptSession.shown = true;
      showAccessibilityDialog = true;
    }
  }

  /** Returning from System Settings: re-arm (no prompt) so a freshly granted
   * permission installs the monitors live, without an app restart — and if the
   * grant is still missing, re-surface the sheet. */
  async function onWindowFocused() {
    if (!preferences.value.dictation.anywhere || !hasLocalModel) return;
    await refreshAnywhereStatus(true);
    // A mic grant flipped in System Settings should clear the warning row
    // the moment the user comes back.
    void refreshSttInfo();
    // Same for an Automation grant (or a browser launched meanwhile).
    void refreshBrowserConsents();
    maybePromptForGrant();
  }

  onMount(() => {
    // The root layout loads preferences once; only load here if they haven't
    // landed yet, so each host (AI page + Settings) doesn't re-fetch.
    const ready = preferences.loaded ? Promise.resolve() : preferences.load();
    void ready.then(async () => {
      // Resolve status AND the local-model probe before deciding whether to
      // surface the grant sheet — both gate it.
      await Promise.all([
        refreshAnywhereStatus(),
        invokeQuiet<SttOverview>("stt_overview")
          .then((info) => (sttInfo = info))
          .catch(() => (sttInfo = null)),
      ]);
      // Per-app overrides only matter when polishing — load the picker then.
      if (preferences.value.dictation.anywherePolish) void loadApps();
      void refreshBrowserConsents();
      maybePromptForGrant();
    });

    // Re-arm on focus so granting in System Settings takes effect immediately.
    let unlisten: (() => void) | undefined;
    void getCurrentWindow()
      .onFocusChanged(({ payload: focused }) => {
        if (focused) void onWindowFocused();
      })
      .then((fn) => (unlisten = fn))
      .catch(() => {
        /* not in a Tauri window (simulator) — nothing to listen to */
      });
    return () => unlisten?.();
  });
</script>

{#if anywhereStatus?.supported}
  <div class={bordered ? "rounded-lg border border-border px-3" : ""}>
    <div class="flex items-center justify-between gap-3 py-2.5">
      <div class="min-w-0">
        <span class="text-[13px] text-fg">Dictate anywhere on this Mac</span>
        <p class="text-[11px] text-fg-subtle mt-0.5">
          Hold <FnKey /> in any app — your words are transcribed on-device and typed
          right where you are. Esc cancels.
        </p>
      </div>
      <Toggle
        checked={dict.anywhere}
        disabled={!hasLocalModel}
        label="Dictate anywhere on this Mac"
        onchange={(next) => void setAnywhere(next)}
      />
    </div>

    {#if !hasLocalModel}
      <div class="flex items-center justify-between gap-3 border-t border-border/60 py-2.5">
        <p class="text-[11px] text-fg-subtle">
          Needs a local speech model — dictate-anywhere captures and transcribes on this
          Mac.
        </p>
        {#if onManageModels}
          <button
            type="button"
            class="shrink-0 h-8 px-2.5 rounded-md border border-accent/40 text-[12px] text-accent hover:bg-accent/10 transition-colors"
            onclick={() => onManageModels?.()}
          >
            Set up…
          </button>
        {/if}
      </div>
    {/if}

    {#if hasLocalModel && micDenied}
      <div class="flex items-center justify-between gap-3 border-t border-border/60 py-2.5">
        <div class="min-w-0">
          <span class="text-[12px] text-status-unhealthy">Microphone access is off for PortBay</span>
          <p class="text-[11px] text-fg-subtle mt-0.5">
            Dictation can't hear you until PortBay is switched on under
            Privacy &amp; Security › Microphone.
          </p>
        </div>
        <button
          type="button"
          class="shrink-0 h-8 px-2.5 rounded-md border border-border text-[12px] text-fg hover:bg-surface-2 transition-colors"
          onclick={() => (showMicDialog = true)}
        >
          Open Privacy Settings…
        </button>
      </div>
    {:else if hasLocalModel && dict.anywhere && micPermission === "not_determined"}
      <div class="flex items-center justify-between gap-3 border-t border-border/60 py-2.5">
        <p class="min-w-0 text-[11px] text-fg-subtle">
          macOS will ask for microphone access on your first dictation — grant it now so
          your first Fn-hold isn't interrupted.
        </p>
        <button
          type="button"
          class="shrink-0 h-8 px-2.5 rounded-md border border-accent/40 text-[12px] text-accent hover:bg-accent/10 transition-colors disabled:opacity-50"
          disabled={micRequesting}
          onclick={() => void requestMic()}
        >
          {micRequesting ? "Waiting for macOS…" : "Enable microphone"}
        </button>
      </div>
    {/if}

    {#if showTryItHint && dict.anywhere && hasLocalModel && anywhereStatus?.trusted}
      <div class="flex items-center justify-between gap-3 border-t border-border/60 py-2.5">
        <p class="text-[11px] text-status-running">
          You're set — hold <FnKey /> in any app and speak; release to paste. Esc cancels.
        </p>
        <button
          type="button"
          class="h-7 shrink-0 px-2 rounded-md text-[11px] text-fg-subtle hover:bg-surface-2"
          onclick={() => (showTryItHint = false)}
        >
          Got it
        </button>
      </div>
    {/if}

    {#if dict.anywhere && hasLocalModel}
      <div class="flex items-center justify-between gap-3 border-t border-border/60 py-2.5">
        <div class="min-w-0" class:opacity-60={dict.anywhereTapToggle}>
          <span class="text-[12px] text-fg">Hands-free with a double-tap</span>
          <p class="text-[11px] text-fg-subtle mt-0.5">
            Double-tap <FnKey /> to start without holding the key; tap <FnKey /> (or the
            stop in the notch) when you're done. Turn off if your Fn key's double-tap is
            taken — e.g. Apple Speech's own shortcut.
          </p>
        </div>
        <Toggle
          checked={dict.anywhereDoubleTap && !dict.anywhereTapToggle}
          disabled={dict.anywhereTapToggle}
          label="Hands-free with a double-tap"
          onchange={(next) => void setAnywhereDoubleTap(next)}
        />
      </div>

      <div class="flex items-center justify-between gap-3 border-t border-border/60 py-2.5">
        <div class="min-w-0">
          <span class="text-[12px] text-fg">Automatic: tap to go hands-free</span>
          <p class="text-[11px] text-fg-subtle mt-0.5">
            One quick <FnKey /> tap starts a hands-free session; holding stays push-to-talk
            — the same key, resolved when you release. Replaces the double-tap. Best with
            the system Fn key set to "Do Nothing", or every accidental tap starts a session.
          </p>
        </div>
        <Toggle
          checked={dict.anywhereTapToggle}
          label="Automatic: tap to go hands-free"
          onchange={(next) => void setAnywhereTapToggle(next)}
        />
      </div>
    {/if}

    {#if dict.anywhere && hasLocalModel && (dict.anywhereDoubleTap || dict.anywhereTapToggle)}
      <div
        class="flex items-center justify-between gap-3 border-t border-border/60 py-2.5"
        title={selectedSupportsEou
          ? undefined
          : "Your selected model can't detect the end of an utterance — pick a streaming model (e.g. Parakeet EOU) to use auto-stop."}
      >
        <div class="min-w-0" class:opacity-60={!selectedSupportsEou}>
          <span class="text-[12px] text-fg">Stop when you stop talking</span>
          <p class="text-[11px] text-fg-subtle mt-0.5">
            Hands-free sessions end on their own after a pause — no closing tap. Works
            with the streaming Parakeet EOU model, which detects the end of an
            utterance on-device; other models keep the tap-to-stop.
          </p>
        </div>
        <Toggle
          checked={dict.anywhereAutoStop && selectedSupportsEou}
          disabled={!selectedSupportsEou}
          label="Stop when you stop talking"
          onchange={(next) => void setAnywhereAutoStop(next)}
        />
      </div>
    {/if}

    {#if dict.anywhere && hasLocalModel}
      <div class="flex items-center justify-between gap-3 border-t border-border/60 py-2.5">
        <div class="min-w-0">
          <span class="text-[12px] text-fg">Cancel key</span>
          <p class="text-[11px] text-fg-subtle mt-0.5">
            Cancels a session in flight — nothing is pasted. Pick an F-key if the app you
            dictate into uses Esc itself (vim, games).
          </p>
        </div>
        <select
          class="h-7 shrink-0 rounded-md border border-border bg-surface px-1.5 text-[12px] text-fg"
          value={String(dict.anywhereCancelKey)}
          aria-label="Cancel key"
          onchange={(e) => void setAnywhereCancelKey(Number(e.currentTarget.value))}
        >
          {#each CANCEL_KEYS as key (key.code)}
            <option value={String(key.code)}>{key.label}</option>
          {/each}
        </select>
      </div>

      <div class="flex items-center justify-between gap-3 border-t border-border/60 py-2.5">
        <div class="min-w-0">
          <span class="text-[12px] text-fg">Start sound</span>
          <p class="text-[11px] text-fg-subtle mt-0.5">
            Played when the mic goes live. The volume is independent of your output volume.
          </p>
        </div>
        <span class="flex shrink-0 items-center gap-2">
          <input
            type="range"
            min="0"
            max="1"
            step="0.05"
            value={dict.anywhereCueVolume}
            aria-label="Start sound volume"
            class="w-20 accent-accent"
            disabled={!dict.anywhereCueSound}
            onchange={(e) => void setAnywhereCueVolume(Number(e.currentTarget.value))}
          />
          <select
            class="h-7 rounded-md border border-border bg-surface px-1.5 text-[12px] text-fg"
            value={dict.anywhereCueSound}
            aria-label="Start sound"
            onchange={(e) => void setAnywhereCueSound(e.currentTarget.value)}
          >
            {#each CUE_SOUNDS as sound (sound.value)}
              <option value={sound.value}>{sound.label}</option>
            {/each}
          </select>
        </span>
      </div>

      <div class="flex items-center justify-between gap-3 border-t border-border/60 py-2.5">
        <div class="min-w-0">
          <span class="text-[12px] text-fg">Polish dictation everywhere</span>
          <p class="text-[11px] text-fg-subtle mt-0.5">
            Clean up filler, run-ons, and layout with your Smart Dictation model before the
            text is pasted — terminals get command formatting, everything else gets tidy
            prose. Technical terms are kept verbatim; if the rewrite can't run, your raw
            words are pasted instead.
          </p>
        </div>
        <Toggle
          checked={dict.anywherePolish}
          label="Polish dictation everywhere"
          onchange={(next) => void setAnywherePolish(next)}
        />
      </div>
    {/if}

    {#if dict.anywhere && hasLocalModel && dict.anywherePolish}
      <div class="border-t border-border/60 py-2.5">
        <div class="min-w-0">
          <span class="text-[12px] text-fg">Formatting per app</span>
          <p class="text-[11px] text-fg-subtle mt-0.5">
            Pick how the polish formats text in specific apps. Apps you don't list use
            tidy prose; terminals get command formatting automatically.
          </p>
        </div>

        {#if appContexts.length > 0}
          <ul class="mt-2.5 space-y-1.5">
            {#each appContexts as rule (rule.bundleId)}
              {@const info = appInfo(rule.bundleId)}
              <li class="flex items-center gap-2.5">
                {#if info?.iconDataUrl}
                  <img class="w-5 h-5 rounded shrink-0" src={info.iconDataUrl} alt="" />
                {:else}
                  <span class="w-5 h-5 rounded bg-surface-2 shrink-0"></span>
                {/if}
                <span class="text-[12px] text-fg truncate flex-1 min-w-0" title={rule.bundleId}>
                  {info?.name ?? rule.bundleId}
                </span>
                <select
                  class="h-7 rounded-md border border-border bg-surface px-1.5 text-[12px] text-fg"
                  value={rule.context}
                  onchange={(e) => void setRuleContext(rule.bundleId, e.currentTarget.value)}
                  aria-label="Formatting for {info?.name ?? rule.bundleId}"
                >
                  {#each CONTEXTS as ctx (ctx.value)}
                    <option value={ctx.value}>{ctx.label}</option>
                  {/each}
                </select>
                <button
                  type="button"
                  class="shrink-0 h-7 w-7 inline-flex items-center justify-center rounded-md text-fg-subtle hover:text-fg hover:bg-surface-2 transition-colors"
                  onclick={() => void removeAppRule(rule.bundleId)}
                  aria-label="Remove {info?.name ?? rule.bundleId}"
                  title="Remove"
                >
                  ✕
                </button>
              </li>
            {/each}
          </ul>
        {/if}

        <div class="mt-2.5 flex items-center gap-2">
          <select
            class="h-7 rounded-md border border-border bg-surface px-1.5 text-[12px] text-fg-muted max-w-[60%]"
            value=""
            onchange={(e) => {
              const v = e.currentTarget.value;
              e.currentTarget.value = "";
              void addAppRule(v);
            }}
            aria-label="Add an app to format"
          >
            <option value="" disabled selected>Add an app…</option>
            {#each addableApps as a (a.bundleId)}
              <option value={a.bundleId}>{a.name}</option>
            {/each}
          </select>
          {#if runningApps.length === 0}
            <button
              type="button"
              class="h-7 px-2.5 rounded-md border border-border text-[12px] text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
              onclick={() => void loadApps()}
            >
              Load apps
            </button>
          {/if}
        </div>
      </div>
    {/if}

    {#if dict.anywhere && hasLocalModel && browserConsents.length > 0}
      <div class="flex items-center justify-between gap-3 border-t border-border/60 py-2.5">
        <div class="min-w-0">
          <span class="text-[12px] text-fg">Site icons in the notch</span>
          <p class="text-[11px] text-fg-subtle mt-0.5">
            When you dictate into a browser, show the site's own icon (ChatGPT, GitHub…)
            instead of the browser's. macOS asks once per browser for permission to read
            the active tab's address — it never leaves this Mac.
          </p>
        </div>
        {#if consentPending.length > 0}
          <button
            type="button"
            class="shrink-0 h-8 px-2.5 rounded-md border border-accent/40 text-[12px] text-accent hover:bg-accent/10 transition-colors disabled:opacity-50"
            disabled={consentRequesting}
            onclick={() => void requestBrowserConsents()}
          >
            {consentRequesting
              ? "Waiting for macOS…"
              : `Enable for ${consentPending.map((b) => b.name).join(", ")}`}
          </button>
        {:else if consentGranted.length > 0}
          <span class="shrink-0 text-[11px] text-status-running">
            On for {consentGranted.map((b) => b.name).join(", ")}
          </span>
        {:else if consentDenied.length > 0}
          <span class="shrink-0 text-[11px] text-fg-subtle max-w-[40%] text-right">
            Turned off for {consentDenied.map((b) => b.name).join(", ")} — switch PortBay on
            under Privacy &amp; Security › Automation.
          </span>
        {/if}
      </div>
    {/if}

    {#if dict.anywhere && hasLocalModel && anywhereStatus && !anywhereStatus.trusted}
      <div class="flex items-center justify-between gap-3 border-t border-border/60 py-2.5">
        <div class="min-w-0">
          <span class="inline-flex items-center gap-1.5 text-[12px] text-fg">
            <span class="w-2 h-2 rounded-full bg-amber-400"></span>
            Accessibility permission needed
          </span>
          <p class="text-[11px] text-fg-subtle mt-0.5">
            macOS requires it for the global hotkey and for typing into other apps. Add
            PortBay in System Settings, then re-check.
          </p>
        </div>
        <span class="inline-flex items-center gap-2 shrink-0">
          <button
            type="button"
            class="h-8 px-2.5 rounded-md border border-border text-[12px] text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
            onclick={() => (showAccessibilityDialog = true)}
          >
            Grant access…
          </button>
          <button
            type="button"
            class="h-8 px-2.5 rounded-md border border-accent/40 text-[12px] text-accent hover:bg-accent/10 transition-colors"
            onclick={() => void refreshAnywhereStatus(true)}
          >
            Re-check
          </button>
        </span>
      </div>
    {:else if dict.anywhere && hasLocalModel && anywhereStatus?.trusted && !anywhereStatus.monitoring}
      <div class="flex items-center justify-between gap-3 border-t border-border/60 py-2.5">
        <p class="text-[11px] text-fg-subtle">Permission granted — activating the hotkey…</p>
        <button
          type="button"
          class="h-8 px-2.5 rounded-md border border-accent/40 text-[12px] text-accent hover:bg-accent/10 transition-colors"
          onclick={() => void refreshAnywhereStatus(true)}
        >
          Activate
        </button>
      </div>
    {/if}
  </div>
{:else if probeFailed}
  <!-- The probe errored (IPC race on startup, sidecar not ready). Don't vanish
       silently — say so and offer a retry. A genuine {supported:false} still
       renders nothing. -->
  <div class={bordered ? "rounded-lg border border-border px-3" : ""}>
    <div class="flex items-center justify-between gap-3 py-2.5">
      <div class="min-w-0">
        <span class="text-[13px] text-fg">Dictate anywhere on this Mac</span>
        <p class="text-[11px] text-fg-subtle mt-0.5">
          Couldn't check availability just now. Retry to set up the global hotkey.
        </p>
      </div>
      <button
        type="button"
        class="shrink-0 h-8 px-2.5 rounded-md border border-accent/40 text-[12px] text-accent hover:bg-accent/10 transition-colors"
        onclick={() => void refreshAnywhereStatus()}
      >
        Retry
      </button>
    </div>
  </div>
{/if}

<!-- Accessibility grant sheet — the same drag-to-grant flow as the DNS helper.
     Closing re-probes and re-arms the monitors so the hotkey goes live the
     moment the switch is flipped in System Settings — no restart. -->
<MacPermissionDialog
  open={showAccessibilityDialog}
  kind="accessibility"
  checkGranted={async () =>
    (await invokeQuiet<DictationAnywhereStatus>("dictation_anywhere_status")).trusted}
  onClose={() => {
    showAccessibilityDialog = false;
    void refreshAnywhereStatus(true);
  }}
/>

<MacPermissionDialog
  open={showMicDialog}
  kind="microphone"
  checkGranted={async () =>
    (await invokeQuiet<SttOverview>("stt_overview")).micPermission === "authorized"}
  onClose={() => {
    showMicDialog = false;
    void refreshSttInfo();
  }}
/>
