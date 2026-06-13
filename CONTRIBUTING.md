# Contributing to PortBay

PortBay is a native local-development manager built with Tauri 2, Rust, and Svelte 5. Contributions should be small, traceable, and tied to an accepted issue.

PortBay Community is open source under **AGPL-3.0-only**. By contributing you
agree your work is licensed under that license (see [Signing off](#signing-off-dco)).

**Deeper guides** live in [`docs/contributing/`](./docs/contributing/overview.md):
[development setup](./docs/contributing/development.md) ·
[build model](./docs/contributing/build-model.md) ·
[issue triage](./docs/contributing/issue-triage.md) ·
[pull requests](./docs/contributing/pull-requests.md) ·
[architecture](./docs/contributing/architecture.md) ·
[license policy](./docs/contributing/license-policy.md) ·
[decision records](./docs/contributing/decision-records.md). This file is the
quick start; those are the details.

## Local Setup

```bash
pnpm install
./scripts/fetch-caddy.sh
./scripts/fetch-mkcert.sh
./scripts/fetch-mailpit.sh
./scripts/fetch-cloudflared.sh
cd src-tauri && cargo test
cd .. && pnpm check
pnpm tauri dev
```

Sidecar binaries are stored under `src-tauri/binaries/` and are ignored by git.

## Signing off (DCO)

PortBay uses the [Developer Certificate of Origin](./DCO.md) — a per-commit
sign-off — instead of a CLA. Sign every commit with `-s`:

```bash
git commit -s -m "feat(hosts): add helper client"
```

This adds a `Signed-off-by:` line with your name and email.

> By contributing to PortBay, you certify that you have the right to submit the
> contribution and that it can be licensed under the repository's applicable
> license (AGPL-3.0-only).

CI checks that commits are signed off. Forgot? `git commit --amend -s --no-edit`,
or for a range, `git rebase --signoff main`.

## What must never go in this repo

PortBay Community is the public, AGPL repo. PortBay Cloud/Pro is developed
separately in the private `portbay-cloud` repo. Never submit to this repo:

- Proprietary PortBay Cloud/Pro server implementation, billing/Stripe logic,
  license-**signing** private keys, or the enterprise policy engine.
- Secrets, credentials, tokens, API keys, private endpoints, or customer data.
- Private cloud infrastructure or deployment code.

Client code that *calls* PortBay Cloud's public APIs and verifies a signed
entitlement with the **public** key is fine. The full policy is in
[repo boundaries](./docs/architecture/repo-boundaries.md), and
`scripts/check-repo-boundaries.sh` enforces it in CI. Run it before you push.

## Workflow

- Use one branch per issue.
- Keep commits atomic and conventional, for example `feat(hosts): add helper client`.
- Link the issue or kanban card in the pull request.
- Keep unrelated refactors out of feature commits.
- Do not commit generated sidecar binaries, local registries, credentials, crash reports, or build output.

## Quality Bar

Before opening a pull request:

```bash
cd src-tauri && cargo test
cd .. && pnpm check
pnpm build
```

Run narrower checks while iterating, but the full set should pass before review.

## Architecture Expectations

- Prefer existing Rust modules and Svelte stores over new abstractions.
- Keep Tauri commands behind `src-tauri/src/commands/`.
- Return structured `AppError` values instead of string errors.
- Keep filesystem writes atomic where user data is involved.
- Treat project paths, environment variables, logs, and registry contents as private user data.

## Pull Request Checklist

Confirm each of these in your PR (the
[PR template](./.github/pull_request_template.md) restates them):

- I have read CONTRIBUTING.md.
- My commits are signed off (`git commit -s`) per the [DCO](./DCO.md).
- I have **not** included proprietary PortBay Cloud/Pro code.
- I have **not** included secrets, credentials, tokens, keys, private
  endpoints, or customer data.
- I have added or updated tests where needed.
- I have updated docs where needed for user-visible behavior.
- My contribution is licensed under the repo's license (AGPL-3.0-only).
- I ran the required local checks: `cd src-tauri && cargo test`, `pnpm check`,
  `pnpm build`, and `scripts/check-repo-boundaries.sh`.
- New Tauri capabilities, sidecars, or network calls are justified and scoped.
- The PR is one coherent change, not a bundle of unrelated work.

## Before your PR can merge

`main` is protected. A pull request can only merge once **all** of these pass —
there is no way around them, so save yourself a round-trip and run the checks
locally first:

- Every commit is **signed off** (`git commit -s`) — the `DCO sign-off` check.
- All builds and checks are **green**: Rust (`fmt + clippy + test`), frontend
  (`check + test + build`), the debug bundle build, the **repo-boundary** check,
  the **secret scan**, the **license** check, and the governance-files check.
- At least one **review approval**, including **CODEOWNERS approval** for
  sensitive paths (licensing, core runtime, cloud boundary, CI).
- All review conversations resolved and your branch **up to date** with `main`.

Direct pushes to `main`, force-pushes, and branch deletion are disabled for
everyone. Maintainers: see [repo hardening](./docs/maintainers/repo-hardening.md).

## Contributions are welcome — and earn Pro

Issues, reproducible bug reports, and design discussion are welcome now. **Code
contributions are open**: pick up an accepted issue (or propose one first so the
scope is agreed), keep the change small and traceable, and open a pull request.

When a **qualifying pull request** you authored is merged, PortBay issues you a
perpetual **Pro** license automatically, tied to your GitHub account — paying for
PortBay with code instead of money is a first-class path (see
[Pro](https://docs.portbay.app/pro/)). "Qualifying" means a non-trivial,
accepted change — a real fix or feature, not a typo or whitespace PR — so the
path can't be farmed. Maintainers confirm qualification on merge.

## Questions

Use GitHub Discussions for setup questions and design discussion. Use issues for reproducible bugs and accepted feature work.
