<!-- GeneralPanel — launch, Dock, reopen, stop-confirm, and the system-wide
     dictation toggles. -->
<script lang="ts">
  import { goto } from "$app/navigation";

  import Toggle from "$lib/components/atoms/Toggle.svelte";
  import DictateAnywhereControls from "$lib/components/ai/DictateAnywhereControls.svelte";
  import { preferences } from "$lib/stores/preferences.svelte";
  import SettingsPanel from "./SettingsPanel.svelte";
</script>

<SettingsPanel
  title="General"
  description="App-level behaviour for launch, the Dock, and project controls."
>
  <div class="space-y-7">
    <section class="divide-y divide-border/60">
      <div class="flex items-center justify-between gap-3 py-2.5 first:pt-0">
        <span class="text-[13px] text-fg">Launch PortBay at login</span>
        <Toggle
          checked={preferences.value.launchAtLogin}
          label="Launch PortBay at login"
          onchange={(v) => preferences.update({ launchAtLogin: v })}
        />
      </div>
      <div class="flex items-center justify-between gap-3 py-2.5">
        <div class="min-w-0">
          <span class="text-[13px] text-fg">Show icon in the Dock</span>
          <p class="text-[12px] text-fg-subtle mt-0.5">
            When off, PortBay stays in the menu bar only — no Dock icon.
          </p>
        </div>
        <Toggle
          checked={preferences.value.showDockIcon}
          label="Show icon in the Dock"
          onchange={(v) => preferences.update({ showDockIcon: v })}
        />
      </div>
      <div class="flex items-center justify-between gap-3 py-2.5">
        <span class="text-[13px] text-fg">Reopen previous projects on launch</span>
        <Toggle
          checked={preferences.value.reopenPreviousProjects}
          label="Reopen previous projects on launch"
          onchange={(v) => preferences.update({ reopenPreviousProjects: v })}
        />
      </div>
      <div class="flex items-center justify-between gap-3 py-2.5 last:pb-0">
        <span class="text-[13px] text-fg">Confirm before stopping all projects</span>
        <Toggle
          checked={preferences.value.confirmBeforeStopAll}
          label="Confirm before stopping all projects"
          onchange={(v) => preferences.update({ confirmBeforeStopAll: v })}
        />
      </div>
    </section>

    <!-- Dictation — system-wide speech controls. These affect the whole app
         (a global Fn hotkey that types into any window), so they live here as
         well as on the AI page next to the local-model setup; both bind the
         same preference. Rendered flush (no card) to match the rows above. -->
    <section class="divide-y divide-border/60">
      <div class="py-2.5 first:pt-0">
        <span class="text-[13px] font-medium text-fg">Dictation</span>
        <p class="text-[12px] text-fg-subtle mt-0.5 leading-relaxed">
          Hold the Fn key to dictate into any app on this Mac. Transcription runs
          on-device with a local speech model — set one up on the AI page.
        </p>
      </div>
      <DictateAnywhereControls bordered={false} onManageModels={() => void goto("/ai")} />
    </section>
  </div>
</SettingsPanel>
