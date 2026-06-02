<!--
  ProjectSelector — shared project-picker dropdown with a sticky search box.

  Used on /logs, /tasks (board picker), and /inspector. Replaces three
  near-duplicate inline pickers that all lacked a search field.

  Props:
    projects       — the list to pick from (ProjectView[])
    selectedId     — currently selected project id, or null for "all"
    disabled       — disables the trigger button
    includeAllOption — prepend an "all" option (default true)
    allOptionLabel — label for the all option (default "All projects")
    onselect       — callback(projectId: string | null)
-->
<script lang="ts">
  import { Icon, ProjectAvatar, StatusDot } from "$lib/components/atoms";
  import type { ProjectView } from "$lib/types/projects";

  interface Props {
    projects: ProjectView[];
    selectedId: string | null;
    disabled?: boolean;
    includeAllOption?: boolean;
    allOptionLabel?: string;
    onselect: (projectId: string | null) => void;
  }

  let {
    projects,
    selectedId,
    disabled = false,
    includeAllOption = true,
    allOptionLabel = "All projects",
    onselect,
  }: Props = $props();

  let open = $state(false);
  let searchQuery = $state("");
  let searchEl = $state<HTMLInputElement | null>(null);
  let root = $state<HTMLElement | null>(null);

  const selected = $derived(
    selectedId === null ? null : (projects.find((p) => p.id === selectedId) ?? null),
  );

  const filtered = $derived.by(() => {
    const q = searchQuery.trim().toLowerCase();
    if (!q) return projects;
    return projects.filter((p) => p.name.toLowerCase().includes(q));
  });

  function toggle() {
    if (disabled) return;
    open = !open;
    if (open) {
      searchQuery = "";
      // Defer focus until the input is rendered.
      requestAnimationFrame(() => searchEl?.focus());
    }
  }

  function select(id: string | null) {
    open = false;
    onselect(id);
  }

  // Close on outside-click or Escape.
  $effect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (root && !root.contains(e.target as Node)) open = false;
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") open = false;
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  });
</script>

<div class="relative" bind:this={root}>
  <!-- Trigger -->
  <button
    type="button"
    onclick={toggle}
    {disabled}
    aria-haspopup="listbox"
    aria-expanded={open}
    class="flex items-center gap-2 h-9 w-56 px-2.5 rounded-lg bg-surface-2
           border border-border text-left hover:border-border-strong
           disabled:opacity-50 disabled:cursor-not-allowed transition-colors
           focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40"
  >
    {#if selected}
      <ProjectAvatar
        id={selected.id}
        name={selected.name}
        type={selected.type}
        size={20}
      />
      <span class="flex-1 truncate text-[13px] font-medium text-fg"
        >{selected.name}</span
      >
    {:else if selectedId === null && includeAllOption}
      <span class="flex-1 truncate text-[13px] text-fg-muted">{allOptionLabel}</span>
    {:else}
      <span class="flex-1 truncate text-[13px] text-fg-subtle">
        {projects.length === 0 ? "No projects" : "Select a project"}
      </span>
    {/if}
    <Icon name="chevron-down" size={16} class="text-fg-subtle shrink-0" />
  </button>

  <!-- Dropdown -->
  {#if open}
    <div
      role="listbox"
      aria-label="Select a project"
      class="absolute z-30 mt-1.5 w-64 rounded-lg bg-surface border border-border shadow-2xl p-1"
    >
      <!-- Sticky search -->
      <div class="px-1 pb-1">
        <div
          class="flex items-center gap-1.5 h-8 rounded-md bg-bg border border-border px-2
                 focus-within:border-accent/60 transition-colors"
        >
          <Icon name="search" size={12} class="text-fg-subtle shrink-0" />
          <input
            bind:this={searchEl}
            type="text"
            bind:value={searchQuery}
            placeholder="Search…"
            class="flex-1 bg-transparent text-[12px] outline-none text-fg placeholder:text-fg-subtle"
          />
        </div>
      </div>

      <div class="max-h-60 overflow-y-auto">
        <!-- All option -->
        {#if includeAllOption}
          <button
            type="button"
            role="option"
            aria-selected={selectedId === null}
            onclick={() => select(null)}
            class="w-full flex items-center gap-2.5 px-2 py-1.5 rounded-md text-left text-[13px]
                   transition-colors {selectedId === null
              ? 'bg-accent/10 text-fg'
              : 'text-fg-muted hover:bg-surface-2 hover:text-fg'}"
          >
            <span class="flex-1 truncate">{allOptionLabel}</span>
            {#if selectedId === null}
              <Icon name="check" size={13} class="text-accent shrink-0" />
            {/if}
          </button>
        {/if}

        <!-- Project list -->
        {#if filtered.length === 0}
          <p class="px-3 py-3 text-[12px] text-fg-subtle italic">No projects match.</p>
        {:else}
          {#each filtered as p (p.id)}
            <button
              type="button"
              role="option"
              aria-selected={p.id === selectedId}
              onclick={() => select(p.id)}
              class="w-full flex items-center gap-2.5 px-2 py-1.5 rounded-md text-left
                     transition-colors {p.id === selectedId
                ? 'bg-accent/10'
                : 'hover:bg-surface-2'}"
            >
              <ProjectAvatar id={p.id} name={p.name} type={p.type} size={20} />
              <span class="flex-1 truncate text-[13px] text-fg">{p.name}</span>
              <StatusDot status={p.status} size="md" />
            </button>
          {/each}
        {/if}
      </div>
    </div>
  {/if}
</div>
