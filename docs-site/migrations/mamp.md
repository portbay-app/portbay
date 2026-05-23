# Migrating From MAMP

This is a release placeholder. Full MAMP import documentation will land after the import UX is finalized.

## What To Capture

| MAMP setting | PortBay field |
| --- | --- |
| Document root | `path` or `document_root` |
| Hostname | `hostname` |
| PHP version | `php_version` |
| Apache/Nginx port | Project `port` only if the app process listens directly |
| SSL certificate | Reissued through mkcert |

## Migration Shape

1. Stop the MAMP site.
2. Add the project in PortBay as PHP.
3. Set `document_root` and `php_version`.
4. Enable HTTPS.
5. Reconcile hostnames.
6. Start and test the site through Caddy.
