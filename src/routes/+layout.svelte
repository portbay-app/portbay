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
  import { goto, afterNavigate } from "$app/navigation";
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
  import { onMount, untrack } from "svelte";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { tunnels } from "$lib/stores/tunnels.svelte";
  import { onboarding } from "$lib/stores/onboarding.svelte";
  import { sidebar } from "$lib/stores/sidebar.svelte";
  import { sidecars } from "$lib/stores/sidecars.svelte";
  import { setupRequirements } from "$lib/stores/setup";
  import Icon from "$lib/components/atoms/Icon.svelte";
  import { preferences } from "$lib/stores/preferences.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import { projectDetailPanel } from "$lib/stores/detailPanel.svelte";
  import { addProjectWizard } from "$lib/stores/wizard.svelte";
  import { databases } from "$lib/stores/databases.svelte";
  import { groupEditor } from "$lib/stores/groupEditor.svelte";
  import { entitlements } from "$lib/stores/entitlements.svelte";
  import { errorBus } from "$lib/stores/errors.svelte";
  import { installCrashReporter } from "$lib/stores/crashReporter.svelte";
  import { updater } from "$lib/stores/updater.svelte";

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
    // The setup banner and Settings "Setup" surface in this layout read sidecar
    // health on every route, so the poll must run layout-wide — not only on the
    // dashboard / Services / Web Servers pages that also start it. Without this,
    // deep-linking to a page that doesn't start the poll leaves the store on its
    // "loading…" placeholder, which falsely reads as "mkcert CA needs setup".
    sidecars.start();
    void preferences.load();
    // Load the cached entitlement immediately (no network), then re-verify a
    // stored session in the background (rotates tokens, refetches the license).
    void entitlements.load().then(() => entitlements.resync());

    // Background update check. Surfaces a non-blocking toast when a newer
    // signed release is published; silent on failure (transient network
    // blips shouldn't nag) and a no-op in the hosted web demo.
    void updater.check({ silent: true });

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

  // Every right-side surface renders into the grid's single rail column so the
  // app speaks one side-panel language — no floating overlay drawers. Exactly
  // one is active at a time, by the precedence below; the passive dashboard
  // rail yields to any explicitly-opened panel.
  const showAddProject = $derived(addProjectWizard.isOpen);
  const showAddDatabase = $derived(databases.wizardOpen);
  const showGroupEditor = $derived(groupEditor.isOpen);
  const showDetailPanel = $derived(projectDetailPanel.id !== null);
  const showRail = $derived(
    !showAddProject &&
      !showAddDatabase &&
      !showGroupEditor &&
      !showDetailPanel &&
      isDashboard &&
      projects.selectedId !== null &&
      density.value !== "compact",
  );

  // grid-template-columns:
  //   sidebar  user-resizable (160–360 px), or forced 180 in compact
  //   main     1fr (greedy; min-w-0 lets it shrink so the grid never overflows)
  //   rail     responsive per active panel — clamp(floor, vw, natural max) so it
  //            scales with the window: never wider than its natural size (no
  //            overflow on small windows) and never cramped (the floor).
  const sidebarCol = $derived(
    density.value === "compact" ? "180px" : `${sidebar.width}px`,
  );
  const railCol = $derived(
    showAddProject
      ? "clamp(380px, 42vw, 600px)"
      : showAddDatabase
        ? "clamp(380px, 40vw, 560px)"
        : showGroupEditor
          ? "clamp(320px, 30vw, 440px)"
          : showDetailPanel
            ? "clamp(340px, 33vw, 480px)"
            : showRail
              ? "clamp(280px, 23vw, 340px)"
              : "0px",
  );
  const gridCols = $derived(`${sidebarCol} 1fr ${railCol}`);
  const currentTheme = $derived(theme.value);

  // Hosted web demo only (try.portbay.app). The desktop build keeps app.html's
  // bare <title> (the Tauri window title comes from tauri.conf.json), so these
  // marketing/share tags ship only when PUBLIC_SIMULATOR is set.
  const isSimulator = import.meta.env.PUBLIC_SIMULATOR === "true";

  // ── Rail panel hygiene ─────────────────────────────────────────────────────
  /** Close every rail panel and clear any dashboard selection. */
  function closeRailPanels() {
    if (projectDetailPanel.id !== null) projectDetailPanel.hide();
    if (addProjectWizard.isOpen) addProjectWizard.hide();
    if (databases.wizardOpen) databases.hideWizard();
    if (groupEditor.isOpen) groupEditor.close();
    if (projects.selectedId !== null) projects.select(null);
  }

  // (1) Navigating to another page closes any open side panel — a panel is
  // never carried across routes.
  afterNavigate((nav) => {
    if (nav.from && nav.from.url.pathname !== nav.to?.url.pathname) {
      closeRailPanels();
    }
  });

  // (2) Single active panel: opening a higher-precedence panel clears the ones
  // below it (precedence: create/edit wizards > project detail > dashboard
  // rail), so closing the top panel never reveals a stale one and panels never
  // layer or carry state. Writes are untracked so this only reacts to a panel
  // *opening*, never to its own cleanup.
  $effect(() => {
    const anyWizard =
      addProjectWizard.isOpen || databases.wizardOpen || groupEditor.isOpen;
    const detailOpen = projectDetailPanel.id !== null;
    untrack(() => {
      if (anyWizard) {
        if (projectDetailPanel.id !== null) projectDetailPanel.hide();
        if (projects.selectedId !== null) projects.select(null);
      } else if (detailOpen) {
        if (projects.selectedId !== null) projects.select(null);
      }
    });
  });

  // Same derivation the Settings "Setup required" surface uses, so the banner
  // count and that list can never disagree.
  const setupReqs = $derived(setupRequirements(sidecars.value));
  const needsSetup = $derived(setupReqs.length > 0);
