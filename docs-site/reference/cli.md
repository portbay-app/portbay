# CLI Reference

Global options:

| Option | Meaning |
| --- | --- |
| `--json` | Emit machine-readable JSON. |
| `--registry <PATH>` | Override the registry file location. |
| `--pc-port <PORT>` | Override the Process Compose daemon port. |

## Commands

| Command | Purpose |
| --- | --- |
| `portbay list` | List registered projects with live status when the daemon is reachable. |
| `portbay status [id]` | Show one project’s status, or all projects when no id is provided. |
| `portbay add <PATH>` | Register a project from a folder path. |
| `portbay remove <id>` | Unregister a project and remove generated artifacts by default. |
| `portbay start <id>` | Start one project. |
| `portbay stop <id>` | Stop one project. |
| `portbay stop --all` | Stop every running process. |
| `portbay restart <id>` | Restart one project. |
| `portbay logs <id>` | Print static log output for a project. |
| `portbay open <id>` | Open the project URL in the default browser. |
| `portbay doctor` | Diagnose runtime, ports, registry, and cert state. |
| `portbay hosts <subcommand>` | Manage PortBay’s `/etc/hosts` block. |
| `portbay export <id>` | Write `<project_path>/.portbay.json`. |

## `add`

```bash
portbay add <PATH> \
  --id <id> \
  --name <name> \
  --hostname <hostname> \
  --kind next|vite|php|static|node|custom \
  --port <port> \
  --start-command <command> \
  --https true|false \
  --auto-start
```

## `remove`

```bash
portbay remove <id>
portbay remove <id> --keep-artifacts
```

`--keep-artifacts` leaves cert files and live Caddy route artifacts in place when reachable.

## `logs`

```bash
portbay logs <id> --limit 200 --offset 0
```

## `hosts`

```bash
portbay hosts list
portbay hosts add <hostname> --ip 127.0.0.1
portbay hosts remove <hostname>
portbay hosts clear
portbay hosts reconcile
```

Host mutations require elevated privileges because they write `/etc/hosts`.

## Exit Codes

| Code | Meaning |
| --- | --- |
| `0` | Success |
| `1` | Generic failure |
| `2` | User input error |
| `3` | Daemon unreachable |
| `4` | Port conflict |
| `5` | Readiness timeout, reserved for future use |
