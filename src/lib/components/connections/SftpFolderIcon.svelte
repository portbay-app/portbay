<!--
  SftpFolderIcon — the three-layer "3D" folder icon (ported from BookSlash's
  FolderCards): a colored back wall with a tab, a white paper sheet, and a
  front flap hinged at the bottom. When an ancestor `.group` is hovered the
  flap tilts open and the paper springs up with a small overshoot — desktop
  pointers only, and disabled entirely under prefers-reduced-motion.
-->
<script lang="ts">
  let { size = 34 }: { size?: number } = $props();
  const h = $derived(Math.round(size * (26 / 34)));
</script>

<span aria-hidden="true" class="folder" style="width: {size}px; height: {h}px">
  <span class="back"></span>
  <span class="paper"></span>
  <span class="front"></span>
</span>

<style>
  .folder {
    position: relative;
    display: inline-block;
    flex: none;
    perspective: 120px;
  }

  /* Back wall + tab. */
  .back {
    position: absolute;
    inset: 0;
    top: 3px;
    border-radius: 4px;
    background: linear-gradient(
      180deg,
      color-mix(in oklab, var(--color-accent), white 22%),
      var(--color-accent)
    );
  }
  .back::before {
    content: "";
    position: absolute;
    top: -3px;
    left: 2px;
    height: 6px;
    width: 41%;
    border-radius: 3px 3px 0 0;
    background: inherit;
  }

  /* The paper sheet that springs up out of the folder on hover. */
  .paper {
    position: absolute;
    inset: 5px 4px 3px;
    border-radius: 2px;
    background:
      repeating-linear-gradient(
        180deg,
        transparent 0 5px,
        color-mix(in oklab, var(--color-border), transparent 30%) 5px 6px
      ),
      linear-gradient(180deg, #fff, #e8eaef);
    opacity: 0;
    transform: translateY(2px) scale(0.94);
    transition:
      transform 460ms cubic-bezier(0.34, 1.56, 0.64, 1) 80ms,
      opacity 180ms ease-out 60ms;
  }

  /* Front flap, hinged at the bottom. */
  .front {
    position: absolute;
    inset: 0;
    top: 38%;
    border-radius: 3px 3px 4px 4px;
    background: linear-gradient(
      180deg,
      color-mix(in oklab, var(--color-accent), white 14%),
      color-mix(in oklab, var(--color-accent), black 8%)
    );
    transform-origin: 50% 100%;
    transform: rotateX(0deg);
    backface-visibility: hidden;
    transition: transform 240ms cubic-bezier(0.23, 1, 0.32, 1);
    box-shadow: 0 -0.5px 0 color-mix(in oklab, var(--color-accent), white 30%) inset;
  }

  /* Hover (via ancestor .group), desktop pointers only. */
  @media (hover: hover) and (pointer: fine) {
    :global(.group:hover) .front {
      transform: rotateX(-30deg);
    }
    :global(.group:hover) .paper {
      opacity: 1;
      transform: translateY(-7px) rotate(-3deg) scale(1);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .paper,
    .front {
      transition: none;
    }
    :global(.group:hover) .front {
      transform: none;
    }
    :global(.group:hover) .paper {
      opacity: 0;
      transform: translateY(2px) scale(0.94);
    }
  }
</style>
