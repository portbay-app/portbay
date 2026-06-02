<!-- AccessibilityPanel — visual accessibility controls applied app-wide. -->
<script lang="ts">
  import Segmented from "$lib/components/atoms/Segmented.svelte";
  import Toggle from "$lib/components/atoms/Toggle.svelte";
  import {
    preferences,
    type AccessibilityFocusMode,
    type AccessibilityPrefs,
    type AccessibilityTextScale,
  } from "$lib/stores/preferences.svelte";
  import SettingsPanel from "./SettingsPanel.svelte";

  const textScaleOptions: { value: AccessibilityTextScale; label: string }[] = [
    { value: "normal", label: "Normal" },
    { value: "large", label: "Large" },
    { value: "larger", label: "Larger" },
  ];

  const focusOptions: { value: AccessibilityFocusMode; label: string }[] = [
    { value: "standard", label: "Standard" },
    { value: "strong", label: "Strong" },
  ];

  function updateAccessibility(patch: Partial<AccessibilityPrefs>) {
    void preferences.update({
      accessibility: {
        ...preferences.value.accessibility,
        ...patch,
      },
    });
  }
</script>

<SettingsPanel
  title="Accessibility"
  description="Adjust motion, contrast, text size, focus visibility, and color-independent cues across PortBay."
>
  <div class="space-y-7">
    <section class="divide-y divide-border/60">
      <div
        class="flex items-center justify-between gap-3 py-2.5 first:pt-0 max-[640px]:items-start max-[640px]:flex-col"
      >
        <div class="min-w-0">
          <span class="text-[13px] text-fg">Reduce motion</span>
          <p class="text-[12px] text-fg-subtle mt-0.5">
            Minimizes animations, transitions, shimmer, and smooth scrolling.
          </p>
        </div>
        <Toggle
          checked={preferences.value.accessibility.reduceMotion}
          label="Reduce motion"
          onchange={(reduceMotion) => updateAccessibility({ reduceMotion })}
        />
      </div>

      <div
        class="flex items-center justify-between gap-3 py-2.5 max-[640px]:items-start max-[640px]:flex-col"
      >
        <div class="min-w-0">
          <span class="text-[13px] text-fg">Reduce transparency</span>
          <p class="text-[12px] text-fg-subtle mt-0.5">
            Uses opaque app surfaces instead of vibrancy and translucent panels.
          </p>
        </div>
        <Toggle
          checked={preferences.value.accessibility.reduceTransparency}
          label="Reduce transparency"
          onchange={(reduceTransparency) => updateAccessibility({ reduceTransparency })}
        />
      </div>

      <div
        class="flex items-center justify-between gap-3 py-2.5 max-[640px]:items-start max-[640px]:flex-col"
      >
        <div class="min-w-0">
          <span class="text-[13px] text-fg">High contrast</span>
          <p class="text-[12px] text-fg-subtle mt-0.5">
            Increases foreground, border, surface, and accent contrast in light and dark themes.
          </p>
        </div>
        <Toggle
          checked={preferences.value.accessibility.highContrast}
          label="High contrast"
          onchange={(highContrast) => updateAccessibility({ highContrast })}
        />
      </div>

      <div
        class="flex items-center justify-between gap-3 py-2.5 last:pb-0 max-[640px]:items-start max-[640px]:flex-col"
      >
        <div class="min-w-0">
          <span class="text-[13px] text-fg">Underline links</span>
          <p class="text-[12px] text-fg-subtle mt-0.5">
            Adds a persistent underline so links are not identified by color alone.
          </p>
        </div>
        <Toggle
          checked={preferences.value.accessibility.underlineLinks}
          label="Underline links"
          onchange={(underlineLinks) => updateAccessibility({ underlineLinks })}
        />
      </div>
    </section>

    <section class="divide-y divide-border/60">
      <div
        class="flex items-center justify-between gap-3 py-2.5 first:pt-0 max-[640px]:items-start max-[640px]:flex-col"
      >
        <div class="min-w-0">
          <span class="text-[13px] text-fg">Text size</span>
          <p class="text-[12px] text-fg-subtle mt-0.5">
            Raises compact interface text while preserving the app's information density.
          </p>
        </div>
        <Segmented
          value={preferences.value.accessibility.textScale}
          options={textScaleOptions}
          label="Accessibility text size"
          onchange={(textScale) => updateAccessibility({ textScale })}
        />
      </div>

      <div
        class="flex items-center justify-between gap-3 py-2.5 max-[640px]:items-start max-[640px]:flex-col"
      >
        <div class="min-w-0">
          <span class="text-[13px] text-fg">Focus indicator</span>
          <p class="text-[12px] text-fg-subtle mt-0.5">
            Strong mode draws a larger focus ring for keyboard navigation.
          </p>
        </div>
        <Segmented
          value={preferences.value.accessibility.focusMode}
          options={focusOptions}
          label="Accessibility focus indicator"
          onchange={(focusMode) => updateAccessibility({ focusMode })}
        />
      </div>

      <div
        class="flex items-center justify-between gap-3 py-2.5 last:pb-0 max-[640px]:items-start max-[640px]:flex-col"
      >
        <div class="min-w-0">
          <span class="text-[13px] text-fg">Color-independent status cues</span>
          <p class="text-[12px] text-fg-subtle mt-0.5">
            Adds stronger outlines to status marks so meaning is not carried by color alone.
          </p>
        </div>
        <Toggle
          checked={preferences.value.accessibility.colorIndependentStatus}
          label="Color-independent status cues"
          onchange={(colorIndependentStatus) => updateAccessibility({ colorIndependentStatus })}
        />
      </div>
    </section>

    <section class="rounded-lg border border-border bg-bg p-3">
      <div class="flex items-start justify-between gap-3 max-[520px]:flex-col">
        <div class="min-w-0">
          <p class="text-[13px] font-medium text-fg">Preview</p>
          <p class="mt-1 text-[12px] leading-relaxed text-fg-muted">
            Sample text, link treatment, and status marks follow the active accessibility profile.
          </p>
          <a href="/settings?tab=appearance" class="mt-2 inline-flex text-[12px] text-accent">
            Appearance settings
          </a>
        </div>
        <div
          class="shrink-0 rounded-md border border-border/70 bg-surface/70 px-2.5 py-2 text-[12px] text-fg-muted"
        >
          <div class="flex items-center gap-2">
            <span class="inline-flex h-3 w-3 rounded-full bg-status-running"></span>
            <span>Ready</span>
          </div>
          <div class="mt-1.5 flex items-center gap-2">
            <span class="inline-flex h-3 w-3 rounded-full bg-status-unhealthy"></span>
            <span>Attention</span>
          </div>
          <div class="mt-1.5 flex items-center gap-2">
            <span class="inline-flex h-3 w-3 rounded-full bg-status-crashed"></span>
            <span>Failed</span>
          </div>
        </div>
      </div>
    </section>
  </div>
</SettingsPanel>
