<!--
  EmptyState — shared zero-data placeholder used across routes (SSH, Tasks,
  Logs, Inspector, Certificates, …). A centred icon + title + optional
  description, with an optional actions slot (a CTA button, a hint) rendered
  below. Generalised from the original projects-specific empty state so every
  surface reads the same instead of hand-rolling its own.
-->
<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon, { type IconName } from "$lib/components/atoms/Icon.svelte";

  interface Props {
    /** Icon name (atoms/Icon). Omit for a text-only empty state. */
    icon?: IconName;
    title: string;
    /** Optional secondary line under the title. */
    description?: string;
    /** Tighter vertical padding for inline / in-panel use. */
    compact?: boolean;
    /** Optional actions (CTA button, link, hint) rendered under the text. */
    children?: Snippet;
  }
  let { icon, title, description, compact = false, children }: Props = $props();
</script>

<div
  class="flex flex-col items-center justify-center text-center gap-3 {compact
    ? 'py-6'
    : 'py-12'}"
>
  {#if icon}
    <Icon name={icon} size={28} class="text-fg-subtle" />
  {/if}
  <div class="space-y-1">
    <p class="text-sm font-medium text-fg">{title}</p>
    {#if description}
      <p class="text-xs text-fg-muted max-w-xs">{description}</p>
    {/if}
  </div>
  {#if children}
    {@render children()}
  {/if}
</div>
