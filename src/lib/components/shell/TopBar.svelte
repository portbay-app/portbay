<!--
  TopBar — global search, page title, and the three primary action buttons:
  Add Project, Universal Stop-All, Settings.

  Most of the bar is `data-tauri-drag-region` so the user can drag the
  window from anywhere except the interactive controls. Buttons stop the
  drag region via their own elements (Tauri's drag region is opt-in per
  element, not inherited).
-->
<script lang="ts">
  import { page } from "$app/state";
  import { goto } from "$app/navigation";
  import Icon from "$lib/components/atoms/Icon.svelte";
  import { search } from "$lib/stores/search.svelte";
  import { addProjectWizard } from "$lib/stores/wizard.svelte";
  import { tunnels } from "$lib/stores/tunnels.svelte";
  import { tunnelModal } from "$lib/stores/tunnelModal.svelte";
  import StopAllButton from "./StopAllButton.svelte";

  // Map route paths to page titles. Falls back to a humanised path segment
  // if the route isn't in the map (defensive — every Phase 2 route is here).
  const pageTitles: Record<string, string> = {
    "/": "Projects",
    "/services": "Services",
    "/domains": "Domains",
    "/logs": "Logs",
    "/settings": "Settings",
    "/preview": "Atoms preview",
  };

  const currentTitle = $derived.by(() => {
    const path = page.url.pathname;
    if (pageTitles[path]) return pageTitles[path];
    const head = "/" + (path.split("/")[1] ?? "");
    if (pageTitles[head]) return pageTitles[head];
    return "PortBay";
  });

  function openAddProject() {
    addProjectWizard.show();
  }

  function focusSearch() {
    // Live-filter the projects table; the command palette (⌘K) is
    // a separate surface tracked by its own kanban card.
    document.getElementById("portbay-search")?.focus();
  }
</script>

<header
  data-tauri-drag-region
  class="h-14 shrink-0 flex items-center gap-3 px-4 border-b border-border bg-bg select-none"
>
  <h1 data-tauri-drag-region class="text-base font-semibold tracking-tight">
    {currentTitle}
  </h1>

  <!-- Search — live filter for the projects table. -->
  <div class="ml-auto flex items-center">
    <div
      class="relative flex items-center w-64 h-8 rounded-md bg-surface border border-border
             focus-within:border-accent/60 transition-colors"
    >
      <span class="pl-2.5 text-fg-subtle">
        <Icon name="search" size={14} />
      </span>
      <input
        id="portbay-search"
        type="text"
        placeholder="Search projects (⌘K)"
        value={search.value}
        oninput={(e) => search.set((e.currentTarget as HTMLInputElement).value)}
        class="flex-1 bg-transparent text-sm pl-2 pr-3 outline-none text-fg placeholder-fg-subtle"
        aria-label="Search projects"
      />
    </div>
  </div>

  <!-- Action cluster: Add Project, Stop-All, Settings -->
  <div class="flex items-center gap-2">
    {#if tunnels.count > 0}
      <button
        type="button"
        onclick={() => {
          // Single tunnel → jump straight to it. Multiple → open the
          // first one and rely on the user to navigate from there;
          // a dedicated "All tunnels" list is a follow-up.
          const first = tunnels.value[0];
          if (first) tunnelModal.show(first.projectId);
        }}
        title="{tunnels.count} active public tunnel{tunnels.count === 1 ? '' : 's'}"
        aria-label="View active tunnels"
        class="inline-flex items-center gap-1.5 h-8 px-2 rounded-md
               text-accent border border-accent/40 hover:bg-accent/10
               transition-colors text-xs font-medium"
      >
        <Icon name="globe" size={13} />
        <span class="tabular-nums">{tunnels.count}</span>
      </button>
    {/if}
    <button
      type="button"
      onclick={openAddProject}
      title="Add project (⌘N)"
      aria-label="Add project"
      class="inline-flex items-center justify-center w-8 h-8 rounded-md
             text-status-running border border-status-running/30
             hover:bg-status-running/10 hover:border-status-running/60
             transition-colors"
    >
      <Icon name="plus" size={16} />
    </button>

    <StopAllButton />

    <button
      type="button"
      onclick={() => goto("/settings")}
      title="Settings"
      aria-label="Settings"
      class="inline-flex items-center justify-center w-8 h-8 rounded-md
             text-fg-muted border border-border
             hover:text-fg hover:bg-surface-2 hover:border-border-strong
             transition-colors"
    >
      <Icon name="settings" size={16} />
    </button>
  </div>
</header>
