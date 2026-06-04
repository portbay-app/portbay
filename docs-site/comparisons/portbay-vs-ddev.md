---
title: "PortBay vs DDEV: local dev without Docker"
description: Compare PortBay and DDEV for local development. Native runtime vs Docker-based containers, macOS-only vs cross-platform, and when each is the right DDEV alternative.
---

# PortBay vs DDEV

PortBay is an open-source (AGPL-3.0), container-free local dev manager for macOS that runs Node, PHP, and static sites natively with automatic HTTPS and bundled DNS. DDEV is an open-source, Docker-based local development tool that is particularly strong for CMS-heavy workflows: Drupal, WordPress, TYPO3, Magento, and similar platforms. Both provide per-project HTTPS, both are open source, and both support PHP. The central difference is containers versus native execution.

## At a glance

| | PortBay | DDEV |
|---|---|---|
| License | AGPL-3.0 (open source) | Apache-2.0 (open source) |
| Price | Free · optional Pro | Free |
| Containers | None (native) | Docker-based |
| Runtimes | Node, PHP, static | PHP-first, Node in containers, multi-CMS |
| Local HTTPS + `.test` | Built in (mkcert) | Built in (mkcert) |
| Managed DNS | Bundled dnsmasq | Built in |
| Reverse proxy | Caddy (automatic) | Traefik (automatic) |
| Footprint | Small (native) | Large (container images per project) |
| Platform | macOS (Apple Silicon) | macOS, Linux, Windows |
| Automation | CLI + MCP (69 tools) | CLI (ddev), extensive add-ons |
| AI agent task board | ✅ Markdown cards + handoff memory | ❌ |

## What they share

Both are open source and provide per-project local HTTPS out of the box using mkcert. Both handle DNS automatically so you work with real hostnames rather than `localhost:PORT`. Both have a full CLI. Both are actively maintained and have real communities behind them. Both run PHP and support database services alongside the web server.

## Where PortBay is different

PortBay runs your code **natively on the host** — no container engine required. There are no images to pull, no container startup time, no Docker daemon to manage. The Node or PHP process runs the same way it would if you ran `npm start` in your terminal, with HTTPS and DNS layered on top automatically.

The **small footprint** is a concrete benefit. PortBay targets under 80 MB idle RAM and a sub-30 MB installer. A DDEV project involves container images that can be hundreds of megabytes each. If you run several projects simultaneously, the resource difference is noticeable.

PortBay's **MCP server** exposes every project action — start, stop, add hostname, view logs — to AI agents and external automation. DDEV has no equivalent today.

Beyond exposing tools, PortBay gives agents **work**. Every project gets a task board whose cards are Markdown files in your repo — move a card to *To Do* and the coding agent you assigned (Claude Code, Codex, Cursor, Gemini, and more) picks it up, does the work on your machine, and appends a handoff brief the next run reads first. Your backlog moves while you review diffs instead of babysitting prompts.

PortBay's **Cloudflare tunnel integration** lets you share a local project publicly in one step. DDEV has `ddev share` via ngrok, but that's a separate service.

Pro access in PortBay is perpetual (earned by donation or merged PR), not a sponsorship model.

## Where DDEV is stronger

DDEV is purpose-built for **CMS ecosystems**. It ships with first-class support for Drupal, WordPress, TYPO3, Magento, Craft CMS, and others via add-ons and project types that pre-configure the stack. If you maintain Drupal multisite or a Magento instance, DDEV's project types save hours of initial configuration.

DDEV is **cross-platform**: the same project config runs on macOS, Linux, and Windows. If you have a team with mixed operating systems, DDEV's `.ddev/config.yaml` travels with the repo and sets up the same environment everywhere.

Because DDEV uses containers, it can match specific PHP versions, system packages, and services that a particular CMS version requires, down to the OS-level package. This is production-parity-style local dev without writing your own Dockerfiles.

DDEV's add-on ecosystem is mature: Solr, Elasticsearch, Redis, Mailpit, and many more are one `ddev get` away.

## Choose DDEV when

- You work primarily with Drupal, WordPress, TYPO3, Magento, or another CMS that DDEV has a project type for.
- Cross-platform team consistency is essential — same config on macOS, Linux, and Windows.
- You need container-level PHP version and system package control to match a specific production environment.
- You already have Docker in your workflow and want to stay in that model.
- You use DDEV's add-on ecosystem (Solr, Elasticsearch, Mailpit, etc.).

## Choose PortBay when

- You want to skip Docker entirely and run Node and PHP natively.
- Footprint matters — fewer GB of container images, lower idle RAM.
- You mix Node, Next.js, or Vite front-ends with PHP back-ends and want one tool for both.
- MCP server support for AI-assisted workflows is on your list — or a task board that dispatches cards to your coding agents.
- macOS-only is fine for your team today.
- You want perpetual Pro access via contribution rather than ongoing sponsorship.

## Bottom line

DDEV is the right choice for CMS-heavy teams that need container parity and cross-platform consistency. PortBay is the right choice when you want native speed, zero container overhead, and a single open-source tool for modern Node + PHP stacks on macOS. [Install PortBay](/getting-started/install) and skip the Docker daemon.

---

See all [comparisons](/comparisons/) — developers also compare PortBay with [Docker / OrbStack](/comparisons/portbay-vs-docker) and [Laravel Valet](/comparisons/portbay-vs-laravel-valet).
