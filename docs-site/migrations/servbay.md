# Migrating From ServBay

This is a release placeholder. The importer exists in the codebase, but the public migration workflow will be documented after the import UX is finalized.

## Inventory First

Record the following from ServBay before moving a project:

| Item | Why it matters |
| --- | --- |
| Site root | Becomes PortBay `path`. |
| Hostname | Becomes PortBay `hostname`. |
| PHP version | Becomes `php_version`. |
| Document root | Becomes `document_root`. |
| Databases and mail tooling | Map to PortBay services or external tools. |
| Environment variables | Move into registry env or `.portbay.json` templates. |

## Migration Shape

1. Stop the ServBay site.
2. Add the project in PortBay.
3. Match hostname, PHP version, document root, and HTTPS setting.
4. Reconcile local DNS or hosts.
5. Start in PortBay and verify the local URL.
