<!--
  FnKey — a small inline keycap rendering the macOS Fn key, for copy that
  tells the user to "hold Fn". Replaces the globe (🌐) emoji we used before:
  the globe reads as a mobile/web glyph, not a keyboard key. The real key is
  labelled "fn" (lowercase) on Apple keyboards, so that is what we draw.

  Inline by design — `align-middle` keeps it sitting on the text baseline of a
  surrounding sentence. Matches the ⌘K keycap in the top bar's search button.
-->
<script lang="ts">
  let { class: klass = "" }: { class?: string } = $props();
</script>

<kbd
  class="inline-flex items-center justify-center align-middle h-[16px] min-w-[18px] px-1
         rounded border border-border/60 bg-bg/40 text-[10px] font-mono leading-none
         text-fg-subtle select-none {klass}"
>fn</kbd>
