# Software Bill of Materials (SBOM)

PortBay publishes a complete **Software Bill of Materials** with every signed
release. It is a machine-readable inventory of every third-party component the
app ships or downloads — so your security team can audit dependencies, match
them against vulnerability feeds, and satisfy supply-chain requirements
(NTIA minimum elements, EO 14028, CRA).

## What the SBOM covers

The release SBOM is a single **CycloneDX 1.5** JSON document
(`portbay-sbom.cdx.json`) that merges three ecosystems into one file:

| Source | Tool | Scope |
|---|---|---|
| **Rust crates** | `cargo-cyclonedx` | The full `src-tauri/Cargo.lock` dependency tree behind the desktop app and its native sidecars. |
| **npm packages** | `@cyclonedx/cyclonedx-npm` | The SvelteKit front-end's production dependency tree. |
| **Bundled non-Rust components** | generated component list | The pinned third-party sidecar binaries (Caddy, cloudflared, dnsmasq, mailpit, mkcert, process-compose) and the on-device AI / speech / image model weights PortBay ships or downloads — none of which appear in the cargo or npm trees. |

Each component carries its name, version, license, and a package URL (`purl`)
where one exists, so the document is consumable by standard SCA tooling
(Dependency-Track, Grype, Trivy, and others).

## Where to get it

The SBOM is attached as a release asset to every signed build:

- **Latest release:**
  [github.com/portbay-app/portbay/releases/latest](https://github.com/portbay-app/portbay/releases/latest)
  → download `portbay-sbom.cdx.json`.

Because the SBOM is regenerated per release, always pull the one attached to the
exact version you are evaluating — component versions change between releases.

## AI model provenance

The SBOM lists the bundled and downloadable AI models as components, but their
**licenses, sources, and redistribution terms** are documented in full in the
[Third-Party Model Manifest](./model-manifest), with a machine-readable
companion (`THIRD-PARTY-MODELS.json`). Use the two together when your review
needs model-level license clarity.

## How it is produced

The merged document is built by `scripts/sbom-merge.mjs` and emitted during the
release workflow (`.github/workflows/release.yml`), so it always reflects the
exact commit that produced the binaries — it is not maintained by hand. See the
[Security Whitepaper](./whitepaper) §6 (Application hardening) for how the SBOM
fits into PortBay's supply-chain controls, and the
[Security Policy](./policy) for how to report a vulnerability you find in a
listed component.
