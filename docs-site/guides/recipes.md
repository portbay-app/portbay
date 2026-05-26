---
title: PortBay Stack Recipes — One-Step Project Configuration
description: Use PortBay stack recipes to register Laravel, Next.js, Vite, Symfony, and more in one MCP tool call — pre-filled PHP version, document root, HTTPS, and database hints.
---

# Stack Recipes

A recipe is a named blueprint that configures a project's framework type, PHP version, document root, and HTTPS setting in one step. Pass a recipe id to `portbay_setup_from_recipe` and PortBay applies the blueprint to an existing folder deterministically — no model or guesswork on the PortBay side.

## Concept

When you add a project manually (via the GUI wizard or `portbay_add_project`), you fill in a set of fields: framework type, PHP version if applicable, document root, HTTPS toggle. Recipes are pre-filled versions of those fields for common stacks. The agent handles natural-language understanding ("a Laravel app at ~/code/blog"); the recipe handles the exact configuration.

Recipes apply to **existing folders**. For scaffolding a brand-new project from scratch, see the [onboarding gallery](#scaffolding-a-new-project-from-scratch).

### What a recipe sets

| Field | What it controls |
| --- | --- |
| `project_type` | Framework / runtime (`php`, `node`, `next`, `vite`, `static`) |
| `php_version` | PHP version label passed to PHP-FPM (PHP recipes only) |
| `document_root` | Subdirectory Caddy serves from (e.g. `public`) |
| `https` | Whether mkcert issues a local TLS cert |
| `needs_database` | Recommended database engine; surfaces as a warning — not auto-provisioned yet |
| `needs_mail` | Whether the stack benefits from a local Mailpit instance — also a warning, not auto-wired |

The `composes_fully` flag in the API response is `true` when PortBay can apply every part of the recipe today (no pending database or mail). It is `false` for Laravel and Symfony, where the project is registered and usable but a database warning is included in the result.

## Where recipes are surfaced

Recipes are exposed only through the **MCP server** (`portbay-mcp`). There is no recipe picker in the GUI today. The GUI's first-run gallery (`/onboarding`) runs upstream scaffolders to create new projects from scratch; that is a different surface — see [below](#scaffolding-a-new-project-from-scratch).

The two MCP tools:

| Tool | Purpose |
| --- | --- |
| `portbay_list_recipes` | List the full catalog with all fields |
| `portbay_setup_from_recipe` | Apply a recipe to an existing folder |

The catalog is also readable as a resource: `portbay://recipes`.

## Quickstart

You have an existing Laravel project at `~/code/blog`. Ask your MCP-connected agent:

```
Set up my Laravel app at ~/code/blog.
```

The agent calls `portbay_list_recipes` to confirm the `laravel` recipe exists, then:

```json
{
  "tool": "portbay_setup_from_recipe",
  "arguments": {
    "recipe": "laravel",
    "path": "/Users/me/code/blog",
    "hostname": "blog.test"
  }
}
```

PortBay registers the project with PHP-FPM, Caddy, `public/` as the document root, and HTTPS. Because Laravel expects MySQL, the result includes a warning:

```json
{
  "ok": true,
  "detail": "Set up the `laravel` recipe — Registered blog at blog.test (HTTPS).",
  "warnings": [
    "the `laravel` recipe recommends a mysql:8.0 database; automatic database provisioning isn't available yet, so the project is registered without it — add one from the app's Databases panel when ready",
    "the `laravel` recipe benefits from a local mail catcher (Mailpit); enable it from the app when ready"
  ],
  "project": {
    "id": "blog",
    "url": "https://blog.test",
    "kind": "php",
    "https": true
  }
}
```

The project is registered and will start. The database and mail warnings are guidance, not failures.

## How to use from an AI agent (MCP)

The MCP server must be configured first. See [Drive PortBay from an AI Agent](/agents/) for setup instructions.

### 1. Discover the catalog

```
You: Which stack recipes does PortBay have?
```

The agent calls `portbay_list_recipes`. It returns every recipe with its id, title, description, and `composes_fully` flag. The agent can also read the catalog as a resource:

```
portbay://recipes
```

### 2. Apply a recipe

Once you know the recipe id, ask the agent to set up an existing folder:

```
You: Register my Symfony project at ~/code/api as symfony.test.
```

The agent calls `portbay_setup_from_recipe`:

```json
{
  "recipe": "symfony",
  "path": "/Users/me/code/api",
  "hostname": "api.test"
}
```

### Parameters for `portbay_setup_from_recipe`

| Parameter | Required | Default | Description |
| --- | --- | --- | --- |
| `recipe` | yes | — | Recipe id (e.g. `laravel`, `next`). Use `portbay_list_recipes` to get the catalog. |
| `path` | yes | — | Absolute path to an existing project folder. |
| `name` | no | Folder name | Display name in the app. |
| `hostname` | no | `<slug>.<domain-suffix>` | Local hostname without scheme (e.g. `blog.test`). |
| `php_version` | no | Recipe default | Overrides the recipe's PHP version. Only meaningful for PHP stacks. |
| `https` | no | Recipe default | Override the recipe's HTTPS setting. |
| `start_now` | no | `true` | Start the project after registering. |
| `auto_launch` | no | `false` | If the daemon is down and `start_now` is true, open the PortBay app first. Use only when you are at your machine. |

### Overriding recipe defaults

You can override `php_version` and `https` per call. Everything else (document root, project type) follows the recipe and cannot be overridden — use `portbay_add_project` directly if you need full control.

Example: apply the `laravel` recipe but pin PHP 8.2:

```json
{
  "recipe": "laravel",
  "path": "/Users/me/code/blog",
  "php_version": "8.2"
}
```

## Scaffolding a new project from scratch

If the project folder does not exist yet, use `portbay_setup_from_template` instead of `portbay_setup_from_recipe`. Templates run an upstream scaffolder (`pnpm create`, `composer create-project`) and then register the result.

Available templates: `nextjs`, `vite`, `astro`, `laravel`, `php`.

```json
{
  "tool": "portbay_setup_from_template",
  "arguments": {
    "template": "laravel",
    "parent_path": "/Users/me/code",
    "name": "blog"
  }
}
```

The GUI's `/onboarding` gallery covers the same five templates with a folder-picker UI and a live log scroller. Recipes and templates are complementary: templates scaffold, recipes configure.

## Recipe catalog reference

Verified against `src-tauri/src/mcp/recipes.rs`. 9 recipes.

| id | Title | Project type | PHP version | Document root | HTTPS | Database | Mail | Composes fully |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `next` | Next.js | `node` | — | — | yes | — | no | yes |
| `vite` | Vite | `vite` | — | — | yes | — | no | yes |
| `astro` | Astro | `node` | — | — | yes | — | no | yes |
| `node` | Node | `node` | — | — | yes | — | no | yes |
| `static` | Static site | `static` | — | — | yes | — | no | yes |
| `php` | PHP | `php` | 8.3 | — | yes | — | no | yes |
| `laravel` | Laravel | `php` | 8.3 | `public` | yes | mysql:8.0 | yes | **no** |
| `symfony` | Symfony | `php` | 8.3 | `public` | yes | mysql:8.0 | yes | **no** |
| `statamic` | Statamic | `php` | 8.3 | `public` | yes | — | no | yes |

### Notes

**next, vite, astro, node** — Node-based. PortBay detects the dev script from `package.json`; start command is not pinned by the recipe.

**static** — Caddy serves the folder directly over HTTPS. No dev server process.

**php** — Plain PHP. Caddy + PHP-FPM 8.3. No document root subdirectory (serves from the project root).

**laravel, symfony** — Caddy + PHP-FPM 8.3. Serves from `public/`. Both recommend MySQL 8.0 and Mailpit. The project registers and starts; the database and mail warnings are guidance to add those services from the app when ready.

**statamic** — Caddy + PHP-FPM 8.3. Serves from `public/`. Statamic supports flat-file mode; no database is required.

## Troubleshooting

| Symptom | Likely cause | Action |
| --- | --- | --- |
| `BAD_INPUT: unknown recipe` | Typo in the recipe id | Call `portbay_list_recipes` to get the exact id. |
| `BAD_INPUT: path is not a directory` | The folder does not exist | Create the folder first, or use `portbay_setup_from_template` to scaffold it. |
| `PROJECT_CAP_REACHED` | Project limit for the current tier | Sign in or upgrade to Pro for unlimited projects. |
| Project registered but won't start | Daemon not running | Open PortBay.app or pass `auto_launch: true` on the next call. |
| Laravel/Symfony returns a database warning | `composes_fully: false` — expected behaviour | Add the database from the app's Databases panel. |
