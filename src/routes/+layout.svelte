<!--
  Root layout — the app shell.

  CSS grid: [sidebar 220px] [main 1fr] [rail 320px], with a 56px top bar
  spanning across the main + rail tracks. Sidebar covers the full window
  height so the macOS traffic lights (overlaid via tauri.conf.json's
  titleBarStyle: "Overlay") sit cleanly inside it — matching the
  screenshot's ServBay-style chrome.

  In `compact` density the right rail is hidden, freeing horizontal room
  for denser project tables (the screenshot's "websites" table at 6 rows
  comfortably fits at compact).
-->
<script lang="ts">
  import "../app.css";
  import type { Snippet } from "svelte";
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { Sidebar, TopBar, RightRail } from "$lib/components/shell";
  import { ToastHost } from "$lib/components/errors";
  import { AddProjectWizard } from "$lib/components/wizard";
  import { ProjectDetailPanel } from "$lib/components/projects";
  import { LogViewer } from "$lib/components/logs";
  import TunnelModal from "$lib/components/tunnels/TunnelModal.svelte";
  import GroupEditorModal from "$lib/components/groups/GroupEditorModal.svelte";
  import CommandPalette from "$lib/components/palette/CommandPalette.svelte";
  import { density } from "$lib/stores/density.svelte";
  import { theme } from "$lib/stores/theme.svelte";
  import { onMount } from "svelte";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { tunnels } from "$lib/stores/tunnels.svelte";
  import { onboarding } from "$lib/stores/onboarding.svelte";
  import { sidebar } from "$lib/stores/sidebar.svelte";
  import { preferences } from "$lib/stores/preferences.svelte";
  import { errorBus } from "$lib/stores/errors.svelte";

  onMount(() => {
    tunnels.start();
    void preferences.load();

    // Tray-driven nav: the menu-bar "Preferences…" item emits
    // `portbay://nav` with the target route. Same channel can be used
    // by any future tray item that should land on a specific page.
    let unlistenNav: UnlistenFn | null = null;
    let unlistenToast: UnlistenFn | null = null;
    void (async () => {
      unlistenNav = await listen<string>("portbay://nav", async ({ payload }) => {
        if (typeof payload === "string" && payload.startsWith("/")) {
          await goto(payload);
        }
      });

      // First-run "still running" hint when the user closes the window
      // while close-to-menu-bar is on. Fires at most once across the
      // app's lifetime (the Rust side persists the seen flag).
      unlistenToast = await listen("portbay://close-to-menubar-hint", () => {
        if (preferences.value.closeToMenuBarToastSeen) return;
        errorBus.push({
          code: "TRAY_HINT",
          whatHappened:
            "PortBay is still running in the menu bar.",
          whyItMatters:
            "Quit from the tray icon (or ⌘Q with the window focused) to stop the background processes.",
          whoCausedIt: "system",
          actions: [],
        });
        void preferences.markCloseToastSeen();
      });
    })();

    // First-run detection: refresh onboarding state, redirect to
    // /onboarding when the marker is missing AND the registry has no
    // projects. Don't redirect when the user is already inside the
    // onboarding flow (avoids a reload loop after they hit "Skip").
    void (async () => {
      await onboarding.refresh();
      if (
        onboarding.shouldOnboard &&
        !page.url.pathname.startsWith("/onboarding")
      ) {
        await goto("/onboarding");
      }
    })();
    return () => {
      tunnels.stop();
      unlistenNav?.();
      unlistenToast?.();
    };
  });

  let { children }: { children: Snippet } = $props();

  /** True when the current route owns the full window — hide the app shell. */
  const isFullscreen = $derived(page.url.pathname.startsWith("/onboarding"));

  // grid-template-columns:
  //   sidebar  user-resizable (160–360 px), or forced 180 in compact
  //   main     1fr (greedy)
  //   rail     320px (collapses to 0 in compact)
  const gridCols = $derived(
    density.value === "compact"
      ? "180px 1fr 0px"
      : `${sidebar.width}px 1fr 320px`,
  );
  const currentTheme = $derived(theme.value);
</script>

{#if isFullscreen}
  <!--
    Full-window takeover (used by /onboarding). Sidebar / top bar /
    right rail are hidden so the onboarding flow owns the whole
    surface, including the macOS traffic-light area.
  -->
  <div
    class="h-screen w-screen overflow-hidden bg-bg"
    data-theme-current={currentTheme}
  >
    {@render children()}
  </div>
{:else}
  <div
    class="h-screen w-screen grid grid-rows-[1fr] overflow-hidden"
    style:grid-template-columns={gridCols}
    data-theme-current={currentTheme}
  >
    <Sidebar />

    <div class="flex flex-col min-w-0 min-h-0">
      <TopBar />
      <main class="flex-1 min-h-0 overflow-y-auto bg-bg">
        {@render children()}
      </main>
    </div>

    <RightRail />
  </div>
{/if}

<AddProjectWizard />
<ProjectDetailPanel />
<LogViewer />
<TunnelModal />
<GroupEditorModal />
<CommandPalette />
<ToastHost />
