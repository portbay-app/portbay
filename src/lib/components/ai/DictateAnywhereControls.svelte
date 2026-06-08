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
  import FnKey from "$lib/components/atoms/FnKey.svelte";
  import Toggle from "$lib/components/atoms/Toggle.svelte";
  import { MacPermissionDialog } from "$lib/components/permissions";
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
      }
    }
  }

  async function setAnywhereDoubleTap(next: boolean) {
    await preferences.update({ dictation: { ...dict, anywhereDoubleTap: next } });
  }

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

  onMount(() => {
    // The root layout loads preferences once; only load here if they haven't
    // landed yet, so each host (AI page + Settings) doesn't re-fetch.
    const ready = preferences.loaded ? Promise.resolve() : preferences.load();
    void ready.then(() => {
      void refreshAnywhereStatus();
      invokeQuiet<SttOverview>("stt_overview")
        .then((info) => (sttInfo = info))
        .catch(() => (sttInfo = null));
      // Per-app overrides only matter when polishing — load the picker then.
      if (preferences.value.dictation.anywherePolish) void loadApps();
    });
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

    {#if dict.anywhere && hasLocalModel}
      <div class="flex items-center justify-between gap-3 border-t border-border/60 py-2.5">
        <div class="min-w-0">
          <span class="text-[12px] text-fg">Hands-free with a double-tap</span>
          <p class="text-[11px] text-fg-subtle mt-0.5">
            Double-tap <FnKey /> to start without holding the key; tap <FnKey /> (or the
            stop in the notch) when you're done. Turn off if your Fn key's double-tap is
            taken — e.g. macOS Dictation's own shortcut.
          </p>
        </div>
        <Toggle
          checked={dict.anywhereDoubleTap}
          label="Hands-free with a double-tap"
          onchange={(next) => void setAnywhereDoubleTap(next)}
        />
      </div>
    {/if}

    {#if dict.anywhere && hasLocalModel}
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
  onClose={() => {
    showAccessibilityDialog = false;
    void refreshAnywhereStatus(true);
  }}
/>
