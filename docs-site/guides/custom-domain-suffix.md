# Custom Domain Suffix

PortBay defaults to local hostnames such as `project.test`. The suffix is part of each project hostname stored in the registry.

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
