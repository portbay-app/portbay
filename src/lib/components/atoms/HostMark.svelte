<!--
  HostMark — a small brand mark for an SSH host's environment, mirroring the
  StackIcon / DatabaseMark / LanguageMark pattern. Maps an environment id
  (cpanel, plesk, ubuntu, aws, …) to a compact branded chip; an unknown/unset
  id falls back to a neutral server glyph.

  MVP ships recognizable monogram chips (brand colour + initials) so no
  third-party logo assets are bundled. To swap in a real logo later, drop
  `/static/hosts/<id>.svg` and render it here for that id — callers don't change.
-->
<script lang="ts">
  interface Props {
    /** Environment id (e.g. "cpanel", "ubuntu", "aws"). Null/unknown → generic. */
    environment?: string | null;
    size?: number;
    class?: string;
  }
  let { environment = null, size = 18, class: cls = "" }: Props = $props();

  // Per-id chip: background, foreground, and a 1–3 char monogram.
  const CHIPS: Record<string, { bg: string; fg: string; label: string }> = {
    // Control panels
    cpanel: { bg: "#FF6C2C", fg: "#ffffff", label: "cP" },
    plesk: { bg: "#53BCE6", fg: "#0b2b3a", label: "Pl" },
    directadmin: { bg: "#2A6FDB", fg: "#ffffff", label: "DA" },
    cyberpanel: { bg: "#1FB57A", fg: "#ffffff", label: "Cy" },
    webmin: { bg: "#7A9CC6", fg: "#10243d", label: "Wm" },
    // OS distros
    ubuntu: { bg: "#E95420", fg: "#ffffff", label: "U" },
    debian: { bg: "#A80030", fg: "#ffffff", label: "Db" },
    alpine: { bg: "#0D597F", fg: "#ffffff", label: "Al" },
    rhel: { bg: "#EE0000", fg: "#ffffff", label: "RH" },
    centos: { bg: "#932279", fg: "#ffffff", label: "C" },
    fedora: { bg: "#51A2DA", fg: "#ffffff", label: "F" },
    amazonlinux: { bg: "#FF9900", fg: "#15212e", label: "AL" },
    arch: { bg: "#1793D1", fg: "#ffffff", label: "Ar" },
    // Cloud providers
    aws: { bg: "#FF9900", fg: "#15212e", label: "AWS" },
    digitalocean: { bg: "#0080FF", fg: "#ffffff", label: "DO" },
    gcp: { bg: "#4285F4", fg: "#ffffff", label: "G" },
    azure: { bg: "#0078D4", fg: "#ffffff", label: "Az" },
    hetzner: { bg: "#D50C2D", fg: "#ffffff", label: "H" },
    linode: { bg: "#00A95C", fg: "#ffffff", label: "L" },
    lambdalabs: { bg: "#6020A0", fg: "#ffffff", label: "λ" },
  };

  const id = $derived((environment ?? "").toLowerCase());
  const chip = $derived(id ? CHIPS[id] : undefined);
  // Monogram font scales with the tile; longer labels shrink a touch.
  const fontSize = $derived(Math.round(size * (chip && chip.label.length >= 3 ? 0.36 : 0.46)));
</script>

{#if chip}
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
  <!-- Generic / unknown: a neutral server glyph. -->
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    class={cls}
    aria-hidden="true"
  >
    <rect x="3" y="4" width="18" height="7" rx="1.6" stroke="currentColor" stroke-width="1.6" />
    <rect x="3" y="13" width="18" height="7" rx="1.6" stroke="currentColor" stroke-width="1.6" />
    <circle cx="7" cy="7.5" r="1" fill="currentColor" />
    <circle cx="7" cy="16.5" r="1" fill="currentColor" />
  </svg>
{/if}
