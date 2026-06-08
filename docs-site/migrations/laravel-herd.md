---
title: Migrate from Laravel Herd to PortBay
description: Import your Laravel Herd sites into PortBay automatically — PortBay reads Herd's config.json and parked-path directories and registers each site as a PHP project.
---

# Migrating From Laravel Herd

PortBay can read Laravel Herd's config file and register your existing sites directly. The importer reads Herd's `config.json` (explicit `sites` entries and `parked_paths` directories), maps each site to a PortBay PHP project, and writes the result to PortBay's registry. No files are moved; both tools can coexist on the same machine after import.

**What gets carried over:** site path, hostname, PHP version, HTTPS flag, and a `source:herd` tag on every imported project.

## Before You Start

| Check | Detail |
| --- | --- |
| Herd config location | `~/Library/Application Support/Herd/config.json` must exist. PortBay probes this path; if it is absent, Herd will show as "not installed" in the import panel. |
| PortBay project cap | Free accounts: up to 6 projects total. Importing stops at the cap and lists skipped sites. Pro accounts have no cap. |
| Source app state | You do not need to quit Herd before running the import — PortBay reads config files only and never calls any Herd binary. However, both tools listen on port 80/443 by default, so running both at the same time will cause a port conflict. Plan to switch one off before starting projects in PortBay. |

## Run the Import

### In the app

1. Open **Settings** (sidebar or menu bar).
2. Select the **Advanced** tab.
3. Scroll to the **Import from another tool** card.
4. PortBay scans installed sources automatically. Laravel Herd will appear with a site count if `config.json` is found.
5. Click **Preview sites** next to the Herd row.
6. A checklist appears with every detected site. Rows flagged **id taken** or **path in use** already exist in PortBay; they are unchecked by default.
7. Adjust the selection, then click **Import N sites**.
8. PortBay writes the new projects to the registry and refreshes the project list. Herd's config is not modified.

### CLI alternative

```bash
# See what is installed and how many sites each source exposes.
portbay import sources

# Preview Herd sites against your current registry (flags collisions).
portbay import preview herd

# Import all Herd sites (stops at your plan cap).
portbay import projects herd --all

# Import specific sites by id.
portbay import projects herd myapp my-api
```

The CLI writes to the same registry the app reads. If the app is open, wait a few seconds for it to pick up the change (the reconciler polls the registry).

## What Gets Imported

| Herd concept | PortBay field | Notes |
| --- | --- | --- |
| `sites[].path` | `path` | Absolute path as recorded in `config.json`. |
| `sites[].tld` (or global `tld`) | Appended to `hostname` | Defaults to `test` when not set. |
| `sites[].alias` | `hostname` | When an alias is present it becomes the full hostname; otherwise PortBay uses `<folder-name>.<tld>`, lowercased. |
| `sites[].php_version` (or global `php_version`) | `php_version` | Per-site value wins over the global default. |
| `sites[].secure` | `https` | `true` when the site is secured in Herd. PortBay issues a new mkcert certificate rather than reusing Herd's certificate. |
| `parked_paths[]` | Expanded to one project per sub-directory | Each child directory becomes a site using the global `tld` and `php_version`. Parked sites default to `https: false` because per-site secure overrides are not tracked in the parked list. |
| (any site with a PHP version or an `index.php` entry point) | `kind: php`, `services: [caddy, php-fpm]` | Non-PHP entries map to `kind: custom`. |

The importer sets the web server to **Caddy** for all Herd sites. Herd itself uses Caddy internally, so this is consistent.

Every imported project receives the tag `source:herd`, visible in the project detail panel.

## What Isn't Imported

- **Queue workers, schedulers, and horizon processes.** Herd manages these as separate worker processes. PortBay has no direct equivalent today; you will need to add them as separate projects or start them manually.
- **Database contents or configuration.** Herd bundles MySQL/MariaDB and Redis via Homebrew or its own sidecar. PortBay manages its own database containers (see the Databases guide); you will need to re-create any databases and import data separately.
- **Per-site environment variables.** Herd does not persist `.env` overrides in `config.json`; they live in the project's own `.env` file, which PortBay will pick up naturally when the project starts.
- **Mailpit / mail-catching configuration.** Configure PortBay's built-in Mailpit sidecar separately.
- **Pro/Herd-specific features** (remote servers, custom PHP extensions, license key). None of these map to PortBay registry fields.

## After the Import

1. Open the Projects view and confirm the imported sites appear with the correct hostname and PHP version.
2. Select a project and click **Start**. PortBay provisions a mkcert certificate and starts Caddy and PHP-FPM.
3. Open the project URL in a browser to verify it loads.
4. If you plan to keep using PortBay going forward, stop Herd (or disable its autostart) to free ports 80 and 443.

If a project fails to start, check the log viewer. Common causes after a Herd migration are a missing PHP version (install it via the Languages page) or a document root mismatch (the importer does not set `document_root`; Laravel projects expect `public` — set it in the project detail panel).
