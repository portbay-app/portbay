# Registry Schema

The registry is a JSON file stored at:

```text
~/Library/Application Support/PortBay/registry.json
```

It is the source of truth for the GUI, CLI, generated Process Compose config, Caddy routes, and host reconciliation.

## Project

| Field | Type | Required | Notes |
| --- | --- | --- | --- |
| `id` | string | Yes | Stable URL-safe identifier. |
| `name` | string | Yes | Human-readable label. |
| `path` | string | Yes | Project root path. |
| `type` | string | Yes | `next`, `vite`, `php`, `static`, `node`, or `custom`. |
| `start_command` | string | No | Shell command run in `path`. |
| `port` | number | No | Primary HTTP port. |
| `extra_ports` | number[] | No | Additional owned ports. |
| `hostname` | string | Yes | Full local hostname. |
| `https` | boolean | Yes | Whether Caddy terminates local TLS. |
| `services` | string[] | No | Shared services needed by the project. |
| `env` | object | No | Environment variables for the dev process. |
| `readiness` | object | No | HTTP, TCP, or process readiness policy. |
| `auto_start` | boolean | No | Start when the daemon comes up. |
| `tags` | string[] | No | User-defined filtering labels. |
| `document_root` | string | No | PHP document root relative to `path`. |
| `php_version` | string | No | PHP version label. |

## Readiness

```json
{ "type": "http", "path": "/", "timeout_seconds": 75 }
```

```json
{ "type": "tcp", "timeout_seconds": 75 }
```

```json
{ "type": "process" }
```

## Group

```json
{
  "id": "commerce-stack",
  "name": "Commerce Stack",
  "projects": ["storefront", "api"]
}
```

## Portable `.portbay.json`

The exported project file uses camelCase and is intended to be committed to an application repo.

| Field | Type | Notes |
| --- | --- | --- |
| `version` | number | Current schema version is `1`. |
| `name` | string | Project name. |
| `type` | string | Project type. |
| `hostname` | string | Desired local hostname. |
| `port` | number | Optional primary port. |
| `phpVersion` | string | Optional PHP version. |
| `https` | boolean | Local TLS setting. |
| `autoStart` | boolean | Auto-start setting. |
| `startCommand` | string | Optional launch command. |
| `documentRoot` | string | Optional PHP document root. |
| `envTemplate` | object | Non-sensitive environment defaults. |
| `secrets` | string[] | Secret names required at import time. |
| `postInstall` | string[] | Setup commands offered by future import flows. |
| `readiness` | object | Optional readiness policy. |
| `tags` | string[] | Optional labels. |
