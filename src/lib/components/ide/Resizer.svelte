<!--
  Resizer — a thin drag handle for the IDE's sidebar / bottom-panel dividers.
  Hand-rolled (no dep), mirroring oxideterm's mousedown divider: on press it
  captures the starting size + pointer position, then on each move computes the
  new size and hands it to `set`. `invert` is for the bottom panel, where
  dragging *up* should *increase* height.

  `axis="x"` is a vertical bar you drag horizontally (sidebar width);
  `axis="y"` is a horizontal bar you drag vertically (panel height).
-->
<script lang="ts">
  interface Props {
    axis: "x" | "y";
    /** Current size in px. */
    value: number;
    /** Commit a new size in px (the store clamps). */
    set: (px: number) => void;
    /** Drag direction is reversed (bottom panel grows as you drag up). */
    invert?: boolean;
    "aria-label"?: string;
  }
  let { axis, value, set, invert = false, "aria-label": ariaLabel }: Props = $props();

  let dragging = $state(false);

  function onPointerDown(e: PointerEvent) {
    e.preventDefault();
    dragging = true;
    const startPos = axis === "x" ? e.clientX : e.clientY;
    const startSize = value;
    const sign = invert ? -1 : 1;

    const move = (ev: PointerEvent) => {
      const pos = axis === "x" ? ev.clientX : ev.clientY;
      set(startSize + sign * (pos - startPos));
    };
    const up = () => {
      dragging = false;
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      document.body.style.removeProperty("cursor");
      document.body.style.removeProperty("user-select");
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    document.body.style.cursor = axis === "x" ? "col-resize" : "row-resize";
    document.body.style.userSelect = "none";
  }
</script>

<div
  role="separator"
  aria-label={ariaLabel}
  aria-orientation={axis === "x" ? "vertical" : "horizontal"}
  onpointerdown={onPointerDown}
  class="group relative shrink-0 {axis === 'x'
    ? 'w-px cursor-col-resize'
    : 'h-px cursor-row-resize'} bg-border/70 hover:bg-accent/60 {dragging ? 'bg-accent' : ''}"
>
  <!-- Invisible wider hit area so the 1px line is easy to grab. -->
  <span
    class="absolute {axis === 'x'
      ? 'inset-y-0 -left-1 -right-1'
      : 'inset-x-0 -top-1 -bottom-1'}"
  ></span>
</div>
