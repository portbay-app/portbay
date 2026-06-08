# Host provider / OS brand marks

Logos rendered by `src/lib/components/atoms/HostMark.svelte` for an SSH host's
detected (or manually set) `environment` / `provider` id. All assets are
bundled — the desktop app never hotlinks logo CDNs.

## Sources & licenses

**Simple Icons** (<https://simpleicons.org>, CC0 1.0 — public domain). Exported
from the `simple-icons` npm package; each glyph is tinted with the brand colour
Simple Icons publishes for it (`fill="#…"` stamped on the root `<svg>`):

`plesk` `webmin` `ubuntu` `debian` `alpine` `rhel` (Red Hat fedora
mark) `centos` `fedora` `arch` `rocky` `almalinux` `opensuse` `digitalocean`
`gcp` `hetzner` `vultr` `ovh` `contabo` `godaddy` `hostinger` `namecheap`
`ionos` `kinsta` `wpengine` `cloudways` `render` `railway` `flyio`

**Official brand assets.** Copied verbatim from the vendor's own site
(nominative use — identifies the service a host runs):

- `cpanel.svg` — the orange "cP" logomark from
  <https://www.cpanel.net/wp-content/uploads/2025/06/brand-cP-logomark.svg>
  (replaces the retired Simple Icons wordmark glyph)

**Devicon** (<https://devicon.dev>, MIT — © 2015 konpa and contributors).
Copied verbatim for brands Simple Icons no longer carries:

- `aws.svg` — `amazonwebservices-original-wordmark.svg` (also used for the
  `amazonlinux` id; Amazon Linux's official mark is the AWS logo)
- `azure.svg` — `azure-original.svg`
- `oraclecloud.svg` — `oracle-original.svg`
- `heroku.svg` — `heroku-original.svg`

## Ids with no asset (monogram chip fallback in HostMark)

`directadmin` `cyberpanel` `virtualmin` `ispconfig` `linode` `lambdalabs`
`hostgator` `siteground` `inmotion` `bluehost` `dreamhost` `a2hosting` —
no licensing-clean vector found in CC0/MIT icon sets at the time of writing.
If one lands here later as `<id>.svg`, add it to the `LOGOS` map in
`HostMark.svelte` and drop the chip entry.

All trademarks and logos remain the property of their respective owners; they
identify the third-party services a host runs, not affiliation or endorsement.
