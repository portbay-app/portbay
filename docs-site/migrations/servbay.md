---
title: Migrate from ServBay to PortBay — Sites, PHP & DNS
description: Import your ServBay sites into PortBay — PortBay reads ServBay's NGINX vhost files, maps each server block to a PHP or static project, and splits document-root sub-directories automatically.
---

# Migrating From ServBay

PortBay reads ServBay's NGINX vhost config files and registers each enabled site as a PortBay project. The importer scans the vhost directories on disk, parses each `server { }` block, and writes matching projects to PortBay's registry. ServBay's config files are not modified.

**What gets carried over:** hostname, document root (with automatic `public`/`web`/`dist` splitting), HTTPS state, and whether the site is PHP or static.

## Before You Start

| Check | Detail |
| --- | --- |
| ServBay vhost directories | The importer scans these directories (all that exist are read): `~/Library/Application Support/ServBay/vhosts/`, `~/Library/Application Support/ServBay/disabled-vhosts/`, `/Applications/ServBay/etc/nginx/manual-vhosts/`, `/Applications/ServBay/etc/nginx/enabled-dev-vhosts/`, `/Applications/ServBay/etc/nginx/sites/`, `/Applications/ServBay/etc/nginx/sites-enabled/`. If none of these exist, ServBay shows as "not installed". |
| PHP version | ServBay does not record the PHP version in vhost files. The import creates PHP projects without a pinned version; PortBay will fall back to the first PHP version it detects on the machine. Set the exact version in the project detail panel after import. |
| PortBay project cap | Free accounts: up to 6 projects total. Importing stops at the cap and lists skipped sites. Pro accounts have no cap. |
| Source app state | PortBay reads config files only and never calls any ServBay binary. You do not need to quit ServBay first. However, both tools listen on port 80/443, so running both at the same time causes a port conflict. |

## Run the Import

### In the app

1. Open **Settings** (sidebar or menu bar).
2. Select the **Advanced** tab.
3. Scroll to the **Import from another tool** card.
4. PortBay scans installed sources automatically. ServBay will appear with a site count if any vhost directory is found.
5. Click **Preview sites** next to the ServBay row. A note reads "uses NGINX vhost format".
6. A checklist shows every detected site. Rows flagged **id taken** or **path in use** already exist in PortBay and are unchecked by default.
7. Adjust the selection and click **Import N sites**.
8. PortBay writes the projects to the registry and refreshes the project list.

### CLI alternative

```bash
# See what is installed and how many sites each source exposes.
portbay import sources

# Preview ServBay sites flagged for collisions.
portbay import preview servbay

# Import all ServBay sites.
portbay import projects servbay --all

# Import specific sites by id.
portbay import projects servbay myapp tribal-house-cms
```

## What Gets Imported

| ServBay concept | PortBay field | Notes |
| --- | --- | --- |
| `server_name` in vhost | `hostname` | First token after `server_name`. Wildcard names (e.g. `*.servbay.demo`) are skipped — they are ServBay's catch-all, not real projects. |
| `root` in vhost | `path` + `document_root` | If the root ends in a conventional sub-directory (`public`, `web`, `html`, `public_html`, `www`, `dist`, `build`, `out`), PortBay splits it: the parent becomes `path` and the leaf becomes `document_root`. A root of `/Users/x/myapp/public` imports as path `/Users/x/myapp`, document root `public`. |
| `listen 443 ssl` | `https: true` | HTTP-only vhosts import as `https: false`; PortBay issues a fresh mkcert certificate when HTTPS is enabled later. |
| `include …php-fpm…` or any `.php` reference in the block | `kind: php`, `services: [caddy, php-fpm]` | The importer detects PHP-FPM includes and `.php` references (try_files, index, router.php) to identify PHP apps. Vhosts with neither are imported as `kind: static`. |
| Reverse-proxy vhosts (`proxy_pass`, no `root`) | **Skipped** | The importer requires a `root` directive. Vhosts that proxy to another process (e.g. a Node dev server) have no `root` and are excluded. |
| PHP version | Not imported | ServBay vhost files do not contain the PHP version. Set it manually in the project detail panel after import. |

The importer assigns **Nginx** as the web server hint for all ServBay projects (reflecting the source configuration). You can change this to Caddy in the project detail panel.

Every imported project receives the tag `source:servbay`.

## What Isn't Imported

- **PHP version.** Not stored in vhost files. PortBay falls back to the first PHP version it detects on your machine; set it explicitly after import.
- **Database contents or credentials.** ServBay bundles its own MySQL, PostgreSQL, Redis, and other databases. PortBay manages separate database containers; you will need to re-create databases and import data independently.
- **Environment variables.** Per-project `.env` files are not read during import; they remain in place and will be picked up by your application normally when the project starts.
- **ServBay-specific extensions, custom PHP builds, or SSL certificates.** PortBay uses mkcert for local HTTPS and manages its own PHP-FPM sidecars; existing ServBay certificates are not reused.
- **Disabled vhosts.** Files in `disabled-vhosts/` are scanned and included in the import preview, but they import with the same settings as enabled ones. Whether you start them in PortBay is up to you.
- **Mail catching configuration.** Configure PortBay's built-in Mailpit sidecar separately.

## After the Import

1. Open the Projects view. Confirm imported projects show the correct hostname and document root.
2. For PHP projects, open the project detail panel and set the **PHP version** explicitly (the importer cannot read this from ServBay's vhost files).
3. Click **Start** on a project. PortBay provisions a mkcert certificate and starts Caddy and PHP-FPM.
4. Visit the project URL in a browser to verify it loads.
5. If you plan to use PortBay going forward, stop ServBay (or disable its startup item) to free ports 80 and 443.

If a project fails to start, check the log viewer. The most common cause after a ServBay migration is a PHP version not installed in PortBay. Install missing versions via **Settings → Languages**.
