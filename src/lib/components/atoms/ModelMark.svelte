<!--
  ModelMark — a small brand mark for an AI model family, mirroring the
  HostMark / StackIcon pattern. Maps a catalog family id (qwen25, llama,
  deepseek, …) to the vendor logo bundled under `static/ai/<vendor>.svg`;
  ids with no licensing-clean asset fall back to a monogram chip, and the
  functional groupings (embeddings, vision) to a neutral Icon glyph.

  Logos sit centred on a white rounded tile (like HostMark) so dark brand
  marks (Ollama, Moonshot black) stay legible on the dark shell. Sources +
  licenses: see `static/ai/README.md`.
-->
<script lang="ts">
  import Icon, { type IconName } from "$lib/components/atoms/Icon.svelte";

  interface Props {
    /** Catalog family id (e.g. "qwen25", "llama", "other"). */
    family?: string | null;
    size?: number;
    class?: string;
  }
  let { family = null, size = 18, class: cls = "" }: Props = $props();

  /** Ids with a real bundled vendor logo under static/ai/. */
  const LOGOS: Record<string, string> = {
    qwen25: "/ai/qwen.svg",
    qwen3: "/ai/qwen.svg",
    llama: "/ai/meta.svg",
    deepseek: "/ai/deepseek.svg",
    kimi: "/ai/moonshot.svg",
    gemma: "/ai/google.svg",
    mistral: "/ai/mistral.svg",
    exaone: "/ai/lg.svg",
    other: "/ai/ollama.svg",
  };

  // Functional groupings (not brands) render a neutral glyph.
  const ICONS: Record<string, IconName> = {
    embeddings: "search",
    vision: "eye",
  };

  // Monogram chip fallback for vendors with no licensing-clean logo asset
  // (Simple Icons dropped Microsoft marks, so Phi gets a chip).
  const CHIPS: Record<string, { bg: string; fg: string; label: string }> = {
    phi: { bg: "#0078D4", fg: "#ffffff", label: "Phi" },
  };

  const id = $derived((family ?? "").toLowerCase());
  const logo = $derived(id ? LOGOS[id] : undefined);
  const icon = $derived(!logo && id ? ICONS[id] : undefined);
  const chip = $derived(!logo && !icon && id ? CHIPS[id] : undefined);
  const fontSize = $derived(Math.round(size * (chip && chip.label.length >= 3 ? 0.36 : 0.46)));
</script>

{#if logo}
  <span
    class="model-mark {cls}"
    style:width={`${size}px`}
    style:height={`${size}px`}
    title={id}
    aria-hidden="true"
  >
    <img src={logo} alt="" />
  </span>
{:else if icon}
  <span
    class="inline-grid place-items-center rounded-[26%] bg-surface-2 text-fg-muted {cls}"
    style:width={`${size}px`}
    style:height={`${size}px`}
    title={id}
    aria-hidden="true"
  >
    <Icon name={icon} size={Math.round(size * 0.62)} />
  </span>
{:else if chip}
  <span
    class="inline-grid place-items-center rounded-[26%] font-semibold leading-none {cls}"
    style:width={`${size}px`}
    style:height={`${size}px`}
    style:background-color={chip.bg}
    style:color={chip.fg}
    style:font-size={`${fontSize}px`}
    title={id}
    aria-hidden="true"
  >
    {chip.label}
  </span>
{:else}
  <!-- Unknown family: a neutral box glyph. -->
  <span
    class="inline-grid place-items-center rounded-[26%] bg-surface-2 text-fg-muted {cls}"
    style:width={`${size}px`}
    style:height={`${size}px`}
    aria-hidden="true"
  >
    <Icon name="package" size={Math.round(size * 0.62)} />
  </span>
{/if}

<style>
  .model-mark {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: #fff;
    border-radius: 26%;
    overflow: hidden;
    /* hairline ring so the white tile reads as a tile, not a glare, on dark */
    box-shadow: inset 0 0 0 1px rgba(0, 0, 0, 0.08);
  }
  .model-mark img {
    width: 72%;
    height: 72%;
    object-fit: contain;
  }
</style>
