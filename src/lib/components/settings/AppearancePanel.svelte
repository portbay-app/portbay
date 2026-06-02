<!-- AppearancePanel — theme, density, accent colour. -->
<script lang="ts">
  import Segmented from "$lib/components/atoms/Segmented.svelte";
  import ColorSwatchGroup from "$lib/components/atoms/ColorSwatchGroup.svelte";
  import { density, type Density } from "$lib/stores/density.svelte";
  import { theme, type ThemePreference } from "$lib/stores/theme.svelte";
  import { preferences, type AccentColor } from "$lib/stores/preferences.svelte";
  import SettingsPanel from "./SettingsPanel.svelte";

  const themeOptions: { value: ThemePreference; label: string }[] = [
    { value: "system", label: "System" },
    { value: "light", label: "Light" },
    { value: "dark", label: "Dark" },
  ];

  const densityOptions: { value: Density; label: string }[] = [
    { value: "compact", label: "Compact" },
    { value: "comfortable", label: "Comfortable" },
  ];
</script>

<SettingsPanel title="Appearance" description="Theme, layout density, and accent colour.">
  <div class="divide-y divide-border/60">
    <div class="flex items-center justify-between gap-3 py-2.5 first:pt-0">
      <span class="text-[13px] text-fg">Theme</span>
      <Segmented
        value={theme.preference}
        options={themeOptions}
        label="Theme"
        onchange={(v) => theme.set(v)}
      />
    </div>
    <div class="flex items-center justify-between gap-3 py-2.5">
      <span class="text-[13px] text-fg">Density</span>
      <Segmented
        value={density.value}
        options={densityOptions}
        label="Density"
        onchange={(v) => density.set(v)}
      />
    </div>
    <div class="flex items-center justify-between gap-3 py-2.5 last:pb-0">
      <span class="text-[13px] text-fg">Accent color</span>
      <ColorSwatchGroup
        value={preferences.value.accentColor}
        onchange={(v: AccentColor) => preferences.update({ accentColor: v })}
      />
    </div>
  </div>
</SettingsPanel>
