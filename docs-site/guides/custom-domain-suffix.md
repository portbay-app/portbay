---
title: Custom Local Domain Suffix — PortBay .test & .localhost
description: Change your PortBay project hostnames from .test to .localhost or a custom suffix — how to edit the registry, reconcile dnsmasq, and avoid stale /etc/hosts entries.
---

# Custom Domain Suffix

PortBay defaults to local hostnames such as `project.test`. The suffix is part of each project hostname stored in the registry.

![PortBay domains — one row per project hostname](/screenshots/domains.png)

## Current Behavior

The active registry stores full hostnames, not a global hostname template. That means a project’s suffix is changed by editing the project hostname:

```json
{
  "id": "marketing-site",
  "hostname": "marketing-site.localhost"
}
```

## Choosing A Suffix

| Suffix | Recommendation |
| --- | --- |
| `.test` | Good default for local development. Reserved for testing. |
| `.localhost` | Safe local-only suffix in modern browsers. |
| Company-internal suffix | Use only when it cannot collide with real DNS. |

## Change Procedure

1. Stop the project.
2. Update the hostname in the app or registry.
3. Reconcile hosts or dnsmasq.
4. Restart Caddy.
5. Start the project and open the new URL.

Changing the suffix without reconciling hostnames leaves stale entries behind.

## Local DNS

PortBay routes exact hostnames through the privileged `/etc/hosts` helper and resolves wildcard `*.<suffix>` through the bundled dnsmasq sidecar. The DNS view shows resolver status, the managed records, and the cache tuning.

![PortBay local DNS](/screenshots/dns.png)

## Via MCP (agent-driven)

When driving PortBay through an AI agent, three tools cover DNS and domain-suffix tasks:

- **`portbay_dns_status`** — read the active suffix, whether the `/etc/resolver/<suffix>` file is installed, the dnsmasq port it targets, and the persisted dnsmasq tuning. Starting or restarting dnsmasq is done from the app.
- **`portbay_list_dns_records`** — list every name PortBay resolves (the wildcard plus one row per project hostname), each tagged with whether it's routed via `dnsmasq` or `/etc/hosts`.
- **`portbay_set_domain_suffix`** — change the suffix for every project at once. This is a high-blast-radius operation: it rewrites all project hostnames and drops their HTTPS cert directories (the app reissues certs on reconcile). Reserved public TLDs are rejected. Confirm with the user before calling.

See the [Tool Reference](../agents/tools.md) for the full argument and return-type details.
