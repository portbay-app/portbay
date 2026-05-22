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
  import { search } from "$lib/stores/search";
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
    // Card #8 wires this to the wizard slide-over.
    console.info("[add-project] stub — wired in card #8");
  }

  function focusSearch() {
    // Card #3 ships a placeholder pill; card-out-of-scope command palette
    // (Phase 3) is the real target.
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

  <!-- Search — wired to a command palette in Phase 3 -->
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
