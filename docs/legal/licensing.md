# Licensing

PortBay Community is free and open-source software licensed under the
**GNU Affero General Public License v3.0 only** (`AGPL-3.0-only`). The binding
text is in [`LICENSE`](../../LICENSE) at the repository root.

> This page explains how licensing is organized in this repository. It is a
> summary for contributors and users — **it is not legal advice.**

## The one rule

The PortBay Community application — the desktop app, the CLI, the Rust core, the
daemon and reconciler, the service/process manager, the reverse-proxy
integration, the local DNS and HTTPS management, and the project-orchestration
logic — is licensed **AGPL-3.0-only**. If you distribute PortBay or run a
modified, network-accessible version of it, the AGPL's terms apply (see
[the plain-English explainer](../pages/license.md)).

PortBay Cloud and Pro are **separate commercial software** developed in the
private `portbay-cloud` repository. They are not part of this repository and are
not covered by this license. See
[repo boundaries](../architecture/repo-boundaries.md).

## License map

This repository is a single application, not a multi-package monorepo, so almost
everything is AGPL-3.0-only. The table below maps the tree as it exists today.

| Path | License | Notes |
|---|---|---|
| `src/` | AGPL-3.0-only | SvelteKit desktop UI (the GUI client). |
| `src-tauri/` | AGPL-3.0-only | Rust core: daemon, reconciler, sidecar adapters, CLI binaries, IPC commands. |
| `scripts/` | AGPL-3.0-only | Build and sidecar-fetch scripts. |
| `docs/`, `docs-site/` | AGPL-3.0-only | Documentation (see "Documentation license" below). |
| `src/lib/components/atoms/*` (Lerd-derived) | MIT (upstream), incorporated into the AGPL whole | Adapted from [Lerd](https://github.com/geodro/lerd); attributed in [`NOTICE`](../../NOTICE). MIT is inbound-compatible with AGPL-3.0. |
| Bundled sidecar binaries (Caddy, dnsmasq, mkcert, Process Compose, cloudflared, mailpit) | Each under its own upstream license | Not committed to this repo (gitignored, fetched at build). Their licenses ship with distributed builds and are credited in `NOTICE`. |

### Reserved for future use

These files **do not exist yet**. They are reserved so that *if* PortBay ever
extracts a genuinely separable, generic package — a plugin SDK, public API type
definitions, or a reusable UI library with no PortBay-specific logic — it can be
released under a permissive license to maximize reuse:

| Path (hypothetical) | Intended license | Notes |
|---|---|---|
| `packages/plugin-sdk/` | Apache-2.0 (preferred) | Public plugin interface only; no core logic. Apache-2.0 for its patent grant. |
| `packages/api-types/` | Apache-2.0 or MIT | Public API/schema type definitions that reveal no private implementation. |
| `packages/ui/` | MIT or Apache-2.0 | Generic, PortBay-agnostic UI components only. |

When (and only when) such a package is created, add a `LICENSE-APACHE` or
`LICENSE-MIT` at its root, an SPDX header on its files, and a row to this table.
Until then there is a single `LICENSE` (AGPL-3.0-only) and no permissive
sub-licensing. **We do not dual-license anything.**

## SPDX headers

First-party source files should carry an SPDX identifier near the top:

```rust
// SPDX-License-Identifier: AGPL-3.0-only
```

```svelte
<!-- SPDX-License-Identifier: AGPL-3.0-only -->
```

New files added in a pull request are expected to include this header. A
repo-wide backfill of existing files is tracked separately so it doesn't collide
with in-flight work; see
[`docs/contributing/license-policy.md`](../contributing/license-policy.md).

## Documentation license

Documentation under `docs/` and `docs-site/` is currently covered by the
repository license (AGPL-3.0-only) for simplicity — one license, one mental
model. The common alternative for prose is **CC BY 4.0**, which is friendlier
for people who want to quote or adapt the docs outside a GPL-compatible context.
We recommend staying single-license (AGPL-3.0-only) until there's concrete
demand for documentation reuse; adding a second license has real maintenance
and clarity costs that aren't justified pre-1.0. If that demand appears,
relicensing only the `docs/` and `docs-site/` prose to CC BY 4.0 is a clean,
isolated change.

## Third-party code

Incoming third-party code must be license-compatible with AGPL-3.0-only
(MIT, BSD, ISC, Apache-2.0, and public-domain code are inbound-compatible) and
must be recorded in [`NOTICE`](../../NOTICE) with attribution. Maintainers
verify this on review — see
[`docs/maintainers/license-review.md`](../maintainers/license-review.md).

---

*This document is informational and not legal advice. The `LICENSE` file is the
authoritative legal text.*
