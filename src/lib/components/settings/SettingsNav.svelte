<!--
  SettingsNav — the left vertical tab list (Cursor/VS Code style).

  A proper WAI-ARIA vertical tablist: roving tabindex (only the active tab is
  in the tab order), Up/Down moves selection and focus, Home/End jump to the
  ends. Selecting a tab calls back to the shell, which reflects it in the URL
  (`?tab=`). On a narrow window the labels drop and it collapses to an icon rail.
-->
<script lang="ts">
  import Icon, { type IconName } from "$lib/components/atoms/Icon.svelte";

  export interface SettingsTab {
    key: string;
    label: string;
    icon: IconName;
  }

  interface Props {
    tabs: SettingsTab[];
    active: string;
    onselect: (key: string) => void;
    class?: string;
  }
  let { tabs, active, onselect, class: cls = "" }: Props = $props();

  let buttons = $state<HTMLButtonElement[]>([]);

  function move(to: number) {
    const i = ((to % tabs.length) + tabs.length) % tabs.length;
    onselect(tabs[i].key);
    buttons[i]?.focus();
  }

  function onKeydown(e: KeyboardEvent, idx: number) {
    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        move(idx + 1);
        break;
      case "ArrowUp":
        e.preventDefault();
        move(idx - 1);
        break;
      case "Home":
        e.preventDefault();
        move(0);
        break;
      case "End":
        e.preventDefault();
        move(tabs.length - 1);
        break;
    }
  }
</script>

<nav class={cls} aria-label="Settings sections">
  <ul
    role="tablist"
    aria-orientation="vertical"
    class="space-y-0.5 max-[900px]:flex max-[900px]:gap-1 max-[900px]:space-y-0 max-[900px]:overflow-x-auto"
  >
    {#each tabs as tab, i (tab.key)}
      {@const isActive = tab.key === active}
      <li role="presentation">
        <button
          bind:this={buttons[i]}
          type="button"
          role="tab"
          id="settings-tab-{tab.key}"
          aria-selected={isActive}
          aria-controls="settings-panel"
          tabindex={isActive ? 0 : -1}
          onclick={() => onselect(tab.key)}
          onkeydown={(e) => onKeydown(e, i)}
          class="w-full flex items-center gap-2.5 pl-2.5 pr-3 h-9 rounded-lg
                 text-[13px] text-left transition-colors
                 max-[900px]:w-auto max-[900px]:shrink-0
                 {isActive
            ? 'bg-surface-2 text-fg font-medium'
            : 'text-fg-muted hover:text-fg hover:bg-surface-2/60'}"
        >
          <Icon name={tab.icon} size={15} class="shrink-0" />
          <span class="truncate max-[680px]:sr-only">{tab.label}</span>
        </button>
      </li>
    {/each}
  </ul>
</nav>
