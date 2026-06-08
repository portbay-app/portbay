---
title: Migrate from MAMP to PortBay — PHP Sites & HTTPS
description: Import your MAMP virtual hosts into PortBay — PortBay parses MAMP's Apache httpd-vhosts.conf and registers each VirtualHost block as a project, mapping ServerName and DocumentRoot directly.
---

# Migrating From MAMP

PortBay parses MAMP's Apache vhost config file and registers each defined site as a PortBay project. The importer reads `ServerName` and `DocumentRoot` from every `<VirtualHost>` block and writes matching projects to PortBay's registry. MAMP's config file is not modified.

**What gets carried over:** hostname, document root path (used directly as the project path), and whether the site is on port 443/HTTPS.

## Before You Start

| Check | Detail |
| --- | --- |
| MAMP vhosts config location | `/Applications/MAMP/conf/apache/extra/httpd-vhosts.conf` must exist. PortBay checks this exact path; MAMP Pro or non-default install locations are not probed. |
| PHP version | MAMP's Apache vhosts do not record the PHP version. Projects import without a pinned PHP version; set it in the project detail panel after import. |
| PortBay project cap | Free accounts: up to 6 projects total. Importing stops at the cap and lists skipped sites. Pro accounts have no cap. |
| Source app state | PortBay reads the config file only and never calls any MAMP binary. You do not need to quit MAMP first. However, both tools will compete for port 80/443 if run simultaneously. |

## Run the Import

### In the app

1. Open **Settings** (sidebar or menu bar).
2. Select the **Advanced** tab.
3. Scroll to the **Import from another tool** card.
4. PortBay scans installed sources automatically. MAMP will appear with a site count if the vhosts file is found. A note reads "uses Apache httpd-vhosts.conf".
5. Click **Preview sites** next to the MAMP row.
6. A checklist shows every detected `<VirtualHost>` block. Rows flagged **id taken** or **path in use** already exist in PortBay and are unchecked by default.
7. Adjust the selection and click **Import N sites**.
8. PortBay writes the projects to the registry and refreshes the project list.

### CLI alternative

```bash
# See what is installed and how many sites each source exposes.
portbay import sources

# Preview MAMP sites flagged for collisions.
portbay import preview mamp

# Import all MAMP sites.
portbay import projects mamp --all

# Import specific sites by id.
portbay import projects mamp myapp my-api
```

## What Gets Imported

| MAMP / Apache concept | PortBay field | Notes |
| --- | --- | --- |
| `ServerName` | `hostname` | Used as-is. `VirtualHost` blocks without a `ServerName` are skipped. |
| `DocumentRoot` | `path` | The document root becomes the project `path`. Paths quoted with double quotes (common for paths with spaces) are unquoted automatically. Blocks without a `DocumentRoot` are skipped. |
| `<VirtualHost *:443>` or `SSLEngine on` | `https: true` | Either a `:443` port in the opening tag or `SSLEngine on` inside the block sets HTTPS. PortBay issues a fresh mkcert certificate; MAMP's SSL certificates are not reused. |
| PHP version | Not imported | Apache vhost blocks do not contain the PHP version. PortBay falls back to the first PHP version it detects on the machine. |
| Project type | `kind: custom` (no PHP version heuristic available) | The MAMP importer does not set a PHP version, so `kind` defaults to `custom`. After import, open the project detail panel and set `kind: php` and the PHP version if the site is a PHP app. |

The importer assigns **Apache** as the web server hint for all MAMP projects, consistent with MAMP's default setup.

Every imported project receives the tag `source:mamp`.

## What Isn't Imported

- **PHP version.** Not stored in Apache vhost files. Set it manually in the project detail panel after import, and change the project type to PHP.
- **Database contents or credentials.** MAMP bundles its own MySQL/MariaDB. PortBay manages separate database containers; re-create databases and import data independently.
- **Environment variables.** Per-project `.env` files are not read during import; they remain in place and your application picks them up on start.
- **MAMP ports configuration.** MAMP defaults to port 8888/8889 when not running as root; the vhosts file may still show `*:80`/`*:443`. PortBay always serves on 80/443 through Caddy. Check your application's environment variables if it hard-codes a port number.
- **MAMP Pro vhost features** (aliases, redirects, custom log paths, per-vhost PHP settings, `.htaccess` directives). The importer reads only `ServerName`, `DocumentRoot`, and the HTTPS indicators; everything else is ignored.
- **Mail catching configuration.** Configure PortBay's built-in Mailpit sidecar separately.
- **Non-standard install paths.** Only `/Applications/MAMP/` is probed. MAMP installations elsewhere on disk will not be detected.

## After the Import

1. Open the Projects view and confirm imported sites show the correct hostname.
2. For PHP projects, open the project detail panel, set **Project type** to PHP, and set the **PHP version** explicitly.
3. Click **Start**. PortBay provisions a mkcert certificate and starts Caddy (and PHP-FPM for PHP projects).
4. Visit the project URL in a browser to verify it loads.
5. If you plan to use PortBay going forward, stop MAMP (or disable its autostart) to free ports 80 and 443.

If a project fails to start, the most common causes after a MAMP migration are: a missing PHP version (install via **Settings → Languages**), a document root mismatch (MAMP's `DocumentRoot` pointed at the PHP entry directory, which may differ from what PortBay expects — adjust `document_root` in the project detail panel), or a port conflict with MAMP still running.
