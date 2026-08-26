---
title: PortBay Domains — Per-Project Routing & Hostname Settings
description: The Domains page — path prefixes, resolver mode, certificate source, wildcard subdomains, and expose-when-running per project hostname.
---

# Domains

<ThemeImage name="domains" alt="PortBay domains — one row per project hostname" />

A "domain" in PortBay is a project's hostname — there's one row per project, not a separate domain registry. The **Domains** page is a side-by-side editor: the left rail lists every domain with a filter and pagination, the right pane edits the selected project's routing settings inline. Every field writes through the same `update_project` command the rest of the app uses.

This page is about **per-project** routing and hostname behavior. To change the shared suffix every project's hostname ends in (`.test` → `.localhost`, or a custom suffix), see [Custom Domain Suffix](/guides/custom-domain-suffix) instead — that's a Pro, whole-registry operation with its own re-pin and cert-migration steps.

## Routing Settings

Each project's domain configuration is additive — a project created before these settings existed behaves exactly as it always did until you touch them:

| Setting | Effect |
| --- | --- |
| **Notes** | Free-text note shown on this page. No runtime effect. |
| **Path prefix** | A URL path prefix stripped before proxying upstream (Caddy `handle_path`). Empty or `/` serves from the root — today's default behavior. |
| **Resolver mode** | `Auto` (default), `Hosts`, or `Dnsmasq` — which resolution mechanism this hostname prefers. See [Networking & DNS](/guides/networking) for how the two mechanisms differ. |
| **Auto-manage certificate** | Whether PortBay issues/renews this hostname's certificate automatically. Defaults on. |
| **SSL mode** | Automatic (local), Custom certificate, Self-signed, or Public ACME (placeholder). See [Certificates](/guides/certificates). |
| **Include wildcard subdomains** | Also routes and certifies `*.hostname`. Only resolves through the dnsmasq wildcard resolver — `/etc/hosts` can't express a wildcard entry. |
| **Expose when running** | Only publishes the Caddy route while the project's process is actually running, instead of always. |

## Public ACME

The `PublicAcme` SSL mode carries its own issuer/environment/challenge configuration (DNS provider, key type, EAB credentials where required) for issuing a real publicly-trusted certificate for a domain you control. This is placeholder scaffolding for future public-domain automation and is not enabled for the local `.test` suffix.

## Via MCP (Agent-Driven)

There is no dedicated "domains" toolset. `portbay_update_project` covers the basics — hostname and HTTPS on/off — but the granular routing knobs on this page (path prefix, resolver mode, SSL mode particulars, wildcard subdomains, expose-when-running) are not yet exposed through MCP; set them from the Domains page in the app. Certificate state for a hostname is readable via `portbay_cert_info` and can be force-reissued via `portbay_reissue_cert` (see [Certificates](/guides/certificates#via-mcp-agent-driven)); DNS routing state (which mechanism is actually resolving a hostname right now) is covered by the DNS toolset (see [Networking & DNS](/guides/networking#via-mcp-agent-driven)).