</script>

<svelte:head>
  {#if isSimulator}
    <title>PortBay — Local development environment manager</title>
    <meta
      name="description"
      content="Manage every local dev project behind clean .test domains with automatic HTTPS and one-click start/stop — no Docker, no config files. Try the live interactive demo of PortBay for macOS."
    />
    <link rel="canonical" href="https://try.portbay.app/" />

    <!-- Open Graph -->
    <meta property="og:type" content="website" />
    <meta property="og:site_name" content="PortBay" />
    <meta property="og:url" content="https://try.portbay.app/" />
    <meta
      property="og:title"
      content="PortBay — Local development environment manager"
    />
    <meta
      property="og:description"
      content="Local dev projects behind clean .test domains with automatic HTTPS and one-click start/stop. Try the live interactive demo for macOS."
    />
    <meta property="og:image" content="https://try.portbay.app/og-image.png" />
    <meta property="og:image:width" content="1200" />
    <meta property="og:image:height" content="630" />

    <!-- Twitter -->
    <meta name="twitter:card" content="summary_large_image" />
    <meta
      name="twitter:title"
      content="PortBay — Local development environment manager"
    />
    <meta
      name="twitter:description"
      content="Local dev projects behind clean .test domains with automatic HTTPS and one-click start/stop. Try the live interactive demo for macOS."
    />
    <meta name="twitter:image" content="https://try.portbay.app/og-image.png" />
  {/if}
</svelte:head>

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
    class="h-screen w-screen grid grid-rows-[minmax(0,1fr)] overflow-hidden"
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

    {#if showAddProject}
      <AddProjectWizard />
    {:else if showAddDatabase}
      <AddDatabaseWizard />
    {:else if showGroupEditor}
      <GroupEditorModal />
    {:else if showDetailPanel}
      <ProjectDetailPanel />
    {:else if showRail}
      <RightRail />
    {/if}
  </div>
{/if}

{#if !isTrayPanel}
  <LogViewer />
  <CommandPalette />
  <ConfirmDialog />
  <SignInSheet />
  <AboutLicenseDialog />
  <FeedbackPrompt />
  <ToastHost />
{/if}
