/**
 * Display labels for the provider/OS `environment` id used by the host
 * dashboard's "Provider / Region" column. Mirrors the chip set in
 * `HostMark.svelte`; an unknown id falls back to a Title-cased form of itself.
 */
const PROVIDER_LABELS: Record<string, string> = {
  // Cloud providers
  aws: "AWS",
  digitalocean: "DigitalOcean",
  gcp: "Google Cloud",
  azure: "Azure",
  hetzner: "Hetzner",
  linode: "Linode",
  lambdalabs: "Lambda Labs",
  vultr: "Vultr",
  ovh: "OVHcloud",
  contabo: "Contabo",
  oraclecloud: "Oracle Cloud",
  amazonlinux: "Amazon Linux",
  // Hosting / PaaS
  godaddy: "GoDaddy",
  hostinger: "Hostinger",
  hostgator: "HostGator",
  siteground: "SiteGround",
  inmotion: "InMotion Hosting",
  bluehost: "Bluehost",
  dreamhost: "DreamHost",
  a2hosting: "A2 Hosting",
  namecheap: "Namecheap",
  ionos: "IONOS",
  kinsta: "Kinsta",
  wpengine: "WP Engine",
  cloudways: "Cloudways",
  render: "Render",
  heroku: "Heroku",
  railway: "Railway",
  flyio: "Fly.io",
  // Control panels
  cpanel: "cPanel",
  plesk: "Plesk",
  directadmin: "DirectAdmin",
  cyberpanel: "CyberPanel",
  webmin: "Webmin",
  virtualmin: "Virtualmin",
  ispconfig: "ISPConfig",
  // OS distros
  ubuntu: "Ubuntu",
  debian: "Debian",
  alpine: "Alpine",
  rhel: "RHEL",
  centos: "CentOS",
  fedora: "Fedora",
  arch: "Arch",
  rocky: "Rocky Linux",
  almalinux: "AlmaLinux",
  opensuse: "openSUSE",
};

/** Human label for a provider/OS id, or null when none is set. */
export function providerLabel(environment: string | null | undefined): string | null {
  const id = (environment ?? "").trim().toLowerCase();
  if (!id) return null;
  return PROVIDER_LABELS[id] ?? id.charAt(0).toUpperCase() + id.slice(1);
}
