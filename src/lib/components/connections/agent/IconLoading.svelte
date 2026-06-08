<!--
  IconLoading — Svelte port of Void's IconLoading (SidebarChat.tsx). A cycling
  ellipsis ("." → ".." → "..." every 300ms). Void uses this everywhere it's
  "working": the thinking placeholder, the Reasoning header while writing, and
  running tool rows. Deliberately a text ellipsis, not a spinner/bouncing dots.
-->
<script lang="ts">
  let { class: cls = "" }: { class?: string } = $props();

  let dots = $state(".");

  $effect(() => {
    const id = setInterval(() => {
      dots = dots.length >= 3 ? "." : dots + ".";
    }, 300);
    return () => clearInterval(id);
  });
</script>

<span class={cls}>{dots}</span>
