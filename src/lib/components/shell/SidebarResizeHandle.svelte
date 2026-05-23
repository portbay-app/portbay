<!--
  SidebarResizeHandle — vertical strip on the sidebar's right edge.

  Pointer drag updates `sidebar.width` 1:1 (RAF-throttled). Double-click
  resets to the default. Arrow keys nudge by `SIDEBAR_KEY_STEP` so
  keyboard users can resize without a mouse.

  Sits on top of the grid gutter; the layout's grid-template-columns
  reads from `sidebar.width` so the rest of the chrome reflows
  automatically. While dragging, body cursor is forced to `col-resize`
  so the user gets correct feedback even when the pointer slips off
  the handle.
-->
<script lang="ts">
  import { sidebar, SIDEBAR_KEY_STEP } from "$lib/stores/sidebar.svelte";

  let handleEl: HTMLDivElement | null = $state(null);
  let rafPending = false;
  let pendingWidth = sidebar.width;

  function flush() {
    rafPending = false;
    sidebar.set(pendingWidth);
  }

  function onPointerDown(e: PointerEvent) {
    if (e.button !== 0 || !handleEl) return;
    e.preventDefault();
    handleEl.setPointerCapture(e.pointerId);
    sidebar.beginDrag();
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
  }

  function onPointerMove(e: PointerEvent) {
    if (!sidebar.dragging) return;
    pendingWidth = e.clientX;
    if (!rafPending) {
      rafPending = true;
      requestAnimationFrame(flush);
    }
  }

  function onPointerUp(e: PointerEvent) {
    if (!sidebar.dragging || !handleEl) return;
    handleEl.releasePointerCapture(e.pointerId);
    sidebar.endDrag();
    document.body.style.cursor = "";
    document.body.style.userSelect = "";
  }

  function onKeyDown(e: KeyboardEvent) {
    switch (e.key) {
      case "ArrowLeft":
        e.preventDefault();
        sidebar.nudge(-SIDEBAR_KEY_STEP);
        break;
      case "ArrowRight":
        e.preventDefault();
        sidebar.nudge(SIDEBAR_KEY_STEP);
        break;
      case "Home":
        e.preventDefault();
        sidebar.reset();
        break;
    }
  }
</script>

<!--
  Resize handles between panes are conventionally `role="separator"`
  with tabindex and arrow-key handlers, per WAI-ARIA Authoring
  Practices §3.27. The linter's "non-interactive" classification
  doesn't account for that pattern, so both warnings are suppressed.
-->
<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
  bind:this={handleEl}
  role="separator"
  aria-orientation="vertical"
  aria-label="Resize sidebar"
  aria-valuemin={160}
  aria-valuemax={360}
  aria-valuenow={sidebar.width}
  tabindex="0"
  class="absolute top-0 right-0 h-full w-1 z-10 cursor-col-resize
         hover:bg-accent/40 focus-visible:bg-accent/60
         outline-none transition-colors"
  class:bg-accent={sidebar.dragging}
  class:bg-transparent={!sidebar.dragging}
  onpointerdown={onPointerDown}
  onpointermove={onPointerMove}
  onpointerup={onPointerUp}
  onpointercancel={onPointerUp}
  ondblclick={() => sidebar.reset()}
  onkeydown={onKeyDown}
></div>
