# MCP Tool Reference

The full tool surface exposed by `portbay-mcp`. Every tool returns structured JSON (`structuredContent`) plus a text mirror, and declares behavior annotations so a client can gate destructive calls. Tools are grouped into [toolsets](./#governance) you can enable or disable.

Legend: 🔍 read-only · ✏️ mutates state · 💥 destructive (confirm first)

## Projects

### `portbay_list_projects` 🔍
List every registered project with hostname, URL, and — when the daemon is running — live status, PID, and restart count. `daemon_reachable: false` means only registry data is shown. No arguments.

### `portbay_status` 🔍
Live runtime detail for one project (`id`) or all (omit `id`): status, PID, restarts, last readiness result.

### `portbay_detect_project` 🔍
Inspect a folder and return the detected framework plus suggested defaults (id, hostname, port, start command). Nothing is registered — a preview to confirm with the user before adding. Args: `path`.

### `portbay_add_project` ✏️
Register an existing folder: local hostname, optional HTTPS, managed start/stop. Omit `kind` to auto-detect. Does **not** start the project. Args: `path` (required), `name?`, `hostname?`, `kind?`, `port?`, `start_command?`, `https?`, `auto_start?`, `php_version?`, `document_root?`.

### `portbay_update_project` ✏️
Patch fields on an existing project; only the fields you pass change. Args: `id` (required) + any of `name`, `hostname`, `port`, `start_command`, `https`, `auto_start`, `tags`. Idempotent.

### `portbay_remove_project` 💥
Unregister a project and clean up its cert and hosts entry. **Source files on disk are not touched.** Args: `id`.

### `portbay_export_config` ✏️
Write a `.portbay.json` into the project folder so its setup can be committed and reproduced. Secret values are never written — only their names. Args: `id`. Idempotent.

### `portbay_import_config` ✏️
Register a project from a committed `.portbay.json`. Args: `path` (folder or the file itself), `secrets?` (key→value map for declared secrets; omitted ones become empty placeholders).

### `portbay_setup` ✏️
The one-call "set this up for me" flow: register an existing folder (auto-detecting the framework) and start it, returning the live URL. Args: `path` (required), `name?`, `hostname?`, `kind?`, `port?`, `start_command?`, `https?`, `start_now?` (default `true`), `auto_launch?`.

### `portbay_list_recipes` 🔍
List the available **stack recipes** — named blueprints (`laravel`, `next`, `vite`, …) that compose a project's framework, language version, document root, and HTTPS in one step. Map the user's request to a recipe id, then call `portbay_setup_from_recipe`. A recipe with `composes_fully: false` also recommends a database or mail service that isn't auto-provisioned yet (the project still registers, with a note). No arguments.

### `portbay_setup_from_recipe` ✏️
Apply a recipe to an existing folder: register it with the recipe's stack and start it. The fastest path when the user names a stack ("set up a Laravel app at `~/code/blog`"). Args: `recipe` (required), `path` (required), `name?`, `hostname?`, `php_version?`, `https?`, `start_now?` (default `true`), `auto_launch?`. For a brand-new project from scratch, use `portbay_setup_from_template`.

## Lifecycle

Lifecycle tools require the PortBay daemon to be running (see [coordination](./#coordination)).

### `portbay_start` ✏️
Start a project. Args: `id` (required), `auto_launch?` — when `true`, opens the PortBay app if the daemon is down, then retries. Idempotent.

### `portbay_stop` ✏️
Stop a running project. Args: `id`. Idempotent.

### `portbay_restart` ✏️
Restart a project (stop then start). Args: `id`. Idempotent.

### `portbay_stop_all` ✏️
Stop every running PortBay process — the universal kill switch. No arguments. Idempotent.

## Diagnostics

### `portbay_logs` 🔍
Recent log output for a project — the first thing to read when a project won't start. Args: `id` (required), `lines?` (default 200), `offset?` (default 0). Requires the daemon.

### `portbay_doctor` 🔍
Environment health check: registry readability, daemon reachability, required tooling on PATH, license tier. No arguments.

### `portbay_sidecar_status` 🔍
State of PortBay's background services. Process Compose is probed directly; the others (Caddy, mkcert, dnsmasq, mailpit) are managed by the daemon and reported as install-presence only. No arguments.

## Scaffold

### `portbay_setup_from_template` ✏️
Scaffold a brand-new project from a starter template into `parent_path/name`, then register it. Runs the upstream scaffolder (pnpm/composer) — may take a while and needs network access. Args: `template` (`nextjs` \| `vite` \| `astro` \| `laravel` \| `php`), `parent_path`, `name`, `start_now?` (default `false`).

## Resources

The server also exposes read-only [resources](https://modelcontextprotocol.io/specification/2025-11-25/server/resources) an agent can pull into its context:

| URI | Contents |
| --- | --- |
| `portbay://registry` | The full registry as JSON. |
| `portbay://doctor` | Environment health snapshot. |
| `portbay://sidecars` | Sidecar status snapshot. |
| `portbay://recipes` | The stack-recipe catalog. |
| `portbay://projects/{id}` | Live status + config for one project. |
| `portbay://projects/{id}/logs` | Recent log tail for one project. |
