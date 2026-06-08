<!--
  ToolHeader — Svelte port of Void's ToolHeaderWrapper (SidebarChat.tsx). A
  bordered row with a chevron, a title, an italic trailing description, and
  right-aligned error/canceled/loading affordances. When a `children` snippet is
  provided it becomes a collapsible dropdown (animated open/close). Used for the
  agent's tool-call rows and for the "Reasoning" block. Tokens translated from
  Void's `void-*` classes.
-->
<script lang="ts">
  import { untrack, type Snippet } from "svelte";

  import Icon from "$lib/components/atoms/Icon.svelte";
  import IconLoading from "./IconLoading.svelte";

  interface Props {
    title: string;
    /** Italic trailing detail (e.g. the tool's argument / result summary). */
    desc1?: string;
    /** Show a spinner instead of static state (tool still running / reasoning). */
    loading?: boolean;
    isError?: boolean;
    isRejected?: boolean;
    /** Controlled open state (two-way bindable). */
    open?: boolean;
    /** Initial open when uncontrolled. */
    defaultOpen?: boolean;
    children?: Snippet;
  }

  let {
    title,
    desc1,
    loading = false,
    isError = false,
    isRejected = false,
    open = $bindable(),
    defaultOpen = false,
    children,
  }: Props = $props();

  // Uncontrolled fallback: if the parent doesn't bind `open`, manage it here.
  // `defaultOpen` is a one-time seed — untrack makes that intent explicit.
  let internalOpen = $state(untrack(() => defaultOpen));
  const isOpen = $derived(open ?? internalOpen);
  const isDropdown = $derived(children !== undefined);

  function toggle() {
    if (!isDropdown) return;
    if (open !== undefined) open = !open;
    else internalOpen = !internalOpen;
  }
</script>

<div class="w-full overflow-hidden rounded border border-border bg-surface-2 px-2 py-1">
  <!-- header -->
  <div class="flex min-h-[24px] select-none items-center">
    <div
      class="flex w-full items-center justify-between gap-x-2 overflow-hidden {isRejected
        ? 'line-through'
        : ''}"
    >
      <!-- left: chevron + title + italic desc -->
      <button
        type="button"
        onclick={toggle}
        disabled={!isDropdown}
        class="ml-1 flex min-w-0 grow items-center overflow-hidden text-left {isDropdown
          ? 'cursor-pointer hover:brightness-125'
          : 'cursor-default'}"
      >
        {#if isDropdown}
          <Icon
            name="chevron-right"
            size={14}
            class="mr-0.5 shrink-0 text-fg-muted transition-transform duration-100 {isOpen
              ? 'rotate-90'
              : ''}"
          />
        {/if}
        <span class="shrink-0 text-[12px] text-fg-muted">{title}</span>
        {#if desc1}
          <span class="ml-2 truncate text-[11.5px] italic text-fg-subtle">{desc1}</span>
        {/if}
        {#if loading}
          <IconLoading class="ml-1 shrink-0 text-fg-subtle" />
        {/if}
      </button>

      <!-- right: state icons -->
      <div class="flex shrink-0 items-center gap-x-2">
        {#if isError}
          <Icon name="circle-alert" size={14} class="shrink-0 text-status-crashed" />
        {/if}
        {#if isRejected}
          <Icon name="ban" size={14} class="shrink-0 text-fg-subtle opacity-90" />
        {/if}
      </div>
    </div>
  </div>

  <!-- collapsible children -->
  {#if isDropdown}
    <div
      class="overflow-hidden text-fg-muted transition-all duration-200 ease-in-out {isOpen
        ? 'py-1 opacity-100'
        : 'max-h-0 opacity-0'}"
    >
      {#if isOpen}{@render children?.()}{/if}
    </div>
  {/if}
</div>
