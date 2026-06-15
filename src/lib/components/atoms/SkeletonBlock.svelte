<!--
  SkeletonBlock — one animated loading placeholder bar (a `bg-surface-2` pulse).
  Compose several for a skeleton list/screen. Replaces the ad-hoc `animate-pulse`
  divs scattered across routes with one consistent shape.

  Sizing comes from the `class` prop (height, width, rounding), so a caller can
  do e.g. `<SkeletonBlock class="h-3 w-24 rounded-full" />`.
-->
<script lang="ts">
  interface Props {
    /** Tailwind sizing / extra classes; defaults to a full-width row bar. */
    class?: string;
  }
  let { class: klass = "h-4 w-full rounded" }: Props = $props();
</script>

<div class="animate-pulse bg-surface-2 {klass}" aria-hidden="true"></div>
