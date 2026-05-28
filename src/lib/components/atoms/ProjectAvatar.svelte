<!--
  ProjectAvatar — the project's visual identity in a rounded tile.

  Shows the project's *real* icon when one is detected on disk (favicon,
  web-clip, or native app icon — see `crate::project_icon`), contained on a
  neutral surface tile so transparent / non-square marks read cleanly. When
  nothing is detected, it falls back to the project's stack glyph
  (`StackIcon`) on the same tile, so e.g. a plain Next app still reads as
  Next. The detected icon is fetched once per project id via the memoized
  `loadProjectIcon` loader.

  Callers may pass an explicit `icon` URL to override detection entirely
  (e.g. a future user-supplied icon picker).
-->
<script lang="ts">
  import type { ProjectType } from "$lib/types/projects";
  import { loadProjectIcon } from "$lib/projectIcon";
  import StackIcon from "./StackIcon.svelte";

  interface Props {
    /** Stable identifier — the key the detected icon is fetched + cached by. */
    id: string;
    /** Display name — used for the image alt / title only. */
    name: string;
    /** Stack type — drives the fallback glyph when no icon is detected. */
    type: ProjectType;
    size?: number;
    /** Optional explicit icon URL. When set, overrides on-disk detection. */
    icon?: string | null;
    class?: string;
  }
  let { id, name, type, size = 28, icon = null, class: cls = "" }: Props =
    $props();

  // The detected icon URL, resolved asynchronously. Stays null while loading
  // or when nothing is found — either way we render the stack glyph until (and
  // unless) a real icon arrives, so there's no blank flash.
  let detected = $state<string | null>(null);

  $effect(() => {
    // An explicit override skips detection entirely.
    if (icon) return;
    let cancelled = false;
    detected = null;
    loadProjectIcon(id).then((url) => {
      if (!cancelled) detected = url;
    });
    return () => {
      cancelled = true;
    };
  });

  const src = $derived(icon ?? detected);
  // Stack glyph fills ~62% of the tile, leaving an even margin on the surface.
  const glyphSize = $derived(Math.round(size * 0.62));
</script>

<span
  class="inline-flex items-center justify-center rounded-lg shrink-0
         overflow-hidden bg-surface-2 {cls}"
  style:width="{size}px"
  style:height="{size}px"
>
  {#if src}
    <img
      {src}
      alt={name}
      class="h-full w-full object-contain p-[2px]"
      draggable="false"
    />
  {:else}
    <StackIcon {type} size={glyphSize} />
  {/if}
</span>
