# Repository Hardening

How the public `portbay-app/portbay` repository is locked down so that:

- nothing proprietary, secret, or forbidden can be committed (let alone merged),
- outside contributors cannot overwrite protected files or tamper with the
  guards, and
- a change cannot merge until it has passed review and all builds/checks.

This is the maintainer reference. The contributor-facing version is in
[`CONTRIBUTING.md`](../../CONTRIBUTING.md) and
[`docs/contributing/pull-requests.md`](../contributing/pull-requests.md).

## Defense in depth — four layers

| Layer | Where | Catches |
|---|---|---|
| `.gitignore` | working tree | secrets/keys/env/artifacts, by default |
| Git hooks (`.githooks/`) | local, pre-commit + pre-push | leaks **before the commit exists** — no force-clean ever |
| CI (`ci.yml` + `governance.yml`) | every push & PR | anything a missing/bypassed hook let through |
| Branch protection (rulesets) | server, on merge | unreviewed, failing, or unauthorized changes |

The hooks are version-controlled and auto-installed by `pnpm install` (via the
`prepare` script → `scripts/setup-hooks.sh`, which sets `core.hooksPath`). A
contributor who skips them, or uses `--no-verify`, is still stopped by CI and
branch protection.

## Applying the server-side settings

Most settings are codified and applied with one script (dry-run by default):

```bash
gh auth login                              # once, as a repo admin
scripts/apply-repo-protection.sh           # preview
scripts/apply-repo-protection.sh --apply   # apply
```

It upserts the rulesets in `.github/rulesets/`, sets the Actions token to
read-only, and turns on secret scanning + push protection, Dependabot alerts +
security fixes, and private vulnerability reporting. Re-running is safe.

You can also import the rulesets by hand: **Settings → Rules → Rulesets → New →
Import**, then upload `.github/rulesets/main-protection.json` and
`release-tags.json`.

## Branch protection (the `main` ruleset)

`main-protection.json` enforces, with **no bypass actors** (admins included):

- **Pull request required** — no direct pushes to `main`. 1 approving review,
  **CODEOWNERS review required**, stale approvals dismissed on new pushes,
  last-push approval required, and all review threads resolved.
- **Required status checks, strict** — the branch must be up to date and every
  check below must be green before merge:
  - `rust · fmt + clippy + test`
  - `frontend · check + test + build`
  - `bundle · debug build`
  - `repo boundaries (no Pro/Cloud leakage)`
  - `secret scan (gitleaks)`
  - `license check (SPDX + dependencies)`
  - `required governance files present`
  - `DCO sign-off`
- **Linear history**, **no force-push** (`non_fast_forward`), **no deletion**.
- **Squash or rebase merges only** (no merge commits).

> Check-name matching: a required status check must match the job's display name
> exactly. If you rename a job in `ci.yml`/`governance.yml`, update the contexts
> in `main-protection.json` and re-apply, or the gate silently won't bind.

### Protecting specific files from being overwritten

`.github/CODEOWNERS` assigns maintainers to the sensitive paths — `LICENSE`,
`NOTICE`, `/.github/workflows/`, `.github/CODEOWNERS`, the cloud-boundary code,
`.repo-boundary-*`, `scripts/check-repo-boundaries.sh`, `SECURITY.md`. Because
the ruleset requires CODEOWNERS review, **a PR that touches any of these cannot
merge without a maintainer's approval** — an outside contributor cannot quietly
edit a workflow, the license, or the boundary guards.

## Release-tag protection

`release-tags.json` makes `v*` tags immutable: no deletion, no update, no
force. Combined with the signing-gated `release.yml`, this protects release
integrity. To additionally restrict *who* may create release tags, add a
maintainers team to that ruleset's `bypass_actors` and set a `creation`
restriction (left open until the team exists).

## Actions hardening

- **Default workflow token is read-only** (`default_workflow_permissions=read`);
  jobs opt into more with an explicit `permissions:` block. `governance.yml`
  already declares `contents: read`.
- **Actions cannot create or approve pull requests.**
- **Fork-PR safety:** workflows trigger on `pull_request` (not
  `pull_request_target`), so a fork's PR runs with a read-only token and **no
  access to secrets** — a malicious PR that edits a workflow cannot exfiltrate
  anything, and it still needs CODEOWNERS approval to merge. Never switch these
  to `pull_request_target` with a checkout of the PR head.

## Contributor requirements (the merge gate)

For a contribution to merge, the contributor must:

1. Work on a branch and open a **pull request** (no direct push).
2. **Sign off every commit** (`git commit -s`) — the `DCO sign-off` check
   enforces it. See [`DCO.md`](../../DCO.md).
3. Pass **all required status checks** (build, tests, lint, boundary, secret
   scan, license, governance files). A red check blocks merge.
4. Get a **review approval**, including **CODEOWNERS approval** for any
   sensitive path.
5. Resolve all review conversations and keep the branch **up to date** with
   `main`.

Contributors can't *open* a PR conditionally, but a PR that fails any of the
above simply cannot be merged.

## Manual steps `gh` can't reliably set (do these once in the UI)

- **Settings → Actions → General → Fork pull request workflows:** require
  approval for **all outside collaborators** (or all external contributors).
- **Settings → Actions → General:** "Allow GitHub Actions to create and approve
  pull requests" → **off** (the script sets the token side; confirm this toggle).
- **Settings → Rules → Rulesets:** restrict **who can create/edit rulesets** to
  admins; confirm `main-protection` and `release-tags` show **Active**.
- **Settings → General → Pull Requests:** enable "Automatically delete head
  branches"; keep only Squash/Rebase enabled.
- **Settings → Code security:** confirm secret scanning, push protection, and
  private vulnerability reporting are **on** (the script enables them; verify).
- **Disable unused surfaces:** Wiki / Projects if not used, to reduce attack
  surface.

## Break-glass

The ruleset has no bypass actors by design. If a genuine emergency requires it,
a repo admin can temporarily add themselves to `bypass_actors` (or set the
ruleset to *evaluate*), make the fix via a reviewed PR if at all possible, and
**revert the bypass immediately**. Record why in the PR.

---

*Operational guidance, not legal advice.*
