# PortBay Governance

PortBay Community is an open-source project (AGPL-3.0-only) developed in the
open at [`portbay-app/portbay`](https://github.com/portbay-app/portbay). This
document describes how decisions are made. It is intentionally lightweight: the
project is young, and the goal is clarity, not bureaucracy.

## Roles

**Maintainers** own the project's direction and have merge rights. They review
contributions, set the roadmap, cut releases, and are the final decision-makers.
At this stage the maintainer group is small; it grows by invitation when a
consistent contributor has earned the trust of the existing maintainers.

**Contributors** are anyone who opens an issue, joins a discussion, or sends a
pull request. You do not need to be a maintainer to shape PortBay — most changes
start as a contributor's issue or proposal.

## How decisions are made

- **Everyday changes** (bug fixes, docs, small features tied to an accepted
  issue) are decided by normal pull-request review. One maintainer approval and
  green CI is enough to merge.
- **Notable changes** (new dependencies or sidecars, new Tauri capabilities,
  user-visible behavior changes, anything touching the cloud boundary) should
  start with an issue or discussion so the approach is agreed before code is
  written. See [`CONTRIBUTING.md`](./CONTRIBUTING.md).
- **Major changes** (architecture shifts, schema changes, anything that affects
  compatibility or the public API) require a short design discussion and are
  recorded as a decision record — see
  [`docs/contributing/decision-records.md`](./docs/contributing/decision-records.md).

When maintainers disagree, they seek consensus first. If consensus is not
reached, the change does not land — the status quo wins until a clear case is
made. Maintainers document the rationale for contested decisions.

## Areas that require explicit maintainer approval

Some changes carry extra weight and are never merged on a single drive-by review:

- **Licensing changes.** Any change to `LICENSE`, `NOTICE`, SPDX headers, or the
  license of a package requires explicit maintainer approval. See
  [`docs/contributing/license-policy.md`](./docs/contributing/license-policy.md).
- **Cloud/Pro boundary changes.** Anything that alters how the Community app
  talks to PortBay Cloud, or that touches the boundary guards
  (`.repo-boundary-denylist`, `scripts/check-repo-boundaries.sh`,
  `docs/architecture/repo-boundaries.md`), is a maintainer decision. The split
  between what is Community and what is Cloud/Pro is decided by the maintainers.
- **Security-sensitive changes.** Code touching authentication, certificates,
  the privileged hosts helper, entitlement verification, or process execution
  requires maintainer review. See [`SECURITY.md`](./SECURITY.md).

## Roadmap and the commercial edge

PortBay maintains a public roadmap for the Community edition. Commercial roadmap
items for PortBay Cloud/Pro are developed in the private `portbay-cloud`
repository and may remain private; the maintainers decide what is shared and
when. Community feature requests that belong on the commercial side are labeled
and redirected rather than silently closed — see
[`docs/contributing/issue-triage.md`](./docs/contributing/issue-triage.md).

## Code of Conduct

Participation is governed by the [Code of Conduct](./CODE_OF_CONDUCT.md).
Conduct concerns go to conduct@portbay.app and are handled by the maintainers.

## Changing this document

Governance evolves with the project. Proposed changes to this file go through a
pull request and require explicit maintainer approval.
