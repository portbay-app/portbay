<!--
  AnywhereGrantBootCheck — boot-time surface for the Accessibility grant.

  Covers the production gap the settings-panel flow can't: "Dictate anywhere"
  is already enabled (persisted prefs) but Accessibility trust is missing, and
  the user never visits the AI page or Settings — they just hold Fn and get
  silence, because macOS withholds global key events from an untrusted process
  (observed live 2026-06-09: TCC held a denied entry for the bundle and nothing
  in-app ever said so). Mounted once in the root layout; shows the same
  drag-to-grant sheet the panel uses, at most once per session
  (grantPromptSession), and only when every gate agrees:
  feature on + local model installed + supported + untrusted.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { MacPermissionDialog } from "$lib/components/permissions";
  import { grantPromptSession } from "./grant-prompt-session";
  import { invokeQuiet } from "$lib/ipc";
  import { preferences } from "$lib/stores/preferences.svelte";
  import type { DictationAnywhereStatus } from "$lib/dictation/types";
  import type { SttOverview } from "$lib/types/ai";

  let open = $state(false);

  onMount(() => {
    const ready = preferences.loaded ? Promise.resolve() : preferences.load();
    void ready.then(async () => {
      const dict = preferences.value.dictation;
      if (!dict.anywhere || dict.sttEngine !== "local" || !dict.sttModel) return;
      if (grantPromptSession.shown) return;
      const [status, stt] = await Promise.all([
        invokeQuiet<DictationAnywhereStatus>("dictation_anywhere_status").catch(() => null),
        invokeQuiet<SttOverview>("stt_overview").catch(() => null),
      ]);
      const hasLocalModel = (stt?.installed.length ?? 0) > 0;
      if (status?.supported && !status.trusted && hasLocalModel && !grantPromptSession.shown) {
        grantPromptSession.shown = true;
        open = true;
      }
    });
  });
</script>

<MacPermissionDialog
  {open}
  kind="accessibility"
  checkGranted={async () =>
    (await invokeQuiet<DictationAnywhereStatus>("dictation_anywhere_status")).trusted}
  onClose={() => {
    open = false;
    // Re-arm so a grant given while the sheet was up activates the Fn
    // monitors live, without an app restart.
    void invokeQuiet("dictation_anywhere_arm", { prompt: false });
  }}
/>
