<!--
  HostMark — a small brand mark for an SSH host's environment, mirroring the
  StackIcon / DatabaseMark / LanguageMark pattern. Maps an environment id
  (cpanel, plesk, ubuntu, aws, …) to the real brand logo bundled under
  `static/hosts/<id>.svg`; ids with no licensing-clean asset fall back to a
  monogram chip, and an unknown/unset id falls back to a neutral server glyph.

  Logos sit centred on a white rounded tile (like DatabaseMark) so dark brand
  colours (Debian maroon, OVH navy, AlmaLinux black, …) stay legible on the
  dark shell. Sources + licenses: see `static/hosts/README.md`.
-->
<script lang="ts">
  interface Props {
    /** Environment id (e.g. "cpanel", "ubuntu", "aws"). Null/unknown → generic. */
    environment?: string | null;
    size?: number;
    class?: string;
  }
  let { environment = null, size = 18, class: cls = "" }: Props = $props();

  /** Ids with a real bundled logo under static/hosts/. */
  const LOGOS: Record<string, string> = {
    // Control panels
    cpanel: "/hosts/cpanel.svg",
    plesk: "/hosts/plesk.svg",
    webmin: "/hosts/webmin.svg",
    // OS distros
    ubuntu: "/hosts/ubuntu.svg",
    debian: "/hosts/debian.svg",
    alpine: "/hosts/alpine.svg",
    rhel: "/hosts/rhel.svg",
    centos: "/hosts/centos.svg",
    fedora: "/hosts/fedora.svg",
    arch: "/hosts/arch.svg",
    rocky: "/hosts/rocky.svg",
    almalinux: "/hosts/almalinux.svg",
    opensuse: "/hosts/opensuse.svg",
    // Amazon Linux's official mark is the AWS smile logo — reuse the asset.
    amazonlinux: "/hosts/aws.svg",
    // Cloud providers
    aws: "/hosts/aws.svg",
    digitalocean: "/hosts/digitalocean.svg",
    gcp: "/hosts/gcp.svg",
    azure: "/hosts/azure.svg",
    hetzner: "/hosts/hetzner.svg",
    vultr: "/hosts/vultr.svg",
    ovh: "/hosts/ovh.svg",
    contabo: "/hosts/contabo.svg",
    oraclecloud: "/hosts/oraclecloud.svg",
    // Hosting / PaaS
    godaddy: "/hosts/godaddy.svg",
    hostinger: "/hosts/hostinger.svg",
    namecheap: "/hosts/namecheap.svg",
    ionos: "/hosts/ionos.svg",
    kinsta: "/hosts/kinsta.svg",
    wpengine: "/hosts/wpengine.svg",
    cloudways: "/hosts/cloudways.svg",
    render: "/hosts/render.svg",
    heroku: "/hosts/heroku.svg",
    railway: "/hosts/railway.svg",
    flyio: "/hosts/flyio.svg",
  };

  // Monogram chip fallback for ids with no licensing-clean logo asset
  // (background, foreground, and a 1–3 char monogram).
  const CHIPS: Record<string, { bg: string; fg: string; label: string }> = {
    directadmin: { bg: "#2A6FDB", fg: "#ffffff", label: "DA" },
    cyberpanel: { bg: "#1FB57A", fg: "#ffffff", label: "Cy" },
    virtualmin: { bg: "#5B8C5A", fg: "#ffffff", label: "Vm" },
    ispconfig: { bg: "#CC0000", fg: "#ffffff", label: "IS" },
    linode: { bg: "#00A95C", fg: "#ffffff", label: "L" },
    lambdalabs: { bg: "#6020A0", fg: "#ffffff", label: "λ" },
    hostgator: { bg: "#FFB400", fg: "#1d2733", label: "HG" },
    siteground: { bg: "#3B3F93", fg: "#ffffff", label: "SG" },
    inmotion: { bg: "#C5003E", fg: "#ffffff", label: "IM" },
    bluehost: { bg: "#1E55EB", fg: "#ffffff", label: "Bh" },
    dreamhost: { bg: "#0073EC", fg: "#ffffff", label: "Dh" },
    a2hosting: { bg: "#00A340", fg: "#ffffff", label: "A2" },
  };

  // Very light brand colours (GoDaddy / WP Engine cyan) wash out on the white
  // tile; both brands set their mark on dark, so give them a dark tile.
  const DARK_TILE = new Set(["godaddy", "wpengine"]);

  const id = $derived((environment ?? "").toLowerCase());
  const logo = $derived(id ? LOGOS[id] : undefined);
  const chip = $derived(!logo && id ? CHIPS[id] : undefined);
  // Monogram font scales with the tile; longer labels shrink a touch.
  const fontSize = $derived(Math.round(size * (chip && chip.label.length >= 3 ? 0.36 : 0.46)));
</script>

{#if logo}
  <span
    class="host-mark {DARK_TILE.has(id) ? 'host-mark--dark' : ''} {cls}"
    style:width={`${size}px`}
    style:height={`${size}px`}
    title={id}
    aria-hidden="true"
  >
    <img src={logo} alt="" />
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

<style>
  .host-mark {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: #fff;
    border-radius: 26%;
    overflow: hidden;
    /* hairline ring so the white tile reads as a tile, not a glare, on dark */
    box-shadow: inset 0 0 0 1px rgba(0, 0, 0, 0.08);
  }
  .host-mark--dark {
    background: #1d2733;
    box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.1);
  }
  .host-mark img {
    width: 76%;
    height: 76%;
    object-fit: contain;
  }
</style>
