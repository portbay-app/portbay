<!--
  Settings — left vertical tabs (Cursor/VS Code style).

  This route is a thin shell: page heading → pinned setup banner → a
  [nav | panel] grid. Each category is a self-contained panel under
  `$lib/components/settings/`; only the active one mounts, so each panel owns
  its own data loading (DNS/telemetry/domain/MCP fetches live in their panels,
  not here). Inactive panels do no IPC.

  Tab state lives in the URL as `?tab=<key>` — deep-linkable (the user menu's
  "Settings" lands on Account) and back/forward-aware. It is a *query* param on
  purpose: the root layout closes side-rail panels only when the pathname
  changes, so query-only tab switches never disturb an open panel.

  Save model: every control writes through its store on change — no Save button.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { page } from "$app/state";

  import { preferences } from "$lib/stores/preferences.svelte";
  import SetupRequirements from "$lib/components/setup/SetupRequirements.svelte";
  import {
    SettingsNav,
    type SettingsTab,
    AccountPanel,
    GeneralPanel,
    AppearancePanel,
    WorkspacePanel,
    DomainsPanel,
    IntegrationsPanel,
    AdvancedPanel,
  } from "$lib/components/settings";

  const TABS: SettingsTab[] = [
    { key: "account", label: "Account", icon: "users" },
    { key: "general", label: "General", icon: "settings" },
    { key: "appearance", label: "Appearance", icon: "layers" },
    { key: "workspace", label: "Workspace", icon: "folder" },
    { key: "domains", label: "Domains & HTTPS", icon: "globe" },
    { key: "ai", label: "AI Integrations", icon: "sparkles" },
    { key: "advanced", label: "Advanced", icon: "file-code" },
  ];
  const KEYS = TABS.map((t) => t.key);

  // Active tab from `?tab=`, validated against known keys; defaults to Account.
  const active = $derived.by(() => {
    const t = page.url.searchParams.get("tab");
    return t && KEYS.includes(t) ? t : "account";
  });

  function select(key: string) {
    void goto(`/settings?tab=${key}`, { noScroll: true, keepFocus: true });
  }

  onMount(() => {
    void preferences.load();

    // Arriving from the dashboard's "Fix it →" banner (/settings#setup): bring
    // the pinned Setup surface into view once it has rendered.
    if (window.location.hash === "#setup") {
      requestAnimationFrame(() =>
        document.getElementById("setup")?.scrollIntoView({ block: "start" }),
      );
    }
  });
</script>

<div class="px-6 py-5">
  <!-- Page heading -->
  <header class="space-y-1">
    <h1 class="text-[22px] font-semibold tracking-tight text-fg">Settings</h1>
    <p class="text-[13px] text-fg-muted">
      Control how PortBay manages your local development environment.
    </p>
  </header>

  <!-- Setup required — pinned above the tabs (self-hides when healthy), so the
       dashboard's /settings#setup deep-link works regardless of active tab. -->
  <div class="mt-5">
    <SetupRequirements />
  </div>

  <div
    class="mt-6 grid gap-6 items-start
           grid-cols-[200px_minmax(0,1fr)] max-[680px]:grid-cols-[52px_minmax(0,1fr)]"
  >
    <SettingsNav tabs={TABS} {active} onselect={select} class="sticky top-0" />

    <div
      id="settings-panel"
      role="tabpanel"
      aria-labelledby="settings-tab-{active}"
      class="min-w-0"
    >
      {#if active === "account"}
        <AccountPanel />
      {:else if active === "general"}
        <GeneralPanel />
      {:else if active === "appearance"}
        <AppearancePanel />
      {:else if active === "workspace"}
        <WorkspacePanel />
      {:else if active === "domains"}
        <DomainsPanel />
      {:else if active === "ai"}
        <IntegrationsPanel />
      {:else}
        <AdvancedPanel />
      {/if}
    </div>
  </div>
</div>
