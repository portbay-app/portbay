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
  import CrashReportCard from "$lib/components/lifecycle/CrashReportCard.svelte";
  import { density } from "$lib/stores/density.svelte";
  import { theme } from "$lib/stores/theme.svelte";
  import { onMount, untrack } from "svelte";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { tunnels } from "$lib/stores/tunnels.svelte";
  import { onboarding } from "$lib/stores/onboarding.svelte";
  import { sidebar } from "$lib/stores/sidebar.svelte";
  import { sidecars } from "$lib/stores/sidecars.svelte";
  import { setupRequirements } from "$lib/stores/setup";
  import Icon from "$lib/components/atoms/Icon.svelte";
  import { preferences } from "$lib/stores/preferences.svelte";
  import { notificationPrefs } from "$lib/stores/notificationPrefs.svelte";
  import { projects } from "$lib/stores/projects.svelte";
  import { projectDetailPanel } from "$lib/stores/detailPanel.svelte";
  import { addProjectWizard } from "$lib/stores/wizard.svelte";
  import { databases } from "$lib/stores/databases.svelte";
  import { groupEditor } from "$lib/stores/groupEditor.svelte";
  import { entitlements } from "$lib/stores/entitlements.svelte";
  import { errorBus } from "$lib/stores/errors.svelte";
  import { installCrashReporter } from "$lib/stores/crashReporter.svelte";
  import { crashSurface } from "$lib/stores/crashSurface.svelte";
  import { updater } from "$lib/stores/updater.svelte";
  import { dbApprovals } from "$lib/stores/dbApprovals.svelte";
  import WriteApprovalModal from "$lib/components/databases/WriteApprovalModal.svelte";
  import SshCredentialPrompt from "$lib/components/connections/SshCredentialPrompt.svelte";
  import SshHostKeyPrompt from "$lib/components/connections/SshHostKeyPrompt.svelte";
  import SshKbiPrompt from "$lib/components/connections/SshKbiPrompt.svelte";

  /** Compact byte label for the auto-clean "freed N" toast. */
  function formatBytes(n: number): string {
    if (n <= 0) return "0 B";
    const units = ["B", "KB", "MB", "GB", "TB"];
    const i = Math.min(units.length - 1, Math.floor(Math.log(n) / Math.log(1024)));
    const v = n / 1024 ** i;
    return `${i === 0 || v >= 100 ? Math.round(v) : v.toFixed(1)} ${units[i]}`;
  }

  onMount(() => {
    document.body.dataset.platform = navigator.userAgent.includes("Linux")
      ? "linux"
      : navigator.userAgent.includes("Mac")
        ? "macos"
        : "other";

    // The tray popover renders in its own webview — it must not start
    // tunnels, listen for nav, or redirect to onboarding. Those side
    // effects belong to the main window's instance of this layout.
    if (isTrayPanel) return;

    // Reveal the main window only once the webview has painted the themed UI.
    // The window is created hidden (`visible: false` in tauri.conf.json); macOS
    // would otherwise show the webview's white default backing for a frame
    // before our dark surface + vibrancy composite — the classic Tauri launch
    // flash. Two rAFs ensure the first paint has landed before we show.
    if (!isSimulator) {
      requestAnimationFrame(() =>
        requestAnimationFrame(() => {
          // `show()` returns a promise — a synchronous try/catch can't catch
          // its rejection, so swallow it on the promise itself. Otherwise a
          // denied/absent window surfaces as an unhandled rejection (and the
          // crash reporter logs it on every launch).
          void getCurrentWindow()
            .show()
            .catch(() => {
              /* not in a Tauri window, or show not permitted — nothing to do */
            });
        }),
      );
    }

    installCrashReporter();
    // Surface any crash left over from a previous session (e.g. a panic that
    // took the app down) as a one-click "send report" card — but only after the
    // launch has settled, so it never fights the window reveal / onboarding.
    const crashTimer = setTimeout(() => void crashSurface.presentLatestPending(), 2500);
    tunnels.start();
    dbApprovals.start();
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
    // Populate the databases store once at boot so the sidebar "Databases"
    // nav badge shows the running-instance count app-wide (the /databases page
    // refreshes it live when open).
    void databases.refresh();
    void preferences.load();
    void notificationPrefs.load();
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
            category: "lifecycle",
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
          category: "lifecycle",
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
      clearTimeout(crashTimer);
      tunnels.stop();
      dbApprovals.stop();
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
  //   sidebar  user-resizable (160–360 px), or a 60px icon-only strip in compact
  //   main     1fr (greedy; min-w-0 lets it shrink so the grid never overflows)
  //   rail     responsive per active panel — clamp(floor, vw, natural max) so it
  //            scales with the window: never wider than its natural size (no
  //            overflow on small windows) and never cramped (the floor).
  const sidebarCol = $derived(
    density.value === "compact" ? "60px" : `${sidebar.width}px`,
  );

  // Any right-side panel currently occupying the rail slot.
  const anyPanel = $derived(
    showAddProject ||
      showAddDatabase ||
      showGroupEditor ||
      showDetailPanel ||
      showRail,
  );

  // The panel's natural width — used both as the pushed grid column (wide
  // windows) and as the floating overlay's width (narrow windows).
  const panelWidth = $derived(
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

  // Live window width (bound via <svelte:window>). 0 until the first measure.
  let winWidth = $state(0);

  // Below this width a pushed panel would squash the main content (sidebar +
  // a ~380px rail leaves too little room), so the panel floats over the content
  // instead of pushing it. Above it, the original push behaviour is kept.
  const OVERLAY_BELOW = 1180;

  // Overlay only matters while a panel is open; 0 width = not yet measured, so
  // default to the push layout until we know the real width.
  const overlayRail = $derived(
    anyPanel && winWidth > 0 && winWidth < OVERLAY_BELOW,
  );

  // In overlay mode the rail column collapses to 0 so main content keeps its
  // full width; the panel is rendered absolutely on top instead.
  const railCol = $derived(overlayRail ? "0px" : panelWidth);
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
    // Null-safe throughout: an exception thrown in an afterNavigate callback
    // propagates into SvelteKit's client navigation and forces a hard
    // `location.href` reload — which tears down the whole shell (sidebar
    // included) and reads as a full page refresh. `nav.from`/`nav.to` and
    // their `url` can be absent on the first navigation, so optional-chain all
    // the way down and only act when the pathname actually changes.
    const fromPath = nav.from?.url?.pathname;
    const toPath = nav.to?.url?.pathname;
    if (fromPath && fromPath !== toPath) {
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
    <!--
      Title / description / canonical / Open Graph / Twitter tags for
      try.portbay.app are stamped per-route into the static HTML at build time
      by scripts/stamp-og-meta.mjs (run from `pnpm build:web`). That is the only
      thing link unfurlers see, since the app is a client-rendered SPA
      (ssr=false) and crawlers don't execute JavaScript. Tags injected here at
      runtime would be invisible to them and would duplicate the stamped ones in
      the live DOM, so the meta lives entirely in the build step now.
    -->
    <title>PortBay — Local development environment manager</title>
  {/if}
</svelte:head>

<svelte:window bind:innerWidth={winWidth} />

<!-- The active right-side panel. Rendered either as the pushed grid column
     (wide windows) or inside a floating overlay (narrow windows). -->
{#snippet railPanel()}
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
{/snippet}

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
    class="h-screen w-screen overflow-hidden bg-app"
    data-theme-current={currentTheme}
  >
    {@render children()}
  </div>
{:else}
  <div
    class="relative h-screen w-screen grid grid-rows-[minmax(0,1fr)] overflow-hidden
           transition-[grid-template-columns] duration-200 ease-out motion-reduce:transition-none"
    style:grid-template-columns={gridCols}
    data-theme-current={currentTheme}
  >
    <Sidebar />

    <div class="flex flex-col min-w-0 min-h-0 bg-app">
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
      <main class="flex-1 min-h-0 overflow-y-auto bg-app">
        {@render children()}
      </main>
    </div>

    {#if anyPanel && !overlayRail}
      <!-- Wide window: the panel pushes — it's the grid's third column. -->
      {@render railPanel()}
    {/if}

    {#if anyPanel && overlayRail}
      <!-- Narrow window: the panel floats over the content so the layout never
           squashes. A scrim behind it dismisses every rail panel on click. -->
      <button
        type="button"
        aria-label="Close panel"
        onclick={closeRailPanels}
        class="absolute inset-0 z-40 bg-black/40 motion-safe:animate-[fade-in_120ms_ease-out]"
      ></button>
      <div
        class="absolute right-0 top-0 bottom-0 z-50 max-w-[92vw] shadow-2xl
               motion-safe:animate-[slide-in-right_180ms_cubic-bezier(0.22,1,0.36,1)]"
        style:width={panelWidth}
      >
        {@render railPanel()}
      </div>
    {/if}
  </div>
{/if}

{#if !isTrayPanel}
  <LogViewer />
  <CommandPalette />
  <ConfirmDialog />
  <SshCredentialPrompt />
  <SshHostKeyPrompt />
  <SshKbiPrompt />
  <WriteApprovalModal />
  <SignInSheet />
  <AboutLicenseDialog />
  <FeedbackPrompt />
  <CrashReportCard />
  <ToastHost />
{/if}
