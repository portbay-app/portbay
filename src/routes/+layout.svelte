<!--
  Root layout — the app shell.

  CSS grid: [sidebar 220px] [main 1fr] [rail 320px], with a 56px top bar
  spanning across the main + rail tracks. Sidebar covers the full window
  height so the macOS traffic lights (overlaid via tauri.conf.json's
  titleBarStyle: "Overlay") sit cleanly inside it — matching the
  screenshot's ServBay-style chrome.

  In `compact` density the right rail is hidden, freeing horizontal room
  for denser project tables (the screenshot's "websites" table at 6 rows
  comfortably fits at compact).
-->
<script lang="ts">
  import "../app.css";
  import type { Snippet } from "svelte";
  import { Sidebar, TopBar, RightRail } from "$lib/components/shell";
  import { density } from "$lib/stores/density";

  let { children }: { children: Snippet } = $props();

  // grid-template-columns chosen for the screenshot's proportions:
  //   sidebar  220px (180px in compact)
  //   main     1fr (greedy)
  //   rail     320px (collapses to 0 in compact)
  const gridCols = $derived(
    density.value === "compact" ? "180px 1fr 0px" : "220px 1fr 320px",
  );
</script>

<div
  class="h-screen w-screen grid grid-rows-[1fr] overflow-hidden"
  style:grid-template-columns={gridCols}
>
  <Sidebar />

  <div class="flex flex-col min-w-0 min-h-0">
    <TopBar />
    <main class="flex-1 min-h-0 overflow-y-auto bg-bg">
      {@render children()}
    </main>
  </div>

  <RightRail />
</div>
