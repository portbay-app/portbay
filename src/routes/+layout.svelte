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
  import AddDatabaseWizard from "$lib/components/databases/AddDatabaseWizard.svelte";
  import { ProjectDetailPanel } from "$lib/components/projects";
  import { LogViewer } from "$lib/components/logs";
  import GroupEditorModal from "$lib/components/groups/GroupEditorModal.svelte";
  import CommandPalette from "$lib/components/palette/CommandPalette.svelte";
  import { ConfirmDialog } from "$lib/components/atoms";
  import { SignInSheet, AboutLicenseDialog } from "$lib/components/account";
  import FeedbackPrompt from "$lib/components/lifecycle/FeedbackPrompt.svelte";
  import { density } from "$lib/stores/density.svelte";
  import { theme } from "$lib/stores/theme.svelte";
  import { onMount } from "svelte";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { tunnels } from "$lib/stores/tunnels.svelte";
  import { onboarding } from "$lib/stores/onboarding.svelte";
  import { sidebar } from "$lib/stores/sidebar.svelte";
  import { sidecars } from "$lib/stores/sidecars.svelte";
  import { setupRequirements } from "$lib/stores/setup";
  import Icon from "$lib/components/atoms/Icon.svelte";
  import { preferences } from "$lib/stores/preferences.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import { entitlements } from "$lib/stores/entitlements.svelte";
  import { errorBus } from "$lib/stores/errors.svelte";
  import { installCrashReporter } from "$lib/stores/crashReporter.svelte";

  /** Compact byte label for the auto-clean "freed N" toast. */
  function formatBytes(n: number): string {
    if (n <= 0) return "0 B";
    const units = ["B", "KB", "MB", "GB", "TB"];
    const i = Math.min(units.length - 1, Math.floor(Math.log(n) / Math.log(1024)));
    const v = n / 1024 ** i;
    return `${i === 0 || v >= 100 ? Math.round(v) : v.toFixed(1)} ${units[i]}`;
  }

  onMount(() => {
    // The tray popover renders in its own webview — it must not start
    // tunnels, listen for nav, or redirect to onboarding. Those side
    // effects belong to the main window's instance of this layout.
    if (isTrayPanel) return;

    installCrashReporter();
    tunnels.start();
    // The projects store has page-spanning lifetime — it's read by
    // /domains, /services, /logs, /languages, the right rail, the
    // sidebar, and the command palette. Starting it in the root
    // layout (mounted once per webview) keeps the listener alive
    // across route navigation; without this, navigating away from
    // the dashboard could leave other routes looking at a stale or
    // emptied store.
    void projects.start();
    void preferences.load();
    // Load the cached entitlement immediately (no network), then re-verify a
    // stored session in the background (rotates tokens, refetches the license).
    void entitlements.load().then(() => entitlements.resync());

    // Tray-driven nav: the menu-bar "Preferences…" item emits
    // `portbay://nav` with the target route. Same channel can be used
    // by any future tray item that should land on a specific page.
    let unlistenNav: UnlistenFn | null = null;
    let unlistenToast: UnlistenFn | null = null;
    let unlistenClean: UnlistenFn | null = null;
    void (async () => {
      unlistenNav = await listen<string>("portbay://nav", async ({ payload }) => {
        if (typeof payload === "string" && payload.startsWith("/")) {
          await goto(payload);
        }
      });

      // Background auto-clean finished a pass — surface the space reclaimed.
      // The Rust scheduler only emits when bytes were actually freed.
      unlistenClean = await listen<number>(
        "portbay://artifacts-auto-cleaned",
        ({ payload }) => {
          const freed = typeof payload === "number" ? payload : 0;
          if (freed <= 0) return;
          errorBus.push({
            code: "ARTIFACTS_AUTO_CLEANED",
            whatHappened: `Freed ${formatBytes(freed)} of build artifacts.`,
            whyItMatters:
              "The scheduled auto-clean removed stale build output across your projects.",
            whoCausedIt: "system",
            severity: "success",
            actions: [],
          });
        },
      );

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
          severity: "info",
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
      unlistenClean?.();
    };
  });

  let { children }: { children: Snippet } = $props();

  /**
   * The tray-panel route runs in its own webview window. It must render
   * without the app shell, without the onboarding redirect, and without
   * subscribing to the tunnels / nav listeners — those live on the main
   * window only. We detect it up front and short-circuit the layout.
   */
  const isTrayPanel = $derived(page.url.pathname.startsWith("/tray-panel"));

  /** True when the current route owns the full window — hide the app shell. */
  const isFullscreen = $derived(page.url.pathname.startsWith("/onboarding"));

  /**
   * The right rail is project-detail surface only. It appears on the
   * Projects dashboard, and only when the user has clicked a row.
   * Other routes (Settings, Domains, Services, Logs, Groups, …) reclaim
   * the horizontal space the rail would otherwise occupy.
   */
  const isDashboard = $derived(page.url.pathname === "/");
  const showRail = $derived(
    isDashboard && projects.selectedId !== null,
  );

  // grid-template-columns:
  //   sidebar  user-resizable (160–360 px), or forced 180 in compact
  //   main     1fr (greedy)
  //   rail     320px when a project is selected on the dashboard, else 0
  const gridCols = $derived(
    density.value === "compact"
      ? "180px 1fr 0px"
      : `${sidebar.width}px 1fr ${showRail ? "320px" : "0px"}`,
  );
  const currentTheme = $derived(theme.value);

  // Same derivation the Settings "Setup required" surface uses, so the banner
  // count and that list can never disagree.
  const setupReqs = $derived(setupRequirements(sidecars.value));
  const needsSetup = $derived(setupReqs.length > 0);
</script>

{#if isTrayPanel}
  <!--
    Tray popover — owns the whole webview, no shell, no chrome, no
    redirects. The route owns its own background (transparent).
  -->
  <div class="h-screen w-screen overflow-hidden" data-theme-current={currentTheme}>
    {@render children()}
  </div>
{:else if isFullscreen}
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
      {#if needsSetup}
        <div
          class="shrink-0 flex items-center gap-2 px-4 py-2
                 bg-amber-500/10 border-b border-amber-500/20
                 text-amber-400 text-[12px]"
        >
          <Icon name="circle-alert" size={13} />
          <span
            >{setupReqs.length}
            {setupReqs.length === 1 ? "tool needs" : "tools need"} setup: {setupReqs
              .map((r) => r.title)
              .join(", ")}.</span
          >
          <a href="/settings#setup" class="ml-auto font-medium hover:underline"
            >Fix it →</a
          >
        </div>
      {/if}
      <main class="flex-1 min-h-0 overflow-y-auto bg-bg">
        {@render children()}
      </main>
    </div>

    {#if showRail}
      <RightRail />
    {/if}
  </div>
{/if}

{#if !isTrayPanel}
  <AddProjectWizard />
  <AddDatabaseWizard />
  <ProjectDetailPanel />
  <LogViewer />
  <GroupEditorModal />
  <CommandPalette />
  <ConfirmDialog />
  <SignInSheet />
  <AboutLicenseDialog />
  <FeedbackPrompt />
  <ToastHost />
{/if}
