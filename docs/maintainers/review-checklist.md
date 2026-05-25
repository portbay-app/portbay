# PR Review Checklist

This checklist is for maintainers reviewing incoming pull requests. Work through every section before approving. "Approve with nits" is fine for minor style issues that don't affect correctness or safety; everything else must be resolved before merge.

---

## 1. Scope and correctness

- [ ] The PR does one coherent thing. If it mixes unrelated changes, ask the contributor to split it.
- [ ] The change is correct: read the diff logic carefully, not just the surface. For Rust, pay attention to error propagation (`?` vs. explicit handling), ownership, and `unwrap()` / `expect()` calls without justification.
- [ ] No dead code, no commented-out code, no debug logging left in.
- [ ] If the PR touches Tauri IPC commands or event handlers, confirm the input is validated before use.

## 2. Tests

- [ ] New behavior has tests. Bug fixes have a regression test. Refactors at minimum preserve existing test coverage.
- [ ] Tests exercise the actual change — not just happy path if failure modes are meaningful.
- [ ] `cargo test --no-default-features` passes locally (or CI confirms it).
- [ ] `pnpm test` (Vitest) passes.
- [ ] No test has been deleted or skipped to make the build pass. If a test was removed, the PR description must justify it.

## 3. Documentation

- [ ] Public-facing behavior changes are reflected in the relevant `docs/` page.
- [ ] New CLI flags, config options, or IPC commands are documented.
- [ ] If the CHANGELOG is maintained, an entry exists for user-visible changes.

## 4. Tauri capabilities and permissions

- [ ] No new Tauri capability (`tauri.conf.json`, `capabilities/`) is added without explicit justification in the PR description.
- [ ] No new sidecar binary is introduced without a corresponding security review of what it does and how it is invoked.
- [ ] No new outbound network call is added without justification. Calls to public PortBay Cloud APIs using a public key or public endpoint are acceptable; any new private endpoint, customer backend, or billing API is a red flag — see section 6.

## 5. Error handling

- [ ] Rust errors use the project's `AppError` type (or the applicable structured error type for that module). New error variants are not bare `String` wraps unless there is a clear reason.
- [ ] Frontend surfaces errors to the user in a way that is actionable, not a raw stack trace or `[object Object]`.

## 6. Repo boundary check

This is the most critical security gate for the public AGPL repo. The following must **never** appear in this repo:

- Pro or Cloud feature implementation (anything that gates behavior on a license tier, other than calling a public API and verifying a signed entitlement with a public key)
- Billing or Stripe logic
- License-signing private keys or signing infrastructure
- Private API endpoints, internal service URLs, or customer/org backend code
- Team-sync server code or hosted tunnel infrastructure
- Enterprise policy engine implementation

**How to check:**
- Run `scripts/check-repo-boundaries.sh` against `.repo-boundary-denylist` and confirm it exits clean.
- Read the diff with the boundary rules in mind. Style-matching code that "happens to implement" a Pro feature is still a violation even if the contributor claims it is generic.
- If anything looks like it belongs in `portbay-app/portbay-cloud`, redirect it there with a clear explanation. Do not merge and then strip it — reject cleanly.

Client-side code that calls a documented public Cloud API and verifies a signed JWT/entitlement using the public key is acceptable. The private key and the signing logic live only in `portbay-cloud`.

## 7. Secrets and sensitive data

- [ ] No credentials, API keys, tokens, or passwords appear anywhere in the diff — including tests, fixtures, comments, and commit messages.
- [ ] No customer data, email addresses, or personally identifiable information appears.
- [ ] Secret scan (gitleaks) passes. If CI does not yet run it automatically, run it manually: `gitleaks detect --source .`

## 8. DCO sign-off

- [ ] Every commit in the PR has a `Signed-off-by: Name <email>` trailer, confirming the contributor certifies they have the right to contribute this code under the DCO (`git commit -s` produces this automatically).
- [ ] If commits are missing sign-off, ask the contributor to rebase and add it. Do not accept "squash at merge will fix it" as a substitute — squashing changes authorship and drops sign-off from the original commits.

## 9. License and SPDX headers

