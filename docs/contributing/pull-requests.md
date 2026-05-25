# Pull Requests

---

## Before you open a PR

- The issue you are fixing or implementing should be accepted (`help wanted` or maintainer confirmation). For anything non-trivial, agree on scope first.
- Check out a feature branch. Never target `main` directly from a fork.
- Keep the change atomic — one coherent thing per PR.
- Run the full check suite locally. See [development.md](development.md) for the commands.

---

## DCO sign-off

Every commit must be signed off. This is enforced by the DCO check in CI.

```bash
git commit -s -m "fix(dns): resolve .test wildcard when dnsmasq restarts"
```

The `-s` flag appends:

```
Signed-off-by: Your Name <you@example.com>
```

If you forget to sign off a commit, you can amend before pushing:

```bash
git commit --amend -s
```

For multiple commits without sign-offs, use `git rebase --signoff HEAD~N`. See [DCO.md](../../DCO.md) for the full certificate text.

---

## PR checklist

The PR template at `.github/pull_request_template.md` includes the checklist that appears when you open a PR. The main items:

- [ ] Commits are signed off (`git commit -s`)
- [ ] `cargo fmt --all -- --check` passes
- [ ] `cargo clippy --all-targets --no-default-features -- -D warnings` passes
- [ ] `cargo test --no-default-features` passes
- [ ] `pnpm check` passes
- [ ] `pnpm test` passes
- [ ] `pnpm build` passes
- [ ] Tests added or updated for backend behaviour changes
- [ ] Docs updated for user-visible behaviour changes
- [ ] New Tauri capabilities justified and scoped in the PR description
- [ ] `scripts/check-repo-boundaries.sh` passes (no denylist hits)
- [ ] The change is one coherent thing — no bundled unrelated work

---

## How maintainers review

Maintainers look at:

**Scope and atomicity** — is this one thing? Bundled refactors that aren't related to the stated change will be asked to be split out.

**Tests** — backend behaviour changes need tests. Rust: unit tests in the same file or `tests/`. Frontend: Vitest. The CI gate must pass.

**Docs** — user-visible changes (new behaviour, changed defaults, new CLI flags) need corresponding doc updates.

**Tauri capability justification** — any new `tauri:allow-*` permission must be explained in the PR body. Overly broad permissions will be rejected.

**Repo boundary check** — `scripts/check-repo-boundaries.sh` runs in CI against `.repo-boundary-denylist`. A hit blocks merge. See [docs/architecture/repo-boundaries.md](../architecture/repo-boundaries.md) and [license-policy.md](license-policy.md).

**DCO** — all commits must carry a sign-off. The CI DCO check is not advisory.

---

## Merge process

PortBay uses **squash merge** for most PRs to keep `main` history clean. If your PR has a well-structured commit history that should be preserved, say so in the PR description.

Maintainers may request changes once or twice. If a PR goes stale (no response to review comments for three weeks), it may be closed with an invitation to reopen when ready.

---

## Earning Pro via a merged PR

When a qualifying pull request you authored is merged, PortBay issues you a perpetual **Pro** license tied to your GitHub account — no payment required.

"Qualifying" means a non-trivial accepted change — a real fix, feature, or meaningful documentation improvement — not a typo, whitespace, or automated PR. Maintainers confirm qualification at merge time. The mechanism is described at [portbay.app/pro](https://portbay.app/pro).

This is intentional: the project values code contributions and wants to reward them directly. It is also the reason a whitespace PR will never qualify — the path cannot be farmed.

---

## Questions during review

If a maintainer's review comment is unclear, reply in the PR thread. Avoid force-pushing to a branch under active review without warning — it resets the diff view for reviewers.
