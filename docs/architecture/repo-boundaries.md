# Repository Boundaries

PortBay lives in two repositories with a hard line between them. This document
defines that line so contributors and maintainers can tell, at a glance, where a
given piece of code belongs.

| Repository | Visibility | License | Contains |
|---|---|---|---|
| [`portbay-app/portbay`](https://github.com/portbay-app/portbay) | Public | AGPL-3.0-only | The Community app and everything it needs to run locally. |
| `portbay-app/portbay-cloud` | Private | Proprietary | PortBay Cloud/Pro: the commercial backend and business logic. |

The rule in one sentence: **the public repo is a local-first application that
may *talk to* cloud services over documented public APIs, but the
*implementation* of those services lives only in `portbay-cloud`.**

## Commit-time decision table

When you're about to commit, every change falls into exactly one bucket:

| Bucket | What | Where it goes | Enforced by |
|---|---|---|---|
| **ALWAYS in the app** | Local runtime, UI, CLI, sidecar adapters, project/registry logic, local HTTPS/DNS, community docs, the thin public-API client. | `portbay-app` (this repo) | CI + review |
| **APP — public client only** | Calls to PortBay Cloud's *public* APIs; entitlement *verification* with the public key; local encryption before sync upload. | `portbay-app` | CODEOWNERS review on the cloud-boundary paths |
| **CLOUD only** | Billing, license **signing**, entitlement issuance/validation, team-sync server, hosted tunnels, org/customer backend, enterprise policy engine, private infra. | `portbay-cloud` (private) | denylist + secret scan block it here |
| **NEVER committed** | Real `.env`, secrets, tokens, private keys/certs (`*.pem`/`*.key`/…), `.secrets-local/`, signed build artifacts (`*.dmg`), customer data. | nowhere (use a secret manager / Releases) | `.gitignore` + `path:` rules + secret scan |

If you can't place a change with confidence, it belongs in a discussion before
it's committed — open an issue or ask a maintainer.

## `portbay-app` (this repo) MAY contain

- The open-source desktop app (Tauri + Svelte) and the CLI.
- The local runtime manager, daemon, and reconciler.
- The service/process manager and sidecar adapters (Process Compose, Caddy,
  dnsmasq, mkcert, mailpit, the privileged hosts helper).
- Local project configuration and the declarative registry.
- Local HTTPS / dev-domain (`.test`) management.
- Local logs and metrics.
- Open plugin interfaces and public API/schema definitions that reveal no
  private implementation.
- Community documentation.
- A **thin, generic client** that calls PortBay Cloud's public APIs and verifies
  a signed entitlement using a **public** key (see
  [cloud integration](./cloud-integration.md)).

## `portbay-app` (this repo) must NOT contain

- Billing logic, or any Stripe/payment secret or private logic.
- License-enforcement secrets or the license-**signing** private key (the
  matching **public** key is shipped on purpose; the private key is not).
- Private cloud APIs or their server-side implementation.
- Team-sync server implementation.
- Hosted tunnel / remote-access infrastructure.
- Proprietary authentication backends (the client-side login flow is fine; the
  server that issues and validates sessions is not).
- Customer or organization management backend.
- Production cloud infrastructure / deployment code or secrets.
- Private API keys or private endpoints.
- Closed-source Pro feature implementations.
- The enterprise policy-engine implementation.
- Private roadmap items that reveal confidential commercial strategy.

## `portbay-cloud` (the private repo) MAY contain

- The commercial backend and private API services.
- Team/organization management.
- Billing and payment integration.
- Cloud sync (server side) and hosted remote tunnels.
- Enterprise authentication and account plans.
- Managed recipes, feature flags, and Pro entitlement **issuance/validation**.
- Telemetry backend (if any) and private infrastructure code.
- The license-**signing** private key (stored as a secret, never in git).

## The integration boundary

The Community app communicates with PortBay Cloud **only** through documented,
stable, public APIs. The client code in this repo is deliberately minimal and
generic:

- It knows one base URL (a public domain) and a set of public request/response
  shapes.
- It verifies signed entitlements with an embedded **public** key — it never
  signs, issues, or validates them server-side.
- It encrypts sync payloads locally; the server stores opaque ciphertext.
- It embeds **no** secrets, private endpoints, or proprietary business logic.

If a feature needs server-side secrets, billing, or proprietary logic, that part
belongs in `portbay-cloud`. The public client should expose only what's needed
to *call* it.

## How the boundary is enforced — four layers

This is not an honor system. The boundary is enforced at four points, each
catching what an earlier one might miss. The goal is that a violation is
**impossible to commit**, so the history never needs a force-clean.

1. **`.gitignore`** — keeps secrets, env files, keys, and build artifacts out by
   default.
2. **Local git hooks** (`.githooks/`, installed via `core.hooksPath` on
   `pnpm install` or `scripts/setup-hooks.sh`):
   - **`pre-commit`** runs `scripts/check-repo-boundaries.sh --staged` plus a
     gitleaks staged scan. It blocks the commit **before it is created** if
     staged changes contain forbidden content, a forbidden path (even via
     `git add -f`, which bypasses `.gitignore` — this does not), or a secret.
   - **`pre-push`** re-runs the full check before anything reaches the remote.
3. **CI** — the `governance` workflow runs the boundary check, gitleaks
   (full history), and the license check on every push and PR, so a bypass
   (`--no-verify`) or a contributor without hooks is still caught.
4. **Branch protection** — `main` requires PR review, passing checks, and
   CODEOWNERS approval on sensitive paths, and forbids force-push and deletion.
   See [`docs/maintainers/repo-hardening.md`](../maintainers/repo-hardening.md).

The rules live in **`.repo-boundary-denylist`** (forbidden content strings,
`re:` regexes, and `path:` globs) with reviewed exceptions in
**`.repo-boundary-allow`**. Run the check anytime:

```bash
scripts/check-repo-boundaries.sh            # full tree (what CI and pre-push run)
scripts/check-repo-boundaries.sh --staged   # just what you're about to commit
```

If a check fires on something that is genuinely documentation rather than a
leak, add the file's path to `.repo-boundary-allow` (and be sure you're right).

When a contribution or feature request belongs on the commercial side, it is
labeled and moved toward `portbay-cloud` rather than merged here — see
[issue triage](../contributing/issue-triage.md).
