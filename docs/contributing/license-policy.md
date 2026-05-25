# License Policy

> This is not legal advice. If you have legal questions about licensing your contribution or using PortBay in your organisation, consult a lawyer.

---

## This repository's license

PortBay Community is licensed under **AGPL-3.0-only**.

SPDX identifier: `AGPL-3.0-only`

The license text is in [`LICENSE`](../../LICENSE) at the repo root.

We are mid-migration from Apache-2.0 to AGPL-3.0. The effective license for all current and future contributions is AGPL-3.0-only. If you encounter an older file with an Apache-2.0 header, open an issue or PR to update it.

---

## What AGPL-3.0 means for contributors

When you submit a pull request and it is merged, your contribution is licensed under AGPL-3.0-only. By signing off your commits (DCO), you certify that you have the right to contribute under that license.

AGPL-3.0 is a copyleft license. Key practical points:

- You retain copyright on your contribution.
- PortBay (and anyone who forks it) must keep the source open and provide it to users, including users who access the software over a network.
- Proprietary forks that serve the software to users over a network without releasing source are not permitted under AGPL-3.0.

The distinction between AGPL-3.0 and GPL-3.0 matters for server-side software: AGPL-3.0 treats network use as distribution, GPL-3.0 does not.

---

## SPDX header convention

Source files in this repository should carry an SPDX identifier comment near the top.

Rust files:
```rust
// SPDX-License-Identifier: AGPL-3.0-only
```

TypeScript / JavaScript files:
```ts
// SPDX-License-Identifier: AGPL-3.0-only
```

Svelte files (at the top of the `<script>` block or as an HTML comment before it):
```svelte
<!-- SPDX-License-Identifier: AGPL-3.0-only -->
```

Markdown and configuration files do not require SPDX headers, but source code files do. If you add a new source file, include the identifier.

---

## Third-party code

### Inbound license compatibility

When incorporating third-party code, the inbound license must be compatible with AGPL-3.0 as the outbound license. The following inbound licenses are acceptable:

| License | Compatible |
|---|---|
| MIT | Yes |
| BSD-2-Clause | Yes |
| BSD-3-Clause | Yes |
| Apache-2.0 | Yes (with NOTICE requirements, see below) |
| ISC | Yes |
| GPL-3.0-only | Yes (same copyleft family) |
| GPL-2.0-only | Conditional — "or later" variants are OK; "only" variants are not |
| LGPL-2.1-only | Conditional — seek a discussion before including |
| GPL-incompatible (e.g. BUSL, proprietary, CC-NC) | **No** |

If you are unsure whether a dependency's license is compatible, open a Discussion before including it.

### NOTICE file attribution

Third-party code incorporated directly into this repository (not as a package dependency) must be recorded in the [`NOTICE`](../../NOTICE) file with:

- The project name and upstream URL
- The copyright statement
- The license name
- A list of which PortBay files are derived from it and which upstream files they trace back to

See the existing Lerd entry in `NOTICE` for the expected format.

Package dependencies managed by Cargo or pnpm do not need individual entries in NOTICE — their licenses are tracked by the package managers. However, if you vendor a dependency's source directly into the repo, treat it as third-party code and add a NOTICE entry.

---

## The cloud/Pro boundary

The boundary between this AGPL-3.0 public repo and the proprietary `portbay-cloud` repo is maintained deliberately. The public app may contain client-side code that:

- Calls documented public PortBay Cloud APIs
- Verifies a signed entitlement using a **public** key embedded in the app

It must never contain:

- Proprietary cloud server implementation
- Private license-signing keys or any private key material
- Billing or Stripe server logic
- Private API endpoints or customer data
- Enterprise policy engine code

This boundary is enforced by `scripts/check-repo-boundaries.sh` and `.repo-boundary-denylist` in CI. See [docs/architecture/repo-boundaries.md](../architecture/repo-boundaries.md).

---

## Future separately-licensed packages (planned)

If a clearly-separated, generic utility developed in this project warrants its own standalone release — with no AGPL-licensed code in scope — it may be published under MIT or Apache-2.0. No such packages exist today. Any such split would require an ADR (see [decision-records.md](decision-records.md)) and maintainer agreement before work starts.

---

## Questions

For licensing questions related to contributing, post in GitHub Discussions. For security-sensitive licensing concerns, email security@portbay.app. For conduct issues, email conduct@portbay.app.
