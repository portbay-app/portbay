---
layout: home
title: PortBay — Open-Source Local Dev Environment
description: Open-source, container-free local dev environment for macOS and Linux — a Laravel Herd alternative. Real HTTPS .test domains, managed DNS, reverse proxy, and bundled databases, with no hand-edited /etc/hosts.

hero:
  name: PortBay
  text: One Play button per project.
  tagline: Run Node, PHP, Python, and static sites with real HTTPS hostnames, managed DNS, and a reverse proxy you never touch. No containers, no hand-edited /etc/hosts.
  actions:
    - theme: brand
      text: Get started
      link: /getting-started/
    - theme: alt
      text: Try it in your browser
      link: https://try.portbay.app
    - theme: alt
      text: CLI reference
      link: /reference/cli
    - theme: alt
      text: Troubleshooting
      link: /troubleshooting/

features:
  - title: One registry, GUI + CLI
    details: Projects, hostnames, ports, services, readiness probes, and environment variables live in a JSON registry both the app and CLI use.
    link: /getting-started/add-project
  - title: Local HTTPS by hostname
    details: Caddy routes each project to https://&lt;name&gt;.test, with mkcert issuing the local certificates.
    link: /guides/caddy-https
  - title: HTTP request inspector
    details: Live Caddy access-log tailing into a filterable request table — method, status, latency, and the matched project.
    link: /guides/http-inspector
  - title: Bundled databases
    details: PortBay-supervised MySQL, MariaDB, Postgres, and Redis instances with connection details and per-project links.
    link: /guides/databases
  - title: A full SSH workspace
    details: Save a remote host, then open an interactive terminal, an SFTP file browser with an inline editor, and port-forward tunnels — without keeping a second app open.
    link: /guides/ssh-tunnels
  - title: Sandboxed runner (Pro)
    details: Run an untrusted project inside a macOS sandbox profile, inspect it, then promote it to a normal local run.
    link: /guides/sandbox
  - title: Drive it from an AI agent
    details: An MCP server exposes 69 tools and resources to Claude Code, Cursor, Zed, and any MCP-aware client.
    link: /agents/
  - title: A task board your agents work
    details: Move a card to To Do and the AI agent you assigned picks it up, works it in your repo, and writes a hand-off note for the next run.
    link: /guides/task-board
---

<ThemeImage name="projects" alt="PortBay managing local projects" />

> **See it without installing.** The [interactive simulator](https://try.portbay.app) runs the real PortBay interface against a set of sample projects, right in your browser — click Play on a project and watch it start.

## Current Release State

PortBay is released for macOS (Apple Silicon), with Linux desktop packages in active support. Signed, notarized macOS builds ship via the DMG and Homebrew cask; Linux builds target AppImage, deb, rpm, Snap, and AUR packages. Windows is still ahead.

| Area | Status |
| --- | --- |
| macOS app (Apple Silicon) | Available — signed & notarized |
| Linux app (x86_64) | In active support — AppImage, deb, rpm, Snap, AUR |
| Windows | On the roadmap |
| Homebrew cask, DMG, auto-update | Available |
| Process Compose sidecar | Bundled in the app and for local development |
| Caddy, mkcert, Mailpit, cloudflared | Bundled in the app; fetched per checkout when building from source |
| Searchable public docs | This site |

## The Short Version

```bash
brew tap portbay-app/portbay
brew install --cask portbay
```

Then add a project, choose its type and port, and use the row actions to start it, open its local URL, inspect logs, or stop it. Prefer to build it yourself? See [Install → Build From Source](/getting-started/install).

## Where To Go

- New to PortBay: start with [Install](/getting-started/install) and [First Run](/getting-started/first-run).
- Setting up routes and certificates: read [Caddy and HTTPS](/guides/caddy-https).
- Running PHP projects: read [PHP Setup](/guides/php-setup).
- Debugging failures: use [Troubleshooting](/troubleshooting/).
- Automating from a terminal: use [CLI Usage](/guides/cli-usage) and the [CLI Reference](/reference/cli).
- Choosing a tool: see how PortBay [compares to Herd, ServBay, Docker, and more](/comparisons/).
