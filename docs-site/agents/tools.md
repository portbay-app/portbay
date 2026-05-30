---
title: PortBay MCP Tool Reference — All 69 Tools
description: "Complete reference for every tool and resource exposed by portbay-mcp: project registration, lifecycle controls, diagnostics, scaffolding, databases, groups, DNS, certificates, sandbox, runtimes, tunnels, HTTP inspector, and migration import."
---

# MCP Tool Reference

The full surface exposed by `portbay-mcp`. Every tool returns `structuredContent` (typed JSON Schema output) plus a plain-text mirror for clients that don't support structured content. Tools declare behavior annotations: `readOnlyHint` for read-only tools, `destructiveHint` for destructive ones, `idempotentHint` where applicable.

Tools are grouped into [toolsets](./index.md#governance) you can enable or disable with `--toolsets`.

**69 tools · 4 static resources · 5 resource templates**

Legend: read-only · mutates state · destructive (confirm first)

---

## Projects toolset

### `portbay_list_projects` (read-only)

List every registered project with hostname, URL, and — when the daemon is running — live status, PID, and restart count. `daemon_reachable: false` means only registry data is shown; `status` will be `unknown`. Start here before acting.

No arguments.

**Returns:** `ListProjectsResult`

| Field | Type | Notes |
| --- | --- | --- |
| `daemon_reachable` | bool | Whether the Process Compose daemon answered. |
| `projects` | `ProjectSummary[]` | See [ProjectSummary](#projectsummary). |

---

### `portbay_status` (read-only)

Live runtime detail for one project or all projects. Returns the same shape as `portbay_list_projects` but filtered to the requested project(s).

| Arg | Type | Notes |
| --- | --- | --- |
| `id` | string? | Project id (slug). Omit to get all projects. |

**Returns:** `ListProjectsResult`

---

### `portbay_detect_project` (read-only)

Inspect a folder and return the detected framework plus suggested registration defaults: id, hostname, port, start command. Nothing is registered — a non-committal preview to confirm with the user before calling `portbay_add_project`.

| Arg | Type | Notes |
| --- | --- | --- |
| `path` | string | Absolute path to the folder to inspect. |

**Returns:** `DetectResult`

| Field | Type | Notes |
| --- | --- | --- |
| `kind` | string | Detected framework (`next`, `vite`, `php`, `static`, `node`, …). |
| `suggested_id` | string | Slug derived from the folder name. |
| `suggested_name` | string | Human-readable display name. |
| `suggested_hostname` | string | `<slug>.<domain-suffix>`. |
| `suggested_port` | number? | Dev server port, when detected. |
| `suggested_start_command` | string? | Dev server command, when detected. |
| `suggested_document_root` | string? | PHP: relative document root, when detected. |
| `suggested_php_version` | string? | PHP: version label, when detected. |

---

### `portbay_detect_workspace_apps` (read-only)

List the runnable apps inside a JS monorepo so the agent can register just one as a standalone PortBay project instead of a root turbo fan-out. Returns `null` for a plain (non-monorepo) folder — use `portbay_detect_project` instead for those. Each app entry carries suggested id, hostname, port, and start command ready for `portbay_add_project`.

| Arg | Type | Notes |
| --- | --- | --- |
| `path` | string | Absolute path to the folder to inspect (typically the monorepo root). |

**Returns:** `WorkspaceScanResult` or `null`

| Field | Type | Notes |
| --- | --- | --- |
| `root` | string | Absolute path of the detected monorepo root. |
| `tool` | string | Package manager / build tool detected from the lockfile (`pnpm`, `npm`, `yarn`, `bun`). |
| `apps` | `WorkspaceAppSummary[]` | Runnable apps found in the monorepo (those declaring a `dev` script). |

`WorkspaceAppSummary` fields:

| Field | Type | Notes |
| --- | --- | --- |
| `package` | string | The package `name` from its `package.json` (may include a scope prefix such as `@acme/web`). |
| `rel_dir` | string | Directory path relative to the monorepo root (e.g. `apps/web`). |
| `path` | string | Absolute path to the package directory. |
| `kind` | string | Detected framework (`next`, `vite`, `node`, …). |
| `suggested_id` | string | Suggested PortBay project id (url-safe slug derived from the leaf dir). |
| `suggested_hostname` | string | Suggested hostname (e.g. `web.portbay.test`). |
| `suggested_port` | number? | Dev-server port detected from the framework, when applicable. |
| `suggested_start_command` | string? | Shell command that starts this app in isolation. |

---

### `portbay_list_recipes` (read-only)

List the available stack recipes — named blueprints (`laravel`, `next`, `vite`, …) that compose a project's framework, language version, document root, and HTTPS in one step. Map the user's request to a recipe id, then call `portbay_setup_from_recipe`. A recipe with `composes_fully: false` also recommends a database or mail service that isn't auto-provisioned yet (the project still registers, with a warning).

No arguments.

**Returns:** `ListRecipesResult` — an object with a `recipes` array of `RecipeSummary`:

| Field | Type | Notes |
| --- | --- | --- |
| `id` | string | Stable recipe id to pass to `portbay_setup_from_recipe`. |
| `title` | string | Human-readable name. |
| `description` | string | One-line summary. |
| `project_type` | string | Framework the recipe registers. |
| `php_version` | string? | Default PHP version (PHP recipes only). |
| `document_root` | string? | Relative document root (e.g. `public`). |
| `https` | bool | Whether HTTPS is on by default. |
| `database` | string? | Recommended database as `engine:version` (e.g. `mysql:8.0`). |
| `mail` | bool | Whether the stack recommends a local mail catcher. |
| `composes_fully` | bool | `false` when a database or mail service is needed but not auto-provisioned. |

Current catalog: `next`, `vite`, `astro`, `node`, `static`, `php`, `laravel`, `symfony`, `statamic`.

---

### `portbay_add_project` (mutates state)

Register an existing local folder as a PortBay project. It gets a local hostname, optional HTTPS via mkcert, and managed start/stop. Omit `kind` to auto-detect the framework from the folder contents. Does **not** start the project — call `portbay_start` after, or use `portbay_setup` to register and start in one call.

| Arg | Type | Notes |
| --- | --- | --- |
| `path` | string | **Required.** Absolute path to the existing folder. |
| `name` | string? | Display name. Defaults to the folder name. |
| `hostname` | string? | Hostname without scheme. Defaults to `<slug>.<domain-suffix>`. |
| `kind` | string? | Framework (`next`, `vite`, `php`, `static`, `node`, `flutter`, `xcode`, `android`, `custom`). Omit to auto-detect. |
| `port` | number? | Dev server port. Omit for static / PHP-only projects. |
| `start_command` | string? | Shell command to start the dev server. Omit for Caddy-only projects. |
| `https` | bool? | Enable local HTTPS via mkcert. Default `true`. |
| `auto_start` | bool? | Start on daemon boot. Default `false`. |
| `php_version` | string? | PHP version label (e.g. `8.3`). PHP projects only. |
| `document_root` | string? | Relative document root (e.g. `public`). PHP projects only. |

**Returns:** `OpResult`

---

### `portbay_update_project` (mutates state · idempotent)

Patch fields on an existing project. Only the fields you provide are changed.

| Arg | Type | Notes |
| --- | --- | --- |
| `id` | string | **Required.** Project id (slug). |
| `name` | string? | New display name. |
| `hostname` | string? | New hostname. Changing this re-issues the cert on the next reconcile. |
| `port` | number? | New dev server port. |
| `start_command` | string? | New start command. |
| `https` | bool? | Enable or disable HTTPS. |
| `auto_start` | bool? | Enable or disable auto-start on daemon boot. |
| `tags` | string[]? | Replace the project's tag list entirely. |

**Returns:** `OpResult`

---

### `portbay_remove_project` (mutates state · destructive)

Unregister a project and clean up its cert and `/etc/hosts` entry. Source files on disk are **not** touched. Confirm with the user before calling — this is irreversible from PortBay's side.

| Arg | Type | Notes |
| --- | --- | --- |
| `id` | string | **Required.** Project id (slug). |

**Returns:** `OpResult`

---

### `portbay_export_config` (mutates state · idempotent)

Write a `.portbay.json` into the project folder so the setup can be committed and reproduced by teammates. Secret values are never written — only their names.

| Arg | Type | Notes |
| --- | --- | --- |
| `id` | string | **Required.** Project id (slug). |

**Returns:** `ExportResult`

| Field | Type | Notes |
| --- | --- | --- |
| `wrote` | string | Absolute path of the written `.portbay.json`. |
| `env_count` | number | Number of env vars written to the template. |
| `secret_names` | string[] | Names of secret vars (values not written). |

---

### `portbay_import_config` (mutates state)

Register a project from a committed `.portbay.json`. Pass the project folder path or the file path directly.

| Arg | Type | Notes |
| --- | --- | --- |
| `path` | string | **Required.** Absolute path to the folder containing `.portbay.json`, or to the file itself. |
| `secrets` | object? | Key → value map for declared secret env vars. Omitted secrets are registered as empty placeholders (a warning is returned listing them). |

**Returns:** `OpResult`

---

### `portbay_setup` (mutates state)

The one-call "set this up for me" flow: register an existing folder (auto-detecting the framework) and immediately start it, returning the live URL. Set `start_now: false` to register without starting.

| Arg | Type | Notes |
| --- | --- | --- |
| `path` | string | **Required.** Absolute path to the existing folder. |
| `name` | string? | Display name. |
| `hostname` | string? | Hostname without scheme. |
| `kind` | string? | Framework. Omit to auto-detect. |
| `port` | number? | Dev server port. |
| `start_command` | string? | Dev server start command. |
| `https` | bool? | Enable HTTPS. Default `true`. |
| `start_now` | bool? | Start after registering. Default `true`. |
| `auto_launch` | bool? | If the daemon is down and `start_now` is `true`, open the PortBay app first. Default `false`. |

**Returns:** `OpResult`

---

### `portbay_setup_from_recipe` (mutates state)

Apply a named stack recipe to an existing folder: register it with the recipe's framework, language version, document root, and HTTPS, then start it. The fastest path when the user names a stack. Call `portbay_list_recipes` first to discover available recipe ids.

For a brand-new project that doesn't exist on disk yet, use `portbay_setup_from_template` instead.

| Arg | Type | Notes |
| --- | --- | --- |
| `recipe` | string | **Required.** Recipe id, e.g. `laravel`, `next`, `vite`. |
| `path` | string | **Required.** Absolute path to the existing project folder. |
| `name` | string? | Display name. |
| `hostname` | string? | Hostname without scheme. |
| `php_version` | string? | Override the recipe's default PHP version. |
| `https` | bool? | Override the recipe's HTTPS default. |
| `start_now` | bool? | Start after registering. Default `true`. |
| `auto_launch` | bool? | If the daemon is down and `start_now` is `true`, open the app first. Default `false`. |

**Returns:** `OpResult`. If the recipe recommends a database or mail catcher that PortBay can't provision yet, the project is still registered and `warnings` describes what to add manually.

---

## Lifecycle toolset

Lifecycle tools require the PortBay daemon to be running. Without it, they return `SIDECAR_DOWN`. Pass `auto_launch: true` on `portbay_start` only when the user is at their machine and expects the app to open.

### `portbay_start` (mutates state · idempotent)

Start a registered project.

| Arg | Type | Notes |
| --- | --- | --- |
| `id` | string | **Required.** Project id (slug). |
| `auto_launch` | bool? | Open the PortBay app if the daemon is down, wait up to ~15 s, then start. Default `false`. |

**Returns:** `OpResult`

---

### `portbay_stop` (mutates state · idempotent)

Stop a running project.

| Arg | Type | Notes |
| --- | --- | --- |
| `id` | string | **Required.** Project id (slug). |

**Returns:** `OpResult`

---

### `portbay_restart` (mutates state · idempotent)

Restart a project (stop then start).

| Arg | Type | Notes |
| --- | --- | --- |
| `id` | string | **Required.** Project id (slug). |

**Returns:** `OpResult`

---

### `portbay_stop_all` (mutates state · idempotent)

Stop every running PortBay process. No arguments.

**Returns:** `OpResult`

---

## Diagnostics toolset

### `portbay_logs` (read-only)

Return recent log output for a project. The first thing to read when a project won't start or is crash-looping. Requires the daemon.

| Arg | Type | Notes |
| --- | --- | --- |
| `id` | string | **Required.** Project id (slug). |
| `lines` | number? | Trailing lines to return. Default `200`. |
| `offset` | number? | Offset into the log buffer (0 = newest). Default `0`. |

**Returns:** `LogsResult`

| Field | Type | Notes |
| --- | --- | --- |
| `id` | string | The project id. |
| `lines` | string[] | Log lines, newest last. |

---

### `portbay_doctor` (read-only)

Run a grouped environment health check — the **same data** the CLI `portbay doctor` renders (both call one shared core, so they can't drift). Categories: **Core** (registry, daemon, `/etc/hosts`), **Web routing & TLS** (Caddy, mkcert, certs), **PHP runtimes**, **Services** (dnsmasq, Mailpit, databases), **Account & sharing**. Bundled sidecars (Caddy, mkcert, dnsmasq, Mailpit, cloudflared) are reported via PortBay's own probe and **never resolved from `$PATH`** — a foreign install is never mistaken for PortBay's own. Use when something is broken and you don't yet know what.

No arguments.

**Returns:** `DoctorReport`

| Field | Type | Notes |
| --- | --- | --- |
| `ok` | bool | `true` when no check returned `fail`. |
| `categories` | `DoctorCategory[]` | Each has `title` (string), `verdict` (`ok` / `warn` / `fail` — worst of its checks), and `checks`. |

Each entry in `checks` has `check` (string), `verdict` (`ok` / `warn` / `fail`), and `detail` (string).

---

### `portbay_sidecar_status` (read-only)

Report the state of PortBay's background services. Process Compose is probed directly over HTTP. Caddy, mkcert, dnsmasq, and Mailpit are managed by the daemon and reported as install-presence only (state `unknown` from outside the daemon). Use `portbay_doctor` for a fuller picture.

No arguments.

**Returns:** `SidecarStatusResult`

| Field | Type | Notes |
| --- | --- | --- |
| `daemon_reachable` | bool | Whether Process Compose answered. |
| `sidecars` | `SidecarReport[]` | Each report has `name`, `state` (`running` / `stopped` / `unknown`), and `detail`. |

---

## Scaffold toolset

### `portbay_setup_from_template` (mutates state)

Scaffold a brand-new project from a starter template into `parent_path/name`, then register it with PortBay. Runs the upstream scaffolder (`pnpm create` for JS frameworks, `composer create-project` for Laravel). This takes time and requires network access. `open_world_hint` is set on this tool because the scaffolder may reach the internet.

For a folder that already exists, use `portbay_add_project` or `portbay_setup_from_recipe` instead.

| Arg | Type | Notes |
| --- | --- | --- |
| `template` | string | **Required.** One of: `nextjs`, `vite`, `astro`, `laravel`, `php`. |
| `parent_path` | string | **Required.** Absolute path to the directory the new folder is created inside. |
| `name` | string | **Required.** Name of the new folder to create under `parent_path`. |
| `start_now` | bool? | Start after registering. Default `false` (scaffolding can be slow; the agent usually reports the URL and lets the user start manually). |

**Returns:** `OpResult`

---

## Groups toolset

### `portbay_list_groups` (read-only)

List every project group registered with PortBay. Each group carries its member project ids, a `known_ids` subset (members that still exist in the registry), and a `member_count`. Use this to discover group ids before calling start/stop/restart/update/remove group tools.

No arguments.

**Returns:** `GroupSummary[]` — see [GroupSummary](#groupsummary).

---

### `portbay_create_group` (mutates state)

Create a named group of projects. Groups let you start, stop, or restart multiple projects in one call. The id is derived from `name` automatically, or pass an explicit `id`. Unknown project ids are tracked and surfaced via `known_ids` on list.

| Arg | Type | Notes |
| --- | --- | --- |
| `name` | string | **Required.** Human-readable display name (e.g. `"Backend services"`). |
| `id` | string? | Explicit group id (url-safe slug). Derived from `name` when omitted. |
| `project_ids` | string[] | Project ids (slugs) to include. May be empty — members can be added later via `portbay_update_group`. |

**Returns:** `GroupSummary`

---

### `portbay_update_group` (mutates state · idempotent)

Rename a group or replace its member list. Only the fields you set are changed. `project_ids` fully replaces the member list (not a merge).

| Arg | Type | Notes |
| --- | --- | --- |
| `id` | string | **Required.** Group id (slug). |
| `name` | string? | New display name. Leave unset to keep the current name. |
| `project_ids` | string[]? | Full replacement member list. Leave unset to keep the current members. |

**Returns:** `GroupSummary`

---

### `portbay_remove_group` (mutates state · destructive)

Delete a group. Member projects are **not** affected — only the group record is removed. Confirm with the user first.

| Arg | Type | Notes |
| --- | --- | --- |
| `id` | string | **Required.** Group id (slug). |

**Returns:** `{}` (empty object on success)

---

### `portbay_start_group` (mutates state · idempotent)

Start every project in a group. Members without a managed process (e.g. mobile/Xcode projects) are counted as succeeded and skipped. Stale members (removed from the registry but still in the group) are counted as failed. Requires the daemon.

| Arg | Type | Notes |
| --- | --- | --- |
| `id` | string | **Required.** Group id (slug). |

**Returns:** `GroupFanoutResult` — see [GroupFanoutResult](#groupfanoutresult).

---

### `portbay_stop_group` (mutates state · idempotent)

Stop every project in a group. Members without a managed process are counted as succeeded and skipped. Requires the daemon.

| Arg | Type | Notes |
| --- | --- | --- |
| `id` | string | **Required.** Group id (slug). |

**Returns:** `GroupFanoutResult`

---

### `portbay_restart_group` (mutates state · idempotent)

Restart every project in a group (stop then start). Members without a managed process are counted as succeeded and skipped. Requires the daemon.

| Arg | Type | Notes |
| --- | --- | --- |
| `id` | string | **Required.** Group id (slug). |

**Returns:** `GroupFanoutResult`

---

## Tunnels toolset

These tools are read-only. Starting and stopping a Cloudflare tunnel share is done from the PortBay app.

### `portbay_list_tunnels` (read-only)

List active public tunnels. Each entry includes the project id, upstream URL, public share URL (or `null` while Cloudflare is still assigning one), running state, and origin reachability.

No arguments.

**Returns:** `TunnelStatus[]` — see [TunnelStatus](#tunnelstatus).

---

### `portbay_tunnel_status` (read-only)

Get the tunnel details for one project by id. Returns `null` when no tunnel exists for the given project.

| Arg | Type | Notes |
| --- | --- | --- |
| `id` | string | **Required.** Project id (slug) whose tunnel to look up. |

**Returns:** `TunnelStatus` or `null`

---

## Runtimes toolset

### `portbay_list_runtimes` (read-only)

List every language PortBay knows about (PHP, Node.js, Python, Go, Ruby, Bun, Flutter) with all detected installs, their source (Homebrew, asdf, mise, nvm, system, manual), and the configured default version. No daemon required. Installing a new language version and editing PHP FPM/ini config are done from the PortBay app.

No arguments.

**Returns:** `RuntimeLanguageSummary[]` — see [RuntimeLanguageSummary](#runtimelanguagesummary).

---

### `portbay_set_default_runtime` (mutates state · idempotent)

Set or clear the default version for a language. The default is inherited by new projects when no version-manager file (`.nvmrc`, `.tool-versions`, etc.) is detected. Omit `version` or pass `null` to clear the current default. The version must already be detected — call `portbay_list_runtimes` first.

| Arg | Type | Notes |
| --- | --- | --- |
| `lang` | string | **Required.** Language id: `php`, `node`, `python`, `bun`, `go`, `ruby`, `flutter`. |
| `version` | string? | Version label to set as default (e.g. `"8.3"`, `"20"`). Omit or `null` to clear. |

**Returns:** `RuntimeLanguageSummary[]` — updated list for the language.

---

### `portbay_add_runtime_path` (mutates state · idempotent)

Register an existing binary as a manual runtime install for a language. PortBay probes the binary for its version string — if it doesn't report one, the call is rejected. Deduplicates by canonical path.

| Arg | Type | Notes |
| --- | --- | --- |
| `lang` | string | **Required.** Language id (e.g. `php`, `node`). |
| `path` | string | **Required.** Absolute path to the runtime binary (e.g. `/usr/local/bin/php`). |

**Returns:** `RuntimeLanguageSummary[]` — updated list.

---

### `portbay_remove_runtime_path` (mutates state · idempotent)

Remove a manually-added runtime install by language id and version label. No-op when the version is not present or was not manually added.

| Arg | Type | Notes |
| --- | --- | --- |
| `lang` | string | **Required.** Language id (e.g. `php`, `node`). |
| `version` | string | **Required.** Version label as returned by `portbay_list_runtimes` (e.g. `"8.3"`). |

**Returns:** `RuntimeLanguageSummary[]` — updated list.

---

## Databases toolset

### `portbay_list_database_engines` (read-only)

List every database engine PortBay can manage (MySQL, PostgreSQL, MariaDB, Redis, MongoDB, Memcached), each with install state, detected version, default port, CLI-client availability, and a Homebrew install hint. Check here before `portbay_create_database` — installing an engine binary is done from the PortBay app.

No arguments.

**Returns:** `DatabaseEngineSummary[]`

| Field | Type | Notes |
| --- | --- | --- |
| `id` | string | Stable engine id: `mysql`, `postgres`, `mariadb`, `redis`, `mongo`, `memcached`. |
| `label` | string | Human-readable name. |
| `installed` | bool | Whether the daemon binary resolves on this machine. |
| `version` | string | Detected daemon version; empty when not installed. |
| `default_port` | number | Engine's default port. |
| `client_available` | bool | Whether the CLI client (`psql`, `mysql`, …) is available. |
| `install_hint` | string | Homebrew command to install the engine. |

---

### `portbay_list_databases` (read-only)

List the database instances PortBay manages, each with engine, port, connection URL, linked projects, and — when the daemon is running — live status. `daemon_reachable: false` means status reflects the registry only.

No arguments.

**Returns:** `ListDatabasesResult`

| Field | Type | Notes |
| --- | --- | --- |
| `daemon_reachable` | bool | Whether the daemon answered. |
| `instances` | `DatabaseInstanceSummary[]` | See [DatabaseInstanceSummary](#databaseinstancesummary). |

---

### `portbay_database_connection` (read-only)

Get connection details for one database instance: the connection URL plus the framework env vars (DATABASE_URL, DB_CONNECTION, DB_HOST, DB_PORT, …) PortBay injects into linked projects.

| Arg | Type | Notes |
| --- | --- | --- |
| `id` | string | **Required.** Database instance id (slug). |

**Returns:** `DatabaseConnectionResult`

| Field | Type | Notes |
| --- | --- | --- |
| `id` | string | Instance id. |
| `engine` | string | Engine id. |
| `connection_url` | string | Connection URL (e.g. `mysql://root@127.0.0.1:3306/`). |
| `account` | string | Default provisioned account (`root`, `postgres`, …). |
| `env` | object | Key → value map of env vars PortBay injects into linked projects. |

---

### `portbay_create_database` (mutates state)

Provision and register a new database instance: PortBay initializes an isolated data directory, writes a config, and tracks the instance. The engine binary must already be installed (check with `portbay_list_database_engines`). The instance joins Process Compose after the app's next reconcile (≤30s); start it with `portbay_start_database`.

| Arg | Type | Notes |
| --- | --- | --- |
| `engine` | string | **Required.** Engine id: `mysql`, `postgres`, `mariadb`, `redis`, `mongo`, or `memcached`. |
| `name` | string | **Required.** Human-readable name. The instance id is slugified from this. |
| `port` | number? | Port to bind. Omit to auto-allocate from the engine's default upward. |
| `auto_start` | bool? | Start on daemon boot. Default `false`. |

**Returns:** `DatabaseOpResult`

---

### `portbay_remove_database` (mutates state · destructive)

Stop (best-effort) and unregister a database instance. By default the on-disk data is kept; pass `delete_data: true` to also delete the data directory (irreversible). Confirm with the user before deleting data.

| Arg | Type | Notes |
| --- | --- | --- |
| `id` | string | **Required.** Database instance id (slug). |
| `delete_data` | bool? | Also delete the on-disk data directory. Default `false`. |

**Returns:** `DatabaseOpResult`

---

### `portbay_start_database` (mutates state · idempotent)

Start a database instance's daemon via Process Compose. Requires the PortBay daemon and the instance to already be in its config (true once the app has reconciled a newly-created instance).

| Arg | Type | Notes |
| --- | --- | --- |
| `id` | string | **Required.** Database instance id (slug). |

**Returns:** `DatabaseOpResult`

---

### `portbay_stop_database` (mutates state · idempotent)

Stop a running database instance. Requires the PortBay daemon.

| Arg | Type | Notes |
| --- | --- | --- |
| `id` | string | **Required.** Database instance id (slug). |

**Returns:** `DatabaseOpResult`

---

### `portbay_restart_database` (mutates state · idempotent)

Restart a database instance (stop then start). Requires the daemon.

| Arg | Type | Notes |
| --- | --- | --- |
| `id` | string | **Required.** Database instance id (slug). |

**Returns:** `DatabaseOpResult`

---

### `portbay_link_database` (mutates state · idempotent)

Link a database instance to a project. PortBay injects the instance's connection env vars (DATABASE_URL, DB_*) into the linked project's process on the next reconcile, so the app can reach the database with zero manual config.

| Arg | Type | Notes |
| --- | --- | --- |
| `id` | string | **Required.** Database instance id (slug). |
| `project_id` | string | **Required.** Project id (slug) to link. |

**Returns:** `DatabaseOpResult`

---

### `portbay_unlink_database` (mutates state · idempotent)

Unlink a database instance from a project, stopping its connection env vars from being injected into that project.

| Arg | Type | Notes |
| --- | --- | --- |
| `id` | string | **Required.** Database instance id (slug). |
| `project_id` | string | **Required.** Project id (slug) to unlink. |

**Returns:** `DatabaseOpResult`

---

### `portbay_set_database_auto_start` (mutates state · idempotent)

Set whether a database instance starts automatically when the PortBay daemon boots.

| Arg | Type | Notes |
| --- | --- | --- |
| `id` | string | **Required.** Database instance id (slug). |
| `auto_start` | bool | **Required.** Whether the instance should auto-start. |

**Returns:** `DatabaseOpResult`

---

## DNS toolset

### `portbay_dns_status` (read-only)

Report local DNS state: the active domain suffix, whether `/etc/resolver/<suffix>` routes wildcard `*.suffix` to PortBay's dnsmasq (and on which port), whether the privileged helper is installed, and the persisted dnsmasq tuning. Starting/restarting dnsmasq and first-run resolver install are done from the PortBay app.

No arguments.

**Returns:** `DnsStatusResult`

| Field | Type | Notes |
| --- | --- | --- |
| `suffix` | string | The active domain suffix (e.g. `portbay.test`). |
| `resolver_installed` | bool | Whether `/etc/resolver/<suffix>` points wildcard `*.suffix` at PortBay's dnsmasq. |
| `resolver_path` | string | Path of the resolver file. |
| `resolver_port` | number? | Port the resolver file targets (parsed from the file). |
| `resolver_contents` | string? | Raw resolver-file contents for diagnostics; `null` when not installed. |
| `helper_available` | bool | Whether PortBay's privileged hosts/resolver helper is installed. |
| `dnsmasq` | object | Persisted dnsmasq settings: `cache_size`, `local_ttl`, `disable_negative_cache`. |

---

### `portbay_list_dns_records` (read-only)

List the names PortBay resolves: the wildcard `*.<suffix>` plus one row per project hostname, each tagged with how it's currently routed (`dnsmasq` via the resolver file, or `hosts` via `/etc/hosts`).

No arguments.

**Returns:** `DnsRecordSummary[]`

| Field | Type | Notes |
| --- | --- | --- |
| `hostname` | string | The resolvable name. |
| `target` | string | Always loopback (`127.0.0.1`) for PortBay-managed names. |
| `kind` | string | `wildcard` or `project`. |
| `project_id` | string? | Associated project id, when `kind` is `project`. |
| `project_name` | string? | Associated project name, when `kind` is `project`. |
| `routed_via` | string | `dnsmasq` when the resolver file routes this name; otherwise `hosts`. |

---

### `portbay_set_domain_suffix` (mutates state · destructive)

Change the local domain suffix (e.g. `test` → `localhost`). Rewrites **every** project hostname to the new suffix and drops their HTTPS cert directories (the app reissues certs and updates `/etc/hosts` on the next reconcile). Reserved public TLDs (`.com`, etc.) are rejected. High blast radius — confirm with the user first.

| Arg | Type | Notes |
| --- | --- | --- |
| `suffix` | string | **Required.** New suffix (e.g. `test`, `localhost`, `portbay.test`). Reserved public TLDs are rejected. |

**Returns:** `SetDomainSuffixResult`

| Field | Type | Notes |
| --- | --- | --- |
| `old_suffix` | string | The previous suffix. |
| `new_suffix` | string | The new suffix now in effect. |
| `changed_projects` | number | Number of project hostnames rewritten. |
| `cert_dirs_removed` | number | Number of HTTPS cert directories removed (reissued by the app on reconcile). |

---

## Certs toolset

### `portbay_cert_info` (read-only)

Report local-HTTPS certificate metadata — file paths, issued/expiry dates, days until expiry, and DNS SANs — for one project (set `id`) or every project that has a cert (omit `id`). Reads cert files directly; no daemon required. A project with no cert yet is absent from the result.

| Arg | Type | Notes |
| --- | --- | --- |
| `id` | string? | Project id to report on. Omit for all projects with a cert. |

**Returns:** `CertInfo[]`

| Field | Type | Notes |
| --- | --- | --- |
| `projectId` | string | The associated project id. |
| `certificatePath` | string | Absolute path to the certificate file. |
| `keyPath` | string | Absolute path to the private key file. |
| `issuedAt` | string? | ISO-8601 timestamp when the cert was issued. |
| `expiresAt` | string? | ISO-8601 expiry timestamp. |
| `daysUntilExpiry` | number? | Days remaining; negative when already expired. |
| `sans` | string[] | DNS Subject Alternative Names on the cert. |

---

### `portbay_reissue_cert` (mutates state · idempotent)

Reissue a project's local-HTTPS certificate: deletes the current cert so the running PortBay app mints a fresh one and reloads Caddy on its next reconcile (≤30s). The mkcert CA must already be trusted — installing it into the system keychain is privileged and interactive, done from the PortBay app.

| Arg | Type | Notes |
| --- | --- | --- |
| `id` | string | **Required.** Project id (slug). |

**Returns:** `OpResult`

---

## Sandbox toolset

### `portbay_sandbox_status` (read-only)

Report Sandboxed Run state: per-project policy (enabled, network, ephemeral), whether this OS supports it (macOS only), whether `sandbox-exec` is present, the tier's sandbox cap, and how many projects are sandboxed. Set `id` for one project, omit for all.

| Arg | Type | Notes |
| --- | --- | --- |
| `id` | string? | Project id. Omit to list every project's sandbox state. |

**Returns:** `SandboxStatusResult`

| Field | Type | Notes |
| --- | --- | --- |
| `platform_supported` | bool | Whether this OS supports Sandboxed Run (macOS Seatbelt only). |
| `sandbox_available` | bool | Whether `sandbox-exec` is present; when false on macOS, enabling fails closed. |
| `community_cap` | number? | Max concurrent sandboxed projects on the current tier; `null` means unlimited (Pro). |
| `enabled_count` | number | How many projects currently have Sandboxed Run enabled. |
| `projects` | `SandboxProjectStatus[]` | Per-project policy. Each has `id`, `name`, `enabled`, `network`, and `ephemeral`. |

---

### `portbay_sandbox_violations` (read-only)

List recent sandbox-denial lines from a project's logs (`deny(...)` / "operation not permitted"), so you can see what the Seatbelt profile blocked. Requires the daemon.

| Arg | Type | Notes |
| --- | --- | --- |
| `id` | string | **Required.** Project id (slug) whose logs to scan. |
| `limit` | number? | How many recent log lines to scan. Default `250`. |

**Returns:** `SandboxViolationsResult`

| Field | Type | Notes |
| --- | --- | --- |
| `id` | string | The project id. |
| `scanned_lines` | number | How many log lines were scanned. |
| `violations` | string[] | The sandbox-denial lines found, in log order. |

---

### `portbay_enable_sandbox` (mutates state · idempotent)

Enable Sandboxed Run on a project (macOS only). Wraps the launch command in a Seatbelt profile that denies credential stores, browser data, and every `.env` outside the project. Fails closed if macOS rejects the profile. The instance is **not** started/restarted here — the app re-wraps the command on its next reconcile (≤30s), then call `portbay_restart` to run it confined. Community tiers cap concurrent sandboxed projects (check `portbay_sandbox_status`); Pro is unlimited.

| Arg | Type | Notes |
| --- | --- | --- |
| `id` | string | **Required.** Project id (slug) to sandbox. |
| `network` | string? | Network access inside the sandbox: `loopback_only` (default), `outbound`, `full`, or `blocked`. |
| `ephemeral` | bool? | Wipe the per-run cache/temp scratch dir before each sandboxed start. Default `true`. |

**Returns:** `SandboxOpResult`

| Field | Type | Notes |
| --- | --- | --- |
| `ok` | bool | Success flag. |
| `detail` | string | Human-readable summary. |
| `project` | `SandboxProjectStatus` | Updated sandbox state for the project. |

---

### `portbay_disable_sandbox` (mutates state · idempotent)

Disable Sandboxed Run on a project. The change applies on the next restart. Works on any OS so a synced sandbox flag can always be cleared.

| Arg | Type | Notes |
| --- | --- | --- |
| `id` | string | **Required.** Project id (slug). |

**Returns:** `SandboxOpResult`

---

## Inspector toolset

### `portbay_recent_requests` (read-only)

List recent HTTP requests Caddy handled (method, host, URI, status, duration, size, matched project), oldest→newest. Reads Caddy's access log off disk — works without the daemon; empty until the app has served traffic. Pass `project` to filter to one project's requests; `limit` to bound the count (default 200, max 2000).

| Arg | Type | Notes |
| --- | --- | --- |
| `limit` | number? | How many recent requests to return. Default `200`, max `2000`. |
| `project` | string? | Project id (slug) to filter to. Omit for all projects' traffic. |

**Returns:** `RequestEntry[]`

| Field | Type | Notes |
| --- | --- | --- |
| `ts` | number | Unix milliseconds when Caddy handled the request. |
| `method` | string | HTTP method (`GET`, `POST`, …). |
| `host` | string | Request host header. |
| `uri` | string | Request URI. |
| `status` | number | HTTP response status code. |
| `durationMs` | number | Response time in milliseconds. |
| `size` | number | Response size in bytes. |
| `projectId` | string? | The PortBay project this host maps to, when known. |
| `reqHeaders` | object? | Request headers Caddy logged (for detail views). |

---

### `portbay_clear_requests` (mutates state · idempotent)

Truncate Caddy's access log so the request inspector starts fresh. Safe while the app is running — the live stream resumes from the next request.

No arguments.

**Returns:** `OpResult`

---

## Migrate toolset

### `portbay_detect_import_sources` (read-only)

List which local-dev migration sources (Laravel Herd, ServBay, MAMP) are installed on this machine and how many sites each exposes. Use this first, then `portbay_preview_import` to inspect a source's sites.

No arguments.

**Returns:** `DetectedSource[]`

| Field | Type | Notes |
| --- | --- | --- |
| `source` | string | Source id: `herd`, `servbay`, or `mamp`. |
| `label` | string | Human-readable name (e.g. `"Laravel Herd"`). |
| `present` | bool | Whether the source tool's config or vhost directory is present. |
| `siteCount` | number | Number of sites that parsed without error. |
| `note` | string? | Free-form note (e.g. `"uses NGINX vhost format"`). |

---

### `portbay_preview_import` (read-only)

Preview the sites a migration source exposes, each flagged for whether its id or path already collides with an existing PortBay project. Read-only — confirm with the user before calling `portbay_import_projects`.

| Arg | Type | Notes |
| --- | --- | --- |
| `source` | string | **Required.** The source tool to scan: `herd`, `servbay`, or `mamp`. |

**Returns:** `ImportPreviewRow[]`

| Field | Type | Notes |
| --- | --- | --- |
| `site` | object | The parsed site (`path`, `hostname`, `phpVersion`, `https`, `documentRoot`, `suggestedId`, `suggestedName`). |
| `idCollision` | bool | `true` if a project with the same id already exists in PortBay. |
| `pathCollision` | bool | `true` if a project at the same path already exists. |

---

### `portbay_import_projects` (mutates state)

Import sites from a migration source into the PortBay registry. Pass the `ids` to import (from `portbay_preview_import`), or set `all: true` to import every site. Returns which ids landed and which were skipped (with a reason). The running PortBay app provisions the new projects — certs, Caddy routes, `/etc/hosts` — on its next reconcile (≤30s).

| Arg | Type | Notes |
| --- | --- | --- |
| `source` | string | **Required.** The source tool to import from: `herd`, `servbay`, or `mamp`. |
| `ids` | string[]? | Suggested ids (from `portbay_preview_import`) to import. |
| `all` | bool? | Import every site the source exposes, ignoring `ids`. Default `false`. |

**Returns:** `ImportResult`

| Field | Type | Notes |
| --- | --- | --- |
| `imported` | string[] | Ids of projects successfully imported. |
| `skipped` | object[] | Rows that were skipped; each has `site` (the parsed site) and `reason` (string). |

---

## Tasks toolset

The task board is a per-project Kanban whose cards are Markdown files in the project's repo (`.portbay/tasks/`). The same board is shown in the GUI, edited by the `portbay tasks` CLI, and exposed here — one source of truth, not three. These tools appear when a project has a board.

A dispatched agent's loop is: read `portbay_handoff_get` and `portbay_task_next` to pick up where work left off, `portbay_task_ack` the run, do the work while posting progress with `portbay_task_update` / `portbay_task_check`, then `portbay_handoff_update` and move the card to `Done` (or `Review`) before finishing. See the [Task Board guide](/guides/task-board) for the human side.

Statuses are `Backlog`, `Todo`, `InProgress`, `Blocked`, `Review`, `Done`, `Rejected`. An agent may move a card to any status **except `Rejected`**, which is human-only.

### `portbay_tasks_list` (read-only)

List a project's board cards — the live board the human sees. Optionally filter by column. Read this to see the plan before acting; the board, not your memory, is the source of truth.

| Arg | Type | Notes |
| --- | --- | --- |
| `project` | string | **Required.** Project id (slug). |
| `status` | string? | Filter to one column (e.g. `Todo`, `InProgress`). Omit for the whole board. |

**Returns:** `TaskCard[]`

---

### `portbay_task_next` (read-only)

Return the next actionable card — the top of the `Todo` column — or `null` when nothing is ready to work.

| Arg | Type | Notes |
| --- | --- | --- |
| `project` | string | **Required.** Project id (slug). |

**Returns:** `TaskCard | null`

---

### `portbay_task_get` (read-only)

Read one card in full by id — title, description (body), acceptance criteria, touchpoints, checklist, labels, status, and claim. Use this to re-read the card you were dispatched to work.

| Arg | Type | Notes |
| --- | --- | --- |
| `project` | string | **Required.** Project id (slug). |
| `id` | string | **Required.** Card id. |

**Returns:** `TaskCard`

---

### `portbay_task_create` (mutates state)

Capture a new card. Use it for work you discover mid-task rather than burying it in chat — it lands in `Backlog` by default.

| Arg | Type | Notes |
| --- | --- | --- |
| `project` | string | **Required.** Project id (slug). |
| `title` | string | **Required.** Card title. |
| `body` | string? | Markdown description. |
| `status` | string? | Starting column. Default `Backlog`. |
| `priority` | string? | `critical` / `high` / `medium` / `low`. |
| `acceptance` | string? | Acceptance criteria. |
| `touchpoints` | string[]? | Files/modules the work is expected to touch. |
| `labels` | string[]? | Label ids (colours come from the board config). |
| `estimate` | number? | Display-only estimate. |
| `template` | string? | Built-in card template id (e.g. `feature`, `bug`, `tests`, `refactor`). |

**Returns:** `TaskCard`

---

### `portbay_task_ack` (mutates state)

Acknowledge a dispatched card with the `run_id` from your prompt — proof you engaged with it (distinct from the process merely launching). Refreshes the run's lease.

| Arg | Type | Notes |
| --- | --- | --- |
| `project` | string | **Required.** Project id (slug). |
| `id` | string | **Required.** Card id. |
| `run_id` | string | **Required.** The run id PortBay passed in your dispatch prompt. |

**Returns:** `TaskCard`

---

### `portbay_task_update` (mutates state)

Advance a card and/or post a progress note. Set `status` to `InProgress` / `Done` / `Blocked` / `Review` / `Todo`, and record what you changed with `touchpoints`. Pass your `run_id` so a stale session can't clobber a re-dispatched card. You may **not** set `Rejected`. The response carries reminders (e.g. update the hand-off before finishing).

| Arg | Type | Notes |
| --- | --- | --- |
| `project` | string | **Required.** Project id (slug). |
| `id` | string | **Required.** Card id. |
| `run_id` | string? | Your dispatch run id; guards against stale-session writes. |
| `status` | string? | New column. Any status except `Rejected`. |
| `note` | string? | Progress note added to the card's activity. |
| `reason` | string? | Why — e.g. what blocked the card when moving to `Blocked`. |
| `touchpoints` | string[]? | Files/modules you touched. |

**Returns:** `TaskCard`

---

### `portbay_task_check` (mutates state)

Tick (or, with `done: false`, reopen) a checklist item by its index, to report sub-step progress as you work.

| Arg | Type | Notes |
| --- | --- | --- |
| `project` | string | **Required.** Project id (slug). |
| `id` | string | **Required.** Card id. |
| `idx` | number | **Required.** Checklist item index. |
| `done` | bool? | `true` to tick (default), `false` to reopen. |
| `run_id` | string? | Your dispatch run id. |

**Returns:** `TaskCard`

---

### `portbay_task_checklist_add` (mutates state)

Append sub-task items to a card's checklist — your own breakdown (e.g. P0/P1/P2 steps) — then tick them with `portbay_task_check` as you finish each.

| Arg | Type | Notes |
| --- | --- | --- |
| `project` | string | **Required.** Project id (slug). |
| `id` | string | **Required.** Card id. |
| `items` | string[] | **Required.** Checklist item descriptions to append. |
| `label` | string? | Optional checklist heading (e.g. `Steps`). |
| `run_id` | string? | Your dispatch run id. |

**Returns:** `TaskCard`

---

### `portbay_task_comment` (mutates state)

Post a comment on a card. Shows in the card's activity thread — record a decision or ask the human something.

| Arg | Type | Notes |
| --- | --- | --- |
| `project` | string | **Required.** Project id (slug). |
| `id` | string | **Required.** Card id. |
| `text` | string | **Required.** Comment body. |
| `run_id` | string? | Your dispatch run id. |

**Returns:** `TaskCard`

---

### `portbay_handoff_get` (read-only)

Read the project's continuation brief — the minimal "where we left off" note. Read this **first** when picking up work; trust it over your own memory.

| Arg | Type | Notes |
| --- | --- | --- |
| `project` | string | **Required.** Project id (slug). |

**Returns:** `HandoffView`

---

### `portbay_handoff_update` (mutates state)

Append a **minimal** entry to the rolling hand-off log (what changed, the next concrete step, open items, pointers). It's prepended as the newest entry; older entries are kept until the log hits its size cap, then the oldest are pruned. Sign it with `author`. Call this before you finish a card or end a session.

| Arg | Type | Notes |
| --- | --- | --- |
| `project` | string | **Required.** Project id (slug). |
| `narrative` | string | **Required.** The entry to prepend. |
| `author` | string? | Your agent name (defaults to the calling agent). |

**Returns:** `HandoffView`

---

## Common output types

### `ProjectSummary`

Returned by list, status, and most mutation results.

| Field | Type | Notes |
| --- | --- | --- |
| `id` | string | Stable slug. Pass this to all other tools. |
| `name` | string | Human-readable display name. |
| `kind` | string | Framework: `next`, `vite`, `php`, `static`, `node`, `flutter`, `xcode`, `android`, `custom`. |
| `hostname` | string | Hostname without scheme. |
| `url` | string | Full URL (`https://` or `http://` + hostname). |
| `https` | bool | Whether HTTPS is enabled. |
| `port` | number? | Dev server port, when set. |
| `status` | string | `running` / `starting` / `stopped` / `crashed` / `unhealthy` / `port_conflict` / `unknown` (when daemon is down). |
| `pid` | number? | Process id when running. |
| `restarts` | number? | Restart count since last start. |
| `ready` | string? | Last readiness-probe result (e.g. `Ready`), when known. |

### `OpResult`

Acknowledgement returned by all mutation and lifecycle tools.

| Field | Type | Notes |
| --- | --- | --- |
| `ok` | bool | `true` on success. |
| `project` | `ProjectSummary?` | The affected project, when applicable. |
| `detail` | string | Human-readable summary of what happened. |
| `warnings` | string[] | Non-fatal issues (e.g. `/etc/hosts` couldn't be updated without sudo; pending database provisioning). May be non-empty even on success. |

### `DatabaseInstanceSummary`

Returned by `portbay_list_databases` and database mutation results.

| Field | Type | Notes |
| --- | --- | --- |
| `id` | string | Stable slug id — pass to start/stop/remove/link tools. |
| `name` | string | Human-readable name. |
| `engine` | string | Engine id (`mysql`, `postgres`, `mariadb`, `redis`, `mongo`, `memcached`). |
| `engine_label` | string | Human-readable engine name. |
| `version` | string | Version string the engine reported at provisioning. |
| `port` | number | Bound port. |
| `status` | string | `running`, `starting`, `errored`, `stopped`, or `unknown`. |
| `auto_start` | bool | Whether the instance auto-starts on daemon boot. |
| `data_dir` | string | Absolute path to the instance's data directory. |
| `config_path` | string? | Absolute path to the generated config file, when applicable. |
| `socket_path` | string? | Absolute path to the Unix socket, when applicable. |
| `connection_url` | string | Connection URL (e.g. `mysql://root@127.0.0.1:3306/`). |
| `account` | string | Default provisioned account. |
| `linked_projects` | string[] | Project ids whose env receives this instance's connection vars. |
| `binary_available` | bool | Whether the engine daemon binary is currently on PATH. |
| `provisioned` | bool | Whether the data directory has been initialized. |

### `DatabaseOpResult`

Acknowledgement returned by database mutation tools.

| Field | Type | Notes |
| --- | --- | --- |
| `ok` | bool | `true` on success. |
| `detail` | string | Human-readable summary. |
| `instance` | `DatabaseInstanceSummary?` | The affected instance, when applicable. |
| `warnings` | string[] | Non-fatal issues. May be non-empty even on success. |

### `GroupSummary`

Returned by group CRUD operations and `portbay_list_groups`.

| Field | Type | Notes |
| --- | --- | --- |
| `id` | string | Stable slug id. |
| `name` | string | Human-readable display name. |
| `project_ids` | string[] | All member project ids (may include stale ids). |
| `known_ids` | string[] | Subset of `project_ids` that currently exist in the registry. |
| `member_count` | number | Total member count (including stale). |

### `GroupFanoutResult`

Returned by group lifecycle operations.

| Field | Type | Notes |
| --- | --- | --- |
| `group_id` | string | The group that was acted on. |
| `succeeded` | number | Count of members that succeeded (or were skipped). |
| `failed` | number | Count of members that failed. |
| `results` | object[] | Per-member: `project_id`, `ok` (bool), `error` (string, omitted on success). |

### `TunnelStatus`

Returned by tunnel tools.

| Field | Type | Notes |
| --- | --- | --- |
| `projectId` | string | The PortBay project this tunnel is for. |
| `upstreamUrl` | string | The local origin the tunnel proxies to. |
| `publicUrl` | string? | The Cloudflare share URL; `null` while still being assigned. |
| `running` | bool | Whether the cloudflared child process is still alive. |
| `originReachable` | bool? | Whether the local origin is reachable; `null` until probed. |
| `startedAtMs` | number | Unix milliseconds when the tunnel started. |

### `RuntimeLanguageSummary`

Returned by runtime tools.

| Field | Type | Notes |
| --- | --- | --- |
| `id` | string | Stable language id (`php`, `node`, `python`, `bun`, `go`, `ruby`, `flutter`). |
| `display_name` | string | Human-readable label (e.g. `"PHP"`, `"Node.js"`). |
| `default_version` | string? | Version label configured as the default; `null` when none is set. |
| `versions` | `RuntimeVersionSummary[]` | All detected + manually-added versions. |
| `install_hint` | string | Suggested install command when no versions are detected. |

`RuntimeVersionSummary` fields:

| Field | Type | Notes |
| --- | --- | --- |
| `version` | string | Version label (e.g. `"8.3"`, `"22.11.0"`). |
| `source` | string | Where the install came from: `homebrew`, `asdf`, `mise`, `nvm`, `pyenv`, `system`, `manual`, … |
| `binary` | string | Absolute path to the primary binary. |
| `is_default` | bool | Whether this version is the language's configured default. |

---

### `TaskCard`

One card on a project's board. Returned by the task tools above.

| Field | Type | Notes |
| --- | --- | --- |
| `id` | string | Card id (stable; the Markdown file's name). |
| `title` | string | Card title. |
| `status` | string | Column: `Backlog` / `Todo` / `InProgress` / `Blocked` / `Review` / `Done` / `Rejected`. |
| `body` | string | Markdown description. |
| `priority` | string? | `critical` / `high` / `medium` / `low`. |
| `labels` | string[]? | Label ids (board config maps them to colours). |
| `acceptance` | string? | Acceptance criteria. |
| `touchpoints` | string[]? | Files/modules the work touches. |
| `checklist` | object? | `{ label?, items: { idx, desc, done }[] }`. |
| `blockedBy` | string[]? | Card ids that must reach a terminal column before this one can dispatch. |
| `agent` | string? | Assigned agent (`claude`, `codex`, `cursor`, `gemini`, `aider`, …). |
| `claim` | object? | Live run claim `{ host, runId, at }` while an agent is working the card. |
| `created` | string | ISO-8601 creation time. |
| `updated` | string? | ISO-8601 of last activity. |

---

### `HandoffView`

The parsed `.portbay/HANDOFF.md` continuation log.

| Field | Type | Notes |
| --- | --- | --- |
| `updated` | string | ISO-8601 of the most recent entry. |
| `maxChars` | number | Size cap; oldest entries are pruned past it. |
| `autoGenerated` | bool | Whether PortBay derived the brief vs. an agent/human wrote it. |
| `body` | string | The rolling log — newest `## <timestamp> · <author>` entry first. |

---

## Resources

The server exposes read-only [MCP resources](https://modelcontextprotocol.io/specification/2025-11-25/server/resources) an agent can read into its context without making tool calls. All resources return `application/json`.

### Static resources

| URI | Contents |
| --- | --- |
| `portbay://registry` | The full PortBay registry as JSON — every project and its config. |
| `portbay://doctor` | Environment health snapshot. Same data as `portbay_doctor`. |
| `portbay://sidecars` | Sidecar status snapshot. Same data as `portbay_sidecar_status`. |
| `portbay://recipes` | The stack-recipe catalog. Same data as `portbay_list_recipes`. |

### Resource templates

| URI template | Contents |
| --- | --- |
| `portbay://projects/{id}` | Live status + config for a single project, by id. |
| `portbay://projects/{id}/logs` | Recent log tail for a single project (200 lines). |
| `portbay://project/{id}/context` | Derived project context — URL, ports, runtime, web server, database vars, services. |
| `portbay://project/{id}/tasks` | The full task board for a project (all cards, by column). |
| `portbay://project/{id}/handoff` | The project's rolling hand-off log (continuation brief). |

---

## Error envelope (`isError`)

When a tool call fails, the result has `isError: true` and `structuredContent` carries PortBay's standard error envelope instead of the success type. The agent reads the envelope and can recover or tell the user the next step.

```json
{
  "code": "SIDECAR_DOWN",
  "whatHappened": "process-compose is not running",
  "whyItMatters": "Projects can't start until process-compose is running again.",
  "whoCausedIt": "system",
  "actions": [{ "label": "Restart process-compose", "command": "sidecars.restart_pc" }]
}
```

Resources do not have an `isError` channel. A failed resource read is reported as a protocol-level error.

Common error codes:

| Code | Meaning |
| --- | --- |
| `PROJECT_NOT_FOUND` | No project with that id in the registry. |
| `SIDECAR_DOWN` | Process Compose daemon not reachable — open the PortBay app. |
| `PORT_CONFLICT` | The configured port is in use. |
| `PROJECT_CAP_REACHED` | Project limit for the current tier reached. Sign in or upgrade to Pro. |
| `BAD_INPUT` | An argument was invalid, malformed, or a required path was missing. |
| `REGISTRY` | The registry file could not be read or written. |
| `INTERNAL` | Unexpected internal failure. |
