<!--
  ProjectAvatar — square gradient + initial(s) for a project.

  The gradient is deterministic from the project id (FNV-1a → hue),
  so the same project always renders the same colour, but no two
  projects in a typical list will collide visually.

  This is the fallback for projects without a user-supplied icon.
  When we add an `iconPath` field to ProjectView, the caller can
  pass `icon` to override.
-->
<script lang="ts">
  interface Props {
    /** Stable identifier used to derive the hue. */
    id: string;
    /** Display name — first 1–2 letters of the first two words. */
    name: string;
    size?: number;
    /** Optional pre-rendered icon URL. When set, overrides the gradient. */
    icon?: string | null;
    class?: string;
  }
  let { id, name, size = 28, icon = null, class: cls = "" }: Props = $props();

  function hashHue(input: string): number {
    let h = 2166136261;
    for (let i = 0; i < input.length; i++) {
      h ^= input.charCodeAt(i);
      h = Math.imul(h, 16777619);
    }
    // Map to 0–360 — biased toward saturated mid-hues by hashing twice.
    return Math.abs(h) % 360;
  }

  const initials = $derived.by<string>(() => {
    const parts = name.trim().split(/\s+/).slice(0, 2);
    if (parts.length === 0) return "?";
    const first = parts[0]?.[0] ?? "";
    const second = parts[1]?.[0] ?? "";
    return (first + second || first || "?").toUpperCase();
  });

  const gradient = $derived.by(() => {
    const h = hashHue(id || name);
    const h2 = (h + 35) % 360;
    return `linear-gradient(135deg, hsl(${h} 65% 52%) 0%, hsl(${h2} 70% 42%) 100%)`;
  });

  const fontSize = $derived(Math.max(10, Math.round(size * 0.4)));
</script>

{#if icon}
  <img
    src={icon}
    alt=""
    width={size}
    height={size}
    class="rounded-lg object-cover shrink-0 {cls}"
    style:width="{size}px"
    style:height="{size}px"
  />
{:else}
  <span
    class="inline-flex items-center justify-center rounded-lg shrink-0
           text-white font-semibold tracking-tight {cls}"
    style:width="{size}px"
    style:height="{size}px"
    style:background={gradient}
    style:font-size="{fontSize}px"
    aria-hidden="true"
  >
    {initials}
  </span>
{/if}
