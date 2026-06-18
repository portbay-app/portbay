---
title: "PortBay vs MAMP / XAMPP: a modern replacement"
description: Compare PortBay and MAMP or XAMPP for local development. Modern open-source tool vs classic PHP/Apache stacks — find the right MAMP alternative for your workflow.
---

# PortBay vs MAMP / XAMPP

PortBay is an open-source (AGPL-3.0), container-free local dev manager for macOS that covers Node, PHP, and static sites with automatic HTTPS and managed DNS. MAMP (macOS, Apache, MySQL, PHP) and XAMPP (cross-platform Apache, MariaDB, PHP, Perl) are classic AMP stacks that have been the default choice for PHP local development for many years. If your team has deep familiarity with MAMP/XAMPP and you only need PHP, they still work. If you want modern HTTPS-by-default, multi-runtime support, and CLI automation, PortBay is a substantial upgrade.

## At a glance

| | PortBay | MAMP / XAMPP |
|---|---|---|
| License | AGPL-3.0 (open source) | MAMP: closed (free + paid Pro) · XAMPP: Apache Friends (free) |
| Price | Free · optional Pro | MAMP: free / paid Pro · XAMPP: free |
| Containers | None (native) | None (native) |
| Runtimes | Node, PHP, static | PHP-first (Apache + PHP + MySQL) |
| Local HTTPS + `.test` | Built in (mkcert) | Manual / limited (MAMP Pro has partial support) |
| Managed DNS | Bundled dnsmasq | None (manual `/etc/hosts`) |
| Reverse proxy | Caddy (automatic) | Apache (manual vhosts) |
| Footprint | Small (native) | Medium |
| Platform | macOS (Apple Silicon) | MAMP: macOS + Windows · XAMPP: cross-platform |
| Automation | CLI + MCP (66 tools) | Limited CLI |
| AI agent task board | ✅ Markdown cards + handoff memory | ❌ |

## What they share

All three tools are container-free and run your app processes natively on the host. All three bundle MySQL/MariaDB. All three let you serve PHP applications locally without Docker. For basic PHP + MySQL use cases, any of them can get a site running.

## Where PortBay is different

PortBay was built with **HTTPS-first** as a default. Every project gets a real `.test` hostname, a mkcert certificate, and wildcard DNS via bundled dnsmasq. With MAMP or XAMPP you are typically on `localhost:8888` or editing `/etc/hosts` manually. MAMP Pro offers some HTTPS support, but it is not the out-of-box default.

PortBay is **multi-runtime**. You run Node, PHP, and static sites under the same tool, each behind the same Caddy proxy, each with its own hostname. MAMP and XAMPP are Apache-centric PHP stacks; Node is outside their model.

The **JSON registry** makes project configuration version-controllable. The full CLI and MCP server let you automate starts, stops, and rebuilds from scripts or AI agents — none of which is possible with a GUI-only tool.

And PortBay belongs to a different era of tooling entirely on one axis: it puts **AI coding agents to work**. Every project gets a task board whose cards are Markdown files in your repo — move a card to *To Do* and the agent you assigned (Claude Code, Codex, Cursor, Gemini, and more) picks it up, does the work on your machine, and leaves a handoff brief for the next run. MAMP and XAMPP predate this workflow and have no answer to it.

PortBay is **open source**. MAMP's core app is closed. XAMPP is from the Apache Friends project but is not itself open-source in the same sense as an auditable codebase under active community development.

## Where MAMP / XAMPP is stronger

MAMP and XAMPP have been around for over a decade and have extensive community documentation, tutorials, and forum threads covering common issues. If you are troubleshooting an obscure PHP module or Apache config, that accumulated knowledge base is valuable.

XAMPP is **cross-platform** (macOS, Windows, Linux) and is a known quantity in educational and shared-team contexts. MAMP Pro offers Windows support. If your team works across operating systems and needs the same local stack on all of them today, these tools have wider OS coverage than PortBay.

For developers maintaining legacy PHP applications that depend on specific Apache behaviors or htaccess directives, MAMP/XAMPP's Apache stack is a closer match to shared-hosting production environments.

## Choose MAMP / XAMPP when

- You need cross-platform parity today (Windows, Linux teammates) and cannot wait for PortBay's roadmap.
- Your application depends on Apache-specific behavior (htaccess rewrites, `.htpasswd`, mod_rewrite edge cases) that differs from Caddy.
- You are working in a legacy PHP environment and the existing tutorial / community ecosystem for MAMP is a genuine time-saver.
- You want a setup that a non-terminal-comfortable developer can use with no CLI knowledge.

## Choose PortBay when

- You want real `.test` HTTPS out of the box without editing `/etc/hosts` or paying for MAMP Pro.
- You run Node projects alongside PHP and need one tool to manage both.
- CLI-first workflows, scripting, and MCP automation matter to you.
- You work with AI coding agents and want a board that dispatches tasks to them instead of pasting prompts into a terminal.
- Open source matters — you want to see the code and contribute.
- You want modern tooling that isn't built around Apache configuration files.

## Bottom line

MAMP and XAMPP are familiar choices with years of community knowledge behind them, but they show their age when it comes to HTTPS, DNS, and multi-runtime support. PortBay is the modern, open-source path forward for macOS developers. [Install PortBay](/getting-started/install) and have a project running on a real `.test` domain in a few minutes.

---

See all [comparisons](/comparisons/) — developers also compare PortBay with [Laravel Herd](/comparisons/portbay-vs-laravel-herd) and [Docker / OrbStack](/comparisons/portbay-vs-docker).

_Last updated: 2026-06-15._
