---
title: "PortBay vs Laravel Valet: GUI + multi-runtime"
description: Compare PortBay and Laravel Valet for local macOS development. Open-source multi-runtime GUI tool vs minimal PHP-first CLI — find the right Valet alternative.
---

# PortBay vs Laravel Valet

PortBay is an open-source (AGPL-3.0), container-free local dev manager for macOS with a GUI, bundled DNS, a Caddy reverse proxy, and support for Node, PHP, and static sites. Laravel Valet is an open-source (MIT), minimal, CLI-only local PHP environment for macOS that uses dnsmasq for DNS and Nginx for routing. Both are container-free and lightweight. The key differences are runtime breadth, the presence of a GUI, and which reverse proxy they use.

## At a glance

| | PortBay | Laravel Valet |
|---|---|---|
| License | AGPL-3.0 (open source) | MIT (open source) |
| Price | Free · optional Pro | Free |
| Containers | None (native) | None (native) |
| Runtimes | Node, PHP, static | PHP-first (Valet drivers for static, some others) |
| Local HTTPS + `.test` | Built in (mkcert) | Built in (mkcert) |
| Managed DNS | Bundled dnsmasq | dnsmasq (Homebrew-managed) |
| Reverse proxy | Caddy (automatic) | Nginx (automatic) |
| Footprint | Small (native) | Tiny (Nginx + PHP-FPM only) |
| Platform | macOS (Apple Silicon) | macOS |
| Automation | CLI + MCP (69 tools) | CLI (valet) |
| AI agent task board | ✅ Markdown cards + handoff memory | ❌ |

## What they share

Both tools are open source and container-free. Both use dnsmasq for local DNS resolution and provide real HTTPS `.test` hostnames via mkcert — no manual `/etc/hosts` editing. Both run on macOS and run project code natively on the host. Both have a full CLI so you can script project management. They share a philosophical approach: stay out of your way, don't add containers, and let your code run as-is.

## Where PortBay is different

PortBay adds a **GUI and menu-bar mode** on top of the Valet-style model. You can see all projects, their status, logs, and metrics at a glance without typing commands. For teams with developers who are less comfortable in the terminal, this matters.

PortBay uses **Caddy** as the reverse proxy instead of Nginx. Caddy's configuration model is simpler for multi-runtime routing and its admin API is what allows PortBay to manage routes programmatically. This is what enables the MCP server — every proxy and project action is available to AI agents and external scripts.

The MCP server is half of the agent story; the **task board** is the other half. Every project gets one, and the cards are Markdown files in your repo — move a card to *To Do* and the coding agent you assigned (Claude Code, Codex, Cursor, Gemini, and more) picks it up, does the work on your machine, and appends a handoff brief the next run reads first. Valet manages requests into your projects; PortBay also manages the work queue on top of them.

PortBay's runtime support is **genuinely multi-runtime**. A Next.js project and a Laravel API can run side by side with their own `.test` hostnames. Valet drivers exist for some non-PHP runtimes but PHP is clearly the primary citizen.

PortBay bundles **databases** (MySQL, MariaDB, Postgres, Redis) and **Cloudflare tunnel** integration for public sharing. Valet is intentionally minimal and leaves database management and tunneling to you.

The **declarative JSON registry** means your project list is a portable config file. Valet's project list is based on parked/linked directories.

## Where Laravel Valet is stronger

Valet is **tiny** — it installs a handful of PHP packages and configures Nginx and dnsmasq via Homebrew. If you want the smallest possible footprint on your machine and you primarily build PHP applications, Valet is hard to beat.

Valet is also **MIT-licensed**, which has fewer restrictions than AGPL-3.0 if software licensing matters to your organization or project.

Valet has a large, established community, extensive driver ecosystem for PHP frameworks, and years of Stack Overflow answers. If you run into an edge case with a specific PHP framework, the odds of finding a Valet solution are high.

Valet's configuration being based on directory parking is familiar and simple. No GUI to learn, no JSON registry — just `valet park` in your projects folder.

## Choose Laravel Valet when

- You want the smallest possible local tool for PHP and are comfortable with the CLI.
- MIT licensing is important for your context.
- You only ship PHP and benefit from the established Valet driver ecosystem.
- You want something that installs as Homebrew dependencies and stays out of your way.

## Choose PortBay when

- You mix Node, PHP, and static projects and need one tool to cover all of them.
- A GUI and menu-bar visibility across projects matters for your team or workflow.
- You want bundled database management (MySQL, Postgres, Redis) without a separate setup step.
- MCP server support for AI-assisted development is on your list — or a task board your agents work cards from.
- Cloudflare tunnel integration for public sharing in one click is useful.
- You want a JSON-based registry for portable, version-controlled project config.

## Bottom line

Valet is a perfectly good, time-tested choice for PHP-only macOS development, and its simplicity is a genuine virtue. PortBay extends that model with multi-runtime support, a GUI, bundled databases, and automation primitives for developers who need more than a PHP proxy. [Install PortBay](/getting-started/install) to see how far beyond Valet it goes.

---

See all [comparisons](/comparisons/) — developers also compare PortBay with [Laravel Herd](/comparisons/portbay-vs-laravel-herd) and [DDEV](/comparisons/portbay-vs-ddev).
