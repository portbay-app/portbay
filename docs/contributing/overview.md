# Contributing to PortBay Community

PortBay Community is an open-source local development environment manager.
This document is the starting point for contributors. Deeper guides live alongside it:

- [Development setup](development.md)
- [Issue triage](issue-triage.md)
- [Pull requests](pull-requests.md)
- [Architecture orientation](architecture.md)
- [License policy](license-policy.md)
- [Decision records](decision-records.md)

The short version of all of these is at the repo-root [CONTRIBUTING.md](../../CONTRIBUTING.md).

---

## What is welcome

| Kind | Where to start |
|---|---|
| Reproducible bug reports | Open an issue using the bug template |
| Feature requests / design discussion | Open a GitHub Discussion first |
| Documentation fixes | PR directly against `main` for small edits |
| Code on an **accepted** issue | Assign yourself or comment, then open a PR |

"Accepted" means a maintainer has labelled the issue `help wanted` or explicitly said the scope is agreed. Do not open code PRs for issues that are still in triage.

---

## What needs a discussion first

These touch architecture, scope, or the cloud boundary. Start a GitHub Discussion before writing code:

- Adding a new bundled sidecar
- Adding a new Tauri capability (`tauri:allow-*` permissions)
- Changes to the declarative project registry schema
- Any change that touches the boundary between this repo and `portbay-cloud`
- Changes to the licensing or DCO terms
- Large refactors that span multiple modules

Raising the discussion first costs you nothing and saves everyone from a rejected PR.

---

## What must never be submitted

The following do not belong in this repository under any circumstances:

- Proprietary Pro or Cloud server implementation (that lives in `portbay-app/portbay-cloud`)
- Private license-signing keys, API keys, or secrets of any kind
- Billing or Stripe integration server code
- Customer data, org data, or any data from a real PortBay Cloud tenant
- Private cloud endpoints or team-sync server logic
- Enterprise policy engine code
- Anything that would expose a private key or secret even inside a test fixture

The CI guard at `scripts/check-repo-boundaries.sh` (denylist: `.repo-boundary-denylist`) will catch many of these automatically, but it is not exhaustive. See [docs/architecture/repo-boundaries.md](../architecture/repo-boundaries.md) and [license-policy.md](license-policy.md) for the full boundary specification.

---

## Developer Certificate of Origin (DCO)

PortBay uses the **Developer Certificate of Origin** — not a CLA. There is no form to sign.

Every commit you contribute must carry a sign-off line:

```
Signed-off-by: Your Name <you@example.com>
```

Add it automatically with:

```bash
git commit -s -m "feat(dns): improve wildcard resolution for .test domains"
```

The sign-off certifies:

> By contributing to PortBay, you certify that you have the right to submit the contribution and that it can be licensed under the repository's applicable license (AGPL-3.0-only).

The full DCO text is in [DCO.md](../../DCO.md) at the repo root.
Governance terms are in [GOVERNANCE.md](../../GOVERNANCE.md).

---

## Earning Pro via a merged pull request

PortBay's Pro license is available two ways: a donation, or a merged qualifying pull request.

When a qualifying PR you authored is merged, PortBay issues a perpetual Pro license tied to your GitHub account — no payment required. "Qualifying" means a non-trivial accepted change: a real fix or feature, not a typo or whitespace PR.

Maintainers confirm qualification at merge time. Details are in [pull-requests.md](pull-requests.md).

---

## Entry points

- **Good first issues:** issues labelled [`good first issue`](https://github.com/portbay-app/portbay/labels/good%20first%20issue)
- **Help wanted:** issues labelled [`help wanted`](https://github.com/portbay-app/portbay/labels/help%20wanted)
- **Discussions:** [github.com/portbay-app/portbay/discussions](https://github.com/portbay-app/portbay/discussions) — design questions, feature proposals, setup help

If you are not sure where to start, open a Discussion.
