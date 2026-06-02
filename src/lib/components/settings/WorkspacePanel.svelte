<!-- WorkspacePanel — default workspace folder, auto-detect, sort, start behaviour. -->
<script lang="ts">
  import Toggle from "$lib/components/atoms/Toggle.svelte";
  import {
    preferences,
    type DefaultSort,
    type StartBehavior,
  } from "$lib/stores/preferences.svelte";
  import SettingsPanel from "./SettingsPanel.svelte";

  const sortOptions: { value: DefaultSort; label: string }[] = [
    { value: "name-asc", label: "Name (A–Z)" },
    { value: "name-desc", label: "Name (Z–A)" },
    { value: "status", label: "Status" },
    { value: "port", label: "Port" },
  ];

  const startOptions: { value: StartBehavior; label: string }[] = [
    { value: "manual", label: "Start manually" },
    { value: "auto", label: "Start automatically" },
  ];

  async function pickWorkspaceFolder() {
    // Dialog plugin opens the native folder picker. Falls back to a toast if
    // the user denies the dialog (e.g. capability missing).
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const result = await open({
        directory: true,
        multiple: false,
        title: "Choose default workspace folder",
        defaultPath: preferences.value.defaultWorkspaceFolder || undefined,
      });
      if (typeof result === "string") {
        await preferences.update({ defaultWorkspaceFolder: result });
      }
    } catch {
      /* dialog plugin already toasted */
    }
  }
</script>

<SettingsPanel
  title="Workspace & Projects"
  description="Where new projects live and how the project list behaves."
>
  <div class="divide-y divide-border/60">
    <div class="flex items-center justify-between gap-3 py-2.5 first:pt-0">
      <span class="text-[13px] text-fg">Default workspace folder</span>
      <div class="flex items-center gap-2">
        <input
          type="text"
          value={preferences.value.defaultWorkspaceFolder}
          oninput={(e) =>
            preferences.update({
              defaultWorkspaceFolder: (e.currentTarget as HTMLInputElement).value,
            })}
          placeholder="~/Projects"
          class="h-8 w-56 rounded-md bg-bg border border-border px-2.5 text-[12px] text-fg font-mono focus:outline-none focus:border-accent/60"
        />
        <button
          type="button"
          onclick={pickWorkspaceFolder}
          class="h-8 px-3 rounded-md border border-border text-[12px] text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
        >
          Change
        </button>
      </div>
    </div>

    <div class="flex items-center justify-between gap-3 py-2.5">
      <span class="text-[13px] text-fg">Auto-detect new projects</span>
      <Toggle
        checked={preferences.value.autoDetectProjects}
        label="Auto-detect new projects"
        onchange={(v) => preferences.update({ autoDetectProjects: v })}
      />
    </div>

    <div class="flex items-center justify-between gap-3 py-2.5">
      <span class="text-[13px] text-fg">Default sort</span>
      <select
        value={preferences.value.defaultSort}
        onchange={(e) =>
          preferences.update({
            defaultSort: (e.currentTarget as HTMLSelectElement).value as DefaultSort,
          })}
        class="h-8 w-56 rounded-md bg-bg border border-border px-2.5 text-[12px] text-fg focus:outline-none focus:border-accent/60"
      >
        {#each sortOptions as opt (opt.value)}
          <option value={opt.value}>{opt.label}</option>
        {/each}
      </select>
    </div>

    <div class="flex items-center justify-between gap-3 py-2.5 last:pb-0">
      <span class="text-[13px] text-fg">Default start behavior</span>
      <select
        value={preferences.value.defaultStartBehavior}
        onchange={(e) =>
          preferences.update({
            defaultStartBehavior: (e.currentTarget as HTMLSelectElement)
              .value as StartBehavior,
          })}
        class="h-8 w-56 rounded-md bg-bg border border-border px-2.5 text-[12px] text-fg focus:outline-none focus:border-accent/60"
      >
        {#each startOptions as opt (opt.value)}
          <option value={opt.value}>{opt.label}</option>
        {/each}
      </select>
    </div>
  </div>
</SettingsPanel>
