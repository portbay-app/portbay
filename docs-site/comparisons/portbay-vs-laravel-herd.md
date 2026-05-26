---
title: "PortBay vs Laravel Herd: open-source alternative"
description: Compare PortBay and Laravel Herd for local macOS development. Covers open-source vs closed, multi-runtime vs PHP-first, and when each tool is the better Laravel alternative.
---

# PortBay vs Laravel Herd

PortBay is an open-source (AGPL-3.0), container-free local dev manager for macOS that runs Node, PHP, and static sites side by side. Laravel Herd is a polished, closed-source local PHP environment for macOS and Windows with an excellent first-party Laravel and PHP workflow. If you ship PHP exclusively and want the smoothest possible Laravel DX out of the box, Herd is a serious option. If you run Node alongside PHP, or open-source matters, PortBay is the stronger fit.

## At a glance

| | PortBay | Laravel Herd |
|---|---|---|
| License | AGPL-3.0 (open source) | Closed source |
| Price | Free · optional Pro | Free tier / paid Pro |
| Containers | None (native) | None (native) |
| Runtimes | Node, PHP, static | PHP-first (PHP, binaries) |
| Local HTTPS + `.test` | Built in (mkcert) | Built in |
| Managed DNS | Bundled dnsmasq | Bundled |
| Reverse proxy | Caddy (automatic) | Nginx (automatic) |
| Footprint | Small (native) | Small (native) |
| Platform | macOS (Apple Silicon) | macOS + Windows |
| Automation | CLI + MCP | CLI (Herd CLI) |

## What they share

Both tools are container-free and run your projects natively on the host — no Docker overhead. Both provide automatic `.test` hostnames with real HTTPS certificates so you never touch `/etc/hosts` by hand. Both aim to get a project running with minimal configuration, and both support PHP-based frameworks including Laravel.

## Where PortBay is different

PortBay is **open source**: the full codebase is on GitHub under AGPL-3.0, so you can read it, fork it, or contribute. Herd is proprietary.

PortBay is **multi-runtime from the ground up**. You can run a Next.js front-end, a PHP/Laravel API, and a static marketing site in the same session, each under its own `.test` hostname, all managed by a single tool. Herd's runtime story is PHP-first; Node support exists but is not the primary use case.

PortBay's reverse proxy is Caddy, managed through its admin API and driven by a declarative JSON registry. Herd uses Nginx. Both are automatic, but PortBay exposes the proxy layer via CLI and an MCP server, making it scriptable by AI agents and CI workflows.

Pro access in PortBay is earned by a donation or a merged pull request — it is a perpetual license, not a subscription.

## Where Laravel Herd is stronger

Herd has a deeply polished PHP developer experience. PHP version switching, Herd-specific binaries (php, composer, node binaries managed through Herd's path), and tight integration with the Laravel ecosystem are first-class. If your day is Laravel artisan commands and Tinker sessions, Herd's UX is hard to beat.

Herd also runs on **Windows**, which PortBay does not yet support. If you need to share a local dev setup with Windows teammates, Herd works cross-platform today.

Herd's paid Pro tier offers extras like Expose (public tunnels), Valet-compatible driver ecosystem, and dedicated Laravel support. For PHP shops that want those specifically, Pro is worth considering.

## Choose Laravel Herd when

- Your entire stack is PHP / Laravel and you want the most refined PHP DX available on macOS or Windows.
- You need Windows support today.
- You want first-party integration with Expose, Herd-managed PHP binaries, and the broader Laravel ecosystem.
- Open-source licensing is not a requirement for local tooling.

## Choose PortBay when

- You mix Node (Next.js, Vite, plain Node APIs) with PHP projects and need one tool to manage both.
- Open source matters — you want to read the code, contribute, or run a fork.
- You want CLI-first automation and MCP server support for AI-assisted workflows.
- You prefer Caddy's declarative config model over Nginx.
- Pro access via contribution (not subscription) fits your preferences.

## Bottom line

Laravel Herd is excellent for PHP-only macOS development, and this page says so plainly. PortBay is the better call if you run a mixed stack, value open source, or want deeper automation primitives. [Install PortBay](/getting-started/install) and have your first project running in minutes.

---

See all [comparisons](/comparisons/) — developers also compare PortBay with [Laravel Valet](/comparisons/portbay-vs-laravel-valet) and [ServBay](/comparisons/portbay-vs-servbay).
