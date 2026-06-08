<!--
  ThemeImage — a docs screenshot that inverts against the page theme.

  On the DARK docs theme we show the light-framed capture (`<name>.png`); on the
  LIGHT theme we show the dark-framed capture (`<name>-dark.png`), so the macOS
  window frame always contrasts with the page behind it. Both <img>s are
  rendered and the visible one is chosen by CSS keyed on `html.dark` (set by
  VitePress before first paint), so there is no hydration flash.

  Usage:  <ThemeImage name="projects" alt="PortBay managing local projects" />
-->
<script setup lang="ts">
import { withBase } from "vitepress";

defineProps<{
  /** Screenshot basename, without extension or `-dark` suffix. */
  name: string;
  alt?: string;
  width?: string | number;
}>();
</script>

<template>
  <img
    class="theme-image theme-image--light"
    :src="withBase(`/screenshots/${name}.png`)"
    :alt="alt ?? ''"
    :width="width"
    loading="lazy"
    decoding="async"
  />
  <img
    class="theme-image theme-image--dark"
    :src="withBase(`/screenshots/${name}-dark.png`)"
    :alt="alt ?? ''"
    :width="width"
    loading="lazy"
    decoding="async"
  />
</template>
