<!--
  TopBar — page title, command palette trigger, and primary actions.

  Design rules (informed by Emil Kowalski's polish principles):

  - **Color reserved for active state.** Idle buttons are tonally
    flat (text-fg-muted on bg-surface). Hover lifts them to text-fg
    with a subtle background. Only the Add action keeps a soft
    accent halo because it's the highest-frequency primary action.
  - **One hero.** The search/palette trigger gets generous width and
    visual weight; everything else recedes.
  - **Consistent rhythm.** Square 28×28 action buttons (was 32×32),
    gap-1 inside clusters, gap-3 between distinct clusters, divider
    between the actions and the destructive Stop-All.
  - **No competing borders.** Replaced colored outlines with
    background-on-hover; the page reads as a quieter surface that
    invites action rather than shouting at the user.
-->
<script lang="ts">
  import { page } from "$app/state";
  import { goto } from "$app/navigation";
  import Icon from "$lib/components/atoms/Icon.svelte";
  import { addProjectWizard } from "$lib/stores/wizard.svelte";
  import { palette } from "$lib/stores/palette.svelte";
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
    "/languages": "Languages",
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

  function openPalette() {
    palette.show();
  }
</script>

<header
  data-tauri-drag-region
  class="h-14 shrink-0 flex items-center gap-4 px-4 border-b border-border/60 bg-bg/95 backdrop-blur-sm select-none"
>
  <h1
    data-tauri-drag-region
    class="text-[15px] font-semibold tracking-tight text-fg shrink-0"
  >
    {currentTitle}
  </h1>

  <!--
    Search / command palette trigger — the hero of the bar.
    Tightly centred in the available space so the cluster on the
    right reads as secondary actions, not equal players.
  -->
  <div class="flex-1 flex justify-center min-w-0">
    <button
      type="button"
      onclick={openPalette}
      aria-label="Open command palette"
      class="group flex items-center gap-2.5 w-full max-w-md h-8 px-3 rounded-md
             bg-surface/60 hover:bg-surface
             text-fg-subtle hover:text-fg-muted
             border border-border/50 hover:border-border
             focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-accent/60
             transition-all duration-150"
    >
      <Icon name="search" size={13} class="shrink-0" />
      <span class="flex-1 text-left text-[13px] truncate">
        Search or run a command
      </span>
      <kbd
        class="text-[10px] font-mono leading-none px-1.5 py-1 rounded
               border border-border/60 text-fg-subtle bg-bg/40
               group-hover:border-border group-hover:text-fg-muted
               transition-colors"
      >
        ⌘K
      </kbd>
    </button>
  </div>

  <!-- Action cluster: tunnels (when active), Add, Stop-All, Settings -->
  <div class="flex items-center gap-1 shrink-0">
    {#if tunnels.count > 0}
      <button
        type="button"
        onclick={() => {
          const first = tunnels.value[0];
          if (first) tunnelModal.show(first.projectId);
        }}
        title="{tunnels.count} active public tunnel{tunnels.count === 1 ? '' : 's'}"
        aria-label="View active tunnels"
        class="inline-flex items-center gap-1.5 h-7 px-2 rounded-md
               text-accent bg-accent/10 hover:bg-accent/15
               text-[12px] font-medium tabular-nums
               transition-colors"
      >
        <Icon name="globe" size={12} />
        {tunnels.count}
      </button>
    {/if}

    <button
      type="button"
      onclick={openAddProject}
      title="Add project (⌘N)"
      aria-label="Add project"
      class="inline-flex items-center justify-center w-7 h-7 rounded-md
             text-fg-muted hover:text-status-running
             bg-transparent hover:bg-status-running/10
             transition-colors"
    >
      <Icon name="plus" size={14} />
    </button>

    <StopAllButton />

    <!-- Vertical divider between transient + settings actions -->
    <div class="w-px h-4 bg-border/60 mx-1" aria-hidden="true"></div>

    <button
      type="button"
      onclick={() => goto("/settings")}
      title="Settings"
      aria-label="Settings"
      class="inline-flex items-center justify-center w-7 h-7 rounded-md
             text-fg-muted hover:text-fg
             bg-transparent hover:bg-surface-2
             transition-colors"
    >
      <Icon name="settings" size={14} />
    </button>
  </div>
</header>
