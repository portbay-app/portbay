# Issue Triage

This document describes how issues are triaged in `portbay-app/portbay` and how maintainers decide scope.

---

## Triage flow

1. A new issue arrives unlabelled (or with `needs-triage`).
2. A maintainer reads the report, asks for a reproduction if needed, and applies labels.
3. If the issue is in scope for this repo, it moves toward `bug` or `enhancement`.
4. If it belongs in `portbay-cloud`, the maintainer applies `cloud/pro` and explains why.
5. Community members can pick up issues labelled `help wanted` or `good first issue`.

Triage usually happens within a few business days. If an issue is stale for more than two weeks with no response from the reporter, it may be closed as `needs-triage` with a comment asking the reporter to update.

---

## Label taxonomy

### Status labels

| Label | Meaning |
|---|---|
| `needs-triage` | Not yet reviewed by a maintainer |
| `discussion-needed` | Scope or approach needs broader agreement before code |
| `help wanted` | Maintainers want a community contribution |
| `good first issue` | Suitable for a first-time contributor |
| `wontfix` | Out of scope or not a bug; will not be fixed in this form |
| `duplicate` | Tracked elsewhere; linked in the comment |

### Type labels

| Label | Meaning |
|---|---|
| `bug` | Confirmed defect with reproducible steps |
| `enhancement` | New capability or improvement to an existing one |

### Area labels

These match the areas in the bug report template:

| Label | Area |
|---|---|
| `area:projects` | Project detection, registry, start/stop |
| `area:sidecars` | Process Compose, Caddy, mailpit, cloudflared |
| `area:dns-hosts` | dnsmasq, wildcard `.test` DNS, `/etc/hosts` |
| `area:https-certs` | mkcert, local certificate management |
| `area:logs` | Log capture, display, filtering |
| `area:cli` | CLI commands and parity with the GUI |
| `area:settings` | Settings UI and persisted configuration |
| `area:installer-update` | Installation, auto-update, build pipeline |

### Scope labels

| Label | Meaning |
|---|---|
| `cloud/pro` | This belongs in `portbay-cloud`, not this repo |

---

## Community vs. cloud scope decisions

The main question is: does this feature require server-side infrastructure, a hosted service, billing, or multi-user/team state that lives outside the user's machine?

**Stays in this repo (portbay community):**
- Anything that runs entirely on the developer's local machine
- Client-side code that calls documented public PortBay Cloud APIs
- Local HTTPS, DNS, port management, project start/stop, sidecar configuration
- CLI and IPC commands with no server dependency

**Redirected to `portbay-cloud`:**
- Hosted tunnel infrastructure or routing
- Team or org project sharing
- Cloud license server, billing, or Stripe integration
- Persistent remote state (org membership, shared configs stored server-side)
- Enterprise policy enforcement logic

When an issue is redirected, the maintainer applies `cloud/pro`, closes the issue in this repo with an explanation, and — if appropriate — opens a tracking issue in the private repo (no public link to internals).

---

## What makes a good bug report

A maintainer should be able to reproduce the bug without asking follow-up questions. The bug template asks for:

- PortBay version and macOS version
- Steps to reproduce (numbered, minimal)
- Expected vs. actual behaviour
- Relevant logs (from the PortBay log viewer or Console.app)
- The affected area (used to apply `area:*` labels)

If a report lacks reproduction steps, the maintainer will ask once and apply `needs-triage`. If there is no response after two weeks, the issue may be closed.
