---
title: "PortBay vs Local (WP Engine) alternative"
description: Compare PortBay and Local by WP Engine for local development. Open-source multi-runtime vs WordPress-focused container tool — find the right Local alternative.
---

# PortBay vs Local by WP Engine

PortBay is an open-source (AGPL-3.0), container-free local dev manager for macOS that handles Node, PHP, and static sites with automatic HTTPS and managed DNS. Local (formerly Local by Flywheel) is a closed-source, WordPress-focused local development tool from WP Engine that uses containers under the hood and provides an excellent one-click WordPress setup experience. If you build WordPress sites and want the smoothest possible one-click WordPress environment, Local is genuinely good at that specific job. If you run anything beyond WordPress, PortBay is the better fit.

## At a glance

| | PortBay | Local by WP Engine |
|---|---|---|
| License | AGPL-3.0 (open source) | Closed source |
| Price | Free · optional Pro | Free |
| Containers | None (native) | Container-based |
| Runtimes | Node, PHP, static | WordPress-first |
| Local HTTPS + `.test` | Built in (mkcert) | Built in |
| Managed DNS | Bundled dnsmasq | Built in |
| Reverse proxy | Caddy (automatic) | Managed by Local |
| Footprint | Small (native) | Large (container images) |
| Platform | macOS (Apple Silicon) | macOS + Windows + Linux |
| Automation | CLI + MCP (69 tools) | Limited CLI |
| AI agent task board | ✅ Markdown cards + handoff memory | ❌ |

## What they share

Both tools provide per-project HTTPS with real local domains — no `localhost:PORT` juggling. Both handle DNS resolution automatically. Both have a GUI and are aimed at developers who don't want to spend time on environment plumbing. Both support PHP-based web applications and bundle database services alongside the web server.

## Where PortBay is different

PortBay is **open source** under AGPL-3.0. Local is proprietary, owned and maintained by WP Engine.

PortBay runs code **natively** — no container engine, no image pulls. Node processes, PHP-FPM, and static file serving all happen directly on your Mac. Local runs each WordPress site inside containers, which adds startup time, image storage, and a container engine dependency.

PortBay handles **Node and static sites** as first-class citizens alongside PHP. If you run a Next.js front-end, a Laravel API, and a static landing page, all three can live in PortBay under their own `.test` hostnames. Local is designed specifically for WordPress.

PortBay includes a **CLI with full parity** to the GUI, plus an **MCP server** that exposes every project action to AI agents and external tools. Local's automation surface is limited by comparison.

PortBay also gives those agents **work to do**. Every project gets a task board whose cards are Markdown files in your repo — move a card to *To Do* and the coding agent you assigned (Claude Code, Codex, Cursor, Gemini, and more) picks it up, does the work on your machine, and appends a handoff brief the next run reads first. For an agency juggling many sites, that means the small fixes queue up and get done without a developer sitting on each one.

PortBay's **Cloudflare tunnel** integration lets you share a live local project publicly in one step without installing a separate tunnel client. Local offers a similar feature called Live Links through its own service.

## Where Local by WP Engine is stronger

Local is built from the ground up for **WordPress** and it shows. Creating a new WordPress site takes a single form fill and a click. WordPress version selection, multisite configuration, WP-CLI integration, and pull/push workflows with WP Engine hosting are all first-class features. For a WordPress developer or agency, the DX is polished.

Local is **cross-platform**: macOS, Windows, and Linux all work today. PortBay is macOS-only; Linux and Windows are on the roadmap but not yet shipped.

Because Local uses containers, it can run multiple PHP versions and match specific WordPress hosting environments with container-level precision — useful when debugging hosting-specific issues.

Local's WP Engine integration is seamless if you host on WP Engine: one-click push to staging, pull from production. That workflow has no equivalent in PortBay.

## Choose Local by WP Engine when

- Your work is primarily or exclusively WordPress sites.
- You want one-click WordPress provisioning with WP-CLI, multisite, and WP Engine push/pull built in.
- Cross-platform support for Windows or Linux teammates is needed today.
- You are a WordPress agency and the WP Engine hosting integration saves you deployment steps.

## Choose PortBay when

- You build beyond WordPress — Node, Next.js, Laravel, Vite apps, static sites, or any mix.
- Open source matters for your tooling or organization.
- You want native execution with no container overhead.
- CLI-first automation, MCP server support, and a task board your coding agents work are important.
- Perpetual Pro access via contribution (not a commercial product) fits your context.
- You prefer a lighter footprint on your Mac.

## Bottom line

Local by WP Engine is excellent at the specific job it was built for: one-click WordPress development with WP Engine hosting integration. PortBay is the right pick for developers who need a multi-runtime, open-source, container-free tool that handles more than WordPress. [Install PortBay](/getting-started/install) to run your whole stack under one roof.

---

See all [comparisons](/comparisons/) — developers also compare PortBay with [Docker / OrbStack](/comparisons/portbay-vs-docker) and [MAMP](/comparisons/portbay-vs-mamp).
