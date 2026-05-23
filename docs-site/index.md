---
layout: home

hero:
  name: PortBay
  text: Local development, managed as a small native control plane.
  tagline: Register projects once, run them predictably, route them through local HTTPS hostnames, and keep the sidecars visible.
  actions:
    - theme: brand
      text: Get started
      link: /getting-started/
    - theme: alt
      text: CLI reference
      link: /reference/cli
    - theme: alt
      text: Troubleshooting
      link: /troubleshooting/

features:
  - title: One registry
    details: Projects, hostnames, ports, services, readiness probes, and environment variables live in a JSON registry that the GUI and CLI both use.
  - title: Sidecar-aware
    details: Process Compose runs project commands, Caddy routes hostnames, mkcert issues local certificates, and service sidecars stay visible in the app.
  - title: Built for recovery
    details: Structured errors include what happened, why it matters, who can fix it, and which action should come next.
---

## Current Release State

PortBay is pre-MVP software for macOS. The current codebase is useful for development and validation, but it is not yet packaged as a general-availability product. The docs are written against the active Phase 3 and Phase 4 implementation.

| Area | Status |
| --- | --- |
| macOS app | In active development |
| Linux and Windows | Deferred |
| Homebrew, notarized builds | Not available yet |
| Process Compose sidecar | Bundled for local development |
| Caddy, mkcert, Mailpit, cloudflared | Fetched per checkout for development |
| Searchable public docs | This site |

## The Short Version

```bash
pnpm install
./scripts/fetch-caddy.sh
./scripts/fetch-mkcert.sh
./scripts/fetch-mailpit.sh
./scripts/fetch-cloudflared.sh
pnpm tauri dev
```

Then add a project, choose its type and port, and use the row actions to start it, open its local URL, inspect logs, or stop it.

## Where To Go

- New to PortBay: start with [Install](/getting-started/install) and [First Run](/getting-started/first-run).
- Setting up routes and certificates: read [Caddy and HTTPS](/guides/caddy-https).
- Running PHP projects: read [PHP Setup](/guides/php-setup).
- Debugging failures: use [Troubleshooting](/troubleshooting/).
- Automating from a terminal: use [CLI Usage](/guides/cli-usage) and the [CLI Reference](/reference/cli).
