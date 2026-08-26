---
title: PortBay Config-as-Code — portbay.yml Reference
description: The in-repo portbay.yml file that makes a PortBay project reproducible. Commit it and a teammate can clone, run portbay add, and get an identical local environment — hostname, runtime, HTTPS, env, and database wiring — with no manual steps.
---

# Config-as-Code (`portbay.yml`)

`portbay.yml` is the file that makes a PortBay project **reproducible across a team**. Commit it to your repo root and a teammate can clone, run one command, and get an identical local environment — same hostname, runtime version, HTTPS mode, env, and database wiring — with no manual setup and no re-detection surprises.

It is the in-repo analogue to Herd's `herd.yml`, DDEV's `.ddev/config.yaml`, and Lando's `.lando.yml`.

> **`portbay.yml` vs `.portbay.json`.** The [Portfile](/reference/registry-schema#portable-portbayjson) (`.portbay.json`) is a *share* export — a JSON snapshot you hand to someone. `portbay.yml` is *config-as-code*: YAML, hand-editable, and committed to the repo so `portbay add .` reproduces the setup automatically. When both exist, `portbay.yml` wins.

## Location & format

A single file at the project root, next to `package.json` / `composer.json`:

```text
my-app/
  portbay.yml   ← here
  package.json
```

YAML, `snake_case` keys. The generated file carries a `# yaml-language-server` schema comment so editors (VS Code's YAML extension) validate and auto-complete it against the [published JSON Schema](https://portbay.app/schema/portbay.schema.json).

## Example

```yaml
# yaml-language-server: $schema=https://portbay.app/schema/portbay.schema.json
version: 1
name: Blog
type: php
framework: laravel
hostname: blog.test
https: true
ssl_mode: automatic_local
runtime:
  lang: php
  version: "8.3"
document_root: public
port: 8000
env:
  APP_ENV: local
  APP_URL: https://blog.test
secrets:
  - APP_KEY          # name only — the value is never committed
databases:
  - engine: mysql
    name: blog_dev
groups:
  - Marketing
tags:
  - client
pre_start:
  - composer install
```

## Fields

| Field | Type | Required | Notes |
| --- | --- | --- | --- |
| `version` | integer | No | Schema version. Defaults to `1`. |
| `name` | string | Yes | Human-readable label. |
| `type` | string | Yes | Project kind (`next`, `vite`, `php`, `python`, `static`, `node`, `laravel`-style meta-frameworks, …). |
| `framework` | string | No | Sub-stack override (brand logo/label only). Re-detected from the folder when omitted. |
| `hostname` | string | Yes | Full local hostname (e.g. `blog.test`). |
| `https` | boolean | Yes | Whether Caddy terminates local TLS. |
| `ssl_mode` | string | No | `automatic_local` (default, mkcert), `custom_certificate`, `self_signed`, `public_acme`. |
| `runtime` | object | No | `{ lang, version }` — the pinned language toolchain. Omit to inherit the language default. |
| `start_command` | string | No | Shell command run in the project path. |
| `document_root` | string | No | PHP document root relative to the path (e.g. `public`). |
| `port` | integer | No | Primary HTTP port. |
| `env` | object | No | Non-secret env vars. Values may use `${PROJECT_PATH}` / `${PROJECT_NAME}` placeholders. |
| `secrets` | string[] | No | Env-var **names** that are secrets — prompted on import, never committed. |
| `databases` | object[] | No | `{ engine, name }` per required database. Provisioned against a running instance of the engine. |
| `groups` | string[] | No | Group names to join. Created on import if missing. |
| `auto_start` | boolean | No | Start automatically when the daemon comes up. Defaults `false`. |
| `tags` | string[] | No | Filtering / grouping labels. |
| `pre_start` | string[] | No | Commands run before the dev server each start. |
| `post_start` | string[] | No | Commands run after the dev server reports ready. |

## Secrets are never stored

Sensitive env vars (`APP_KEY`, `*_PASSWORD`, `*_TOKEN`, `*_SECRET`, …) are split out on export: only their **names** land in `secrets:`, never their values. On import PortBay prompts for the values (GUI) or leaves them blank for you to fill (CLI). Commit the file freely.

## Databases

A `databases:` entry declares that the project needs a database of a given engine, provisioned under `name`. On import, PortBay links the project to a matching engine instance (injecting `DATABASE_URL` / `DB_*` at launch) and — when the instance is running — provisions a dedicated database + user via the same path as the in-app "Provision database" action. If no instance of that engine exists yet, PortBay tells you to create one and re-link; nothing is silently skipped.

## Workflow

### Author the file

From a registered project, export its exact setup:

```bash
portbay init                 # writes ./portbay.yml from the registry entry
portbay init path/to/project
```

Run in a folder that isn't registered yet and `portbay init` scaffolds the file from framework detection instead. Pass `--force` to overwrite an existing file. In the app, the project detail panel has an **Export to portbay.yml** button.

### Reproduce on another machine

```bash
git clone <repo> && cd <repo>
portbay add .                # reads portbay.yml, registers an identical project
```

`portbay add` prefers `portbay.yml`, then `.portbay.json`, then falls back to detection. CLI flags (`--id`, `--name`) still override the file.

### Drift

When a registered project's committed file no longer matches its registry entry (someone edited settings in the app, or pulled a teammate's change), PortBay surfaces a **config drift** prompt listing exactly which fields differ, so you can *adopt the file* or *keep the registry*. Drift is semantic — reordering `env` keys or `tags` is never flagged.

## Out of scope

`portbay.yml` deliberately does **not** cover: cloud sync of the file, secret *management* (only names are recorded), per-service configuration beyond database requirements, or Windows.