- [ ] New first-party source files carry the SPDX header:
  ```
  // SPDX-License-Identifier: AGPL-3.0-only
  // Copyright (c) 2026 Tribal House
  ```
  (Adjust year and comment style for the file type.)
- [ ] No new file carries a header that contradicts the project's AGPL-3.0-only license.
- [ ] See `docs/maintainers/license-review.md` for compatibility rules on third-party code.

## 10. Third-party and lifted code

**Why this matters:** copied code that arrives without attribution is a license and provenance risk, even when the contributor had good intentions.

**Signals to watch for:**
- A sudden large addition of well-structured code that does not match the contributor's prior commit style.
- Module or function names that do not follow PortBay conventions.
- Comments referencing an external project, author, or file path not in this repo.
- `git log --follow` shows the file appeared in one commit with no prior history.

**When you spot it:**
1. Ask in the PR: "This block looks like it may be adapted from an external source — can you confirm the origin and license?"
2. If the contributor confirms it is from a third-party project, verify the license is compatible (see `docs/maintainers/license-review.md`).
3. Require a `NOTICE` entry following the pattern already established in the top-level `NOTICE` file before merging.
4. If provenance cannot be established, reject the PR and explain why. Do not merge uncertain code and add attribution retroactively.

## 11. Redirecting feature requests to portbay-cloud

If a PR implements a feature that belongs in the private `portbay-cloud` repo (Pro-only functionality, billing, server-side team features, hosted infrastructure), close it with a clear explanation:

> "Thank you for this contribution. This feature is part of PortBay's Pro/Cloud tier and is implemented in our private backend repository. The public AGPL repo contains only the open-source client. We cannot merge Pro/Cloud implementation here."

Do not leave these PRs open. Close them promptly. If the feature could be designed to have a public-repo component (e.g., a UI that calls a public API), describe what that boundary would look like and invite a redesigned contribution.

## 12. Pro-qualifying PRs

A contributor earns a perpetual Pro license for a merged qualifying PR. "Qualifying" means the PR contains a non-trivial accepted change — not a typo fix, whitespace cleanup, documentation reword, or dependency bump.

When merging a qualifying PR:
1. Make a judgment call: is this substantive? If you are unsure, discuss with another maintainer.
2. Note in the merge comment (or in a subsequent comment on the closed PR): "This qualifies as a Pro PR."
3. Follow the internal process (documented in `portbay-cloud`) to issue the Pro entitlement to the contributor's GitHub account.

Do not promise Pro to a contributor before the PR is merged and confirmed qualifying.

## 13. Security reports

If a PR or issue appears to contain a security vulnerability report — whether filed as a bug report, as a PR with a "fix", or as a comment:

1. **Do not discuss details in the public PR or issue thread.** Even acknowledging specifics publicly can help adversaries.
2. Acknowledge receipt publicly with a generic response: "Thanks — we handle security reports privately. Please see `SECURITY.md`."
3. Contact the reporter and the maintainer team via `security@portbay.app` to continue the discussion privately.
4. Coordinate disclosure timing. Do not publish a fix before the reporter has been notified and an embargo period agreed upon (unless the vulnerability is already actively exploited in the wild).
5. Close or lock the public issue/PR once the private channel is established.

## 14. Contributor disputes and Code of Conduct

If a PR involves a Code of Conduct issue (harassment, hostile comments, bad-faith submissions):

1. Do not engage with the problematic behavior in the PR thread beyond one factual, neutral statement.
2. Contact `conduct@portbay.app` with the relevant links and context.
3. Maintainers decide collectively. Document the rationale in the internal conduct log, not the public thread.
4. If a PR itself is filed in bad faith (spam, vandalism, coordinated manipulation), close and lock it without extended engagement.

---

## Quick reference: approve vs. request changes vs. close

| Situation | Action |
|---|---|
| Looks good, minor nits only | Approve with inline comments |
| Correct but missing tests/docs | Request changes |
| Boundary violation (Pro/Cloud leak) | Request changes or close with explanation |
| No DCO sign-off | Request changes (fix the commits) |
| Unknown provenance of lifted code | Request changes; block on NOTICE entry |
| Feature belongs in portbay-cloud | Close with explanation |
| Security report in a PR | Acknowledge generically; move to private channel |
