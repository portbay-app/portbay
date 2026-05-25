# MCP Tool Reference

The full surface exposed by `portbay-mcp`. Every tool returns `structuredContent` (typed JSON Schema output) plus a plain-text mirror for clients that don't support structured content. Tools declare behavior annotations: `readOnlyHint` for read-only tools, `destructiveHint` for destructive ones, `idempotentHint` where applicable.

Tools are grouped into [toolsets](./index.md#governance) you can enable or disable with `--toolsets`.

**19 tools · 4 static resources · 2 resource templates**

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

Run an environment health check. Checks: registry readability, daemon reachability on the configured port, `mkcert` / `caddy` / `process-compose` on PATH, current license tier. Use when something is broken and you don't yet know what.

No arguments.

**Returns:** `DoctorResult`

| Field | Type | Notes |
| --- | --- | --- |
| `ok` | bool | `true` when no check returned `fail`. |
| `findings` | `DoctorFinding[]` | Each finding has `check` (string), `verdict` (`ok` / `warn` / `fail`), and `detail` (string). |

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
