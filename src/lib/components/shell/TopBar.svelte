<!--
  TopBar — search palette + primary actions + chrome cluster.

  Redesign:
    - Page title moves out of the bar; the route component owns its own
      heading (centred main area, larger type).
    - Search palette trigger stretches to fill the available space —
      it's the dominant element.
    - Primary actions: "+ Add Project" (filled accent) and "Stop All"
      (filled status-crashed). The two land side by side as peer CTAs.
    - Right cluster: theme toggle, notifications bell (with unread
      badge), and a deterministic-avatar user-menu trigger. Tunnels
      pill appears between Stop All and the cluster when active.
-->
<script lang="ts">
  import { goto } from "$app/navigation";
  import Icon from "$lib/components/atoms/Icon.svelte";
  import { addProjectWizard } from "$lib/stores/wizard.svelte";
  import { palette } from "$lib/stores/palette.svelte";
  import { tunnels } from "$lib/stores/tunnels.svelte";
  import { theme } from "$lib/stores/theme.svelte";
  import { notifications } from "$lib/stores/notifications.svelte";
  import StopAllButton from "./StopAllButton.svelte";
  import NotificationsPanel from "./NotificationsPanel.svelte";
  import UserMenu from "./UserMenu.svelte";

  let notificationsOpen = $state<boolean>(false);
  let userMenuOpen = $state<boolean>(false);

  function openAddProject() {
    addProjectWizard.show();
  }

  function openPalette() {
    palette.show();
  }

  function toggleNotifications(e: MouseEvent) {
    // The panels listen on window-click to close on outside-click. Without
    // stopping propagation here, opening the panel and closing it would
    // race in the same click — the bell's click would bubble to the
    // panel's just-attached window listener and immediately close it.
    e.stopPropagation();
    notificationsOpen = !notificationsOpen;
    if (notificationsOpen) userMenuOpen = false;
  }

  function toggleUserMenu(e: MouseEvent) {
    e.stopPropagation();
    userMenuOpen = !userMenuOpen;
    if (userMenuOpen) notificationsOpen = false;
  }

  // Avatar gradient is deterministic — PortBay is single-user, but we
  // still want the topbar's chip to read as "an account UI" so the
  // affordance is unmistakable. "P" initial sits on a brand-flavoured
  // teal→indigo gradient.
  const avatarGradient = "linear-gradient(135deg, #4d9cff 0%, #7b5cff 100%)";
</script>

<header
  data-tauri-drag-region
  class="relative h-14 shrink-0 flex items-center gap-3 px-4
         border-b border-border/60 bg-bg/95 backdrop-blur-sm select-none"
>
  <!--
    Search / command palette trigger — hero of the bar. Grows to fill
    horizontal space; the actions on the right are tight clusters.
  -->
  <div class="flex-1 min-w-0">
    <button
      type="button"
      onclick={openPalette}
      aria-label="Open command palette"
      class="group flex items-center gap-2.5 w-full max-w-xl h-9 px-3
             rounded-lg
             bg-surface/60 hover:bg-surface
             text-fg-subtle hover:text-fg-muted
             border border-border/50 hover:border-border
             focus-visible:outline-none focus-visible:ring-1
             focus-visible:ring-accent/60 transition-all duration-150"
    >
      <Icon name="search" size={13} class="shrink-0" />
      <span class="flex-1 text-left text-[13px] truncate">
        Search projects, domains, groups…
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

  <!-- Primary action cluster -->
  <div class="flex items-center gap-2 shrink-0">
    {#if tunnels.count > 0}
      <button
        type="button"
        onclick={() => void goto("/tunnels")}
        title="{tunnels.count} active public tunnel{tunnels.count === 1 ? '' : 's'}"
        aria-label="View active tunnels"
        class="inline-flex items-center gap-1.5 h-9 px-2.5 rounded-lg
               text-accent bg-accent/10 hover:bg-accent/15
               text-[12px] font-medium tabular-nums transition-colors"
      >
        <Icon name="cloud" size={12} />
        {tunnels.count}
      </button>
    {/if}

    <button
      type="button"
      onclick={openAddProject}
      title="Add project (⌘N)"
      aria-label="Add project"
      class="inline-flex items-center gap-1.5 h-9 px-3 rounded-lg
             text-[13px] font-medium tracking-tight
             bg-accent text-on-accent shadow-sm
             hover:brightness-110 active:brightness-95
             focus-visible:outline-none focus-visible:ring-2
             focus-visible:ring-accent/40 transition"
    >
      <Icon name="plus" size={14} />
      Add Project
    </button>

    <StopAllButton />
  </div>

  <!-- Divider -->
  <div class="w-px h-5 bg-border/60 mx-1 shrink-0" aria-hidden="true"></div>

  <!-- Chrome cluster: theme, notifications, user -->
  <div class="relative flex items-center gap-1 shrink-0">
    <button
      type="button"
      onclick={() => theme.toggle()}
      title="Toggle theme"
      aria-label="Toggle theme ({theme.value === 'dark' ? 'currently dark' : 'currently light'})"
      class="inline-flex items-center justify-center w-9 h-9 rounded-lg
             text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
    >
      <Icon name={theme.value === "dark" ? "sun" : "moon"} size={15} />
    </button>

    <div class="relative">
      <button
        type="button"
        onclick={toggleNotifications}
        title="Notifications"
        aria-label="Notifications ({notifications.unreadCount} unread)"
        aria-expanded={notificationsOpen}
        class="inline-flex items-center justify-center w-9 h-9 rounded-lg
               text-fg-muted hover:text-fg hover:bg-surface-2 transition-colors"
      >
        <Icon name="bell" size={15} />
        {#if notifications.unreadCount > 0}
          <span
            class="absolute top-1 right-1.5 min-w-[14px] h-3.5 px-1
                   rounded-full bg-status-crashed text-on-accent
                   text-[9px] leading-[14px] font-semibold text-center
                   tabular-nums shadow"
          >
            {notifications.unreadCount > 9 ? "9+" : notifications.unreadCount}
          </span>
        {/if}
      </button>
      <NotificationsPanel
        open={notificationsOpen}
        onclose={() => (notificationsOpen = false)}
      />
    </div>

    <div class="relative">
      <button
        type="button"
        onclick={toggleUserMenu}
        title="User menu"
        aria-label="User menu"
        aria-expanded={userMenuOpen}
        class="inline-flex items-center gap-1 h-9 pl-1 pr-1.5 rounded-lg
               hover:bg-surface-2 transition-colors"
      >
        <span
          class="inline-flex items-center justify-center w-7 h-7 rounded-full
                 text-on-accent text-[11px] font-semibold tracking-tight
                 shadow-inner"
          style:background={avatarGradient}
        >
          P
        </span>
        <Icon
          name="chevron-down"
          size={12}
          class="text-fg-subtle"
        />
      </button>
      <UserMenu
        open={userMenuOpen}
        onclose={() => (userMenuOpen = false)}
      />
    </div>
  </div>
</header>
