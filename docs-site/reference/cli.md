---
title: PortBay CLI Reference — Commands, Flags & Exit Codes
description: "Full reference for the portbay CLI: add, start, stop, logs, doctor, hosts, export, login, and license commands with all flags, defaults, and exit code meanings."
---

# CLI Reference

The `portbay` CLI shares the same `portbay_lib` core as the Tauri GUI. It acts as a client: lifecycle commands (`start`, `stop`, `restart`) require the PortBay daemon (the GUI app, or a future `portbay daemon` subcommand) to be running and exposing Process Compose on a discoverable port.

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
| `portbay status [id]` | Show one project's status, or all projects when no id is provided. |
| `portbay add <PATH>` | Register a project from a folder path. Auto-detects `.portbay.json` if present. |
| `portbay remove <id>` | Unregister a project and remove generated artifacts by default. |
| `portbay start <id>` | Start one project. |
| `portbay stop <id>` | Stop one project. |
| `portbay stop --all` | Stop every running process. |
| `portbay restart <id>` | Restart one project. |
| `portbay logs <id>` | Print static log output for a project. |
| `portbay open <id>` | Open the project URL in the default browser. |
| `portbay doctor` | Grouped, `flutter doctor`-style health check across registry, daemon, routing/TLS, PHP, services, and account. |
| `portbay hosts <subcommand>` | Manage PortBay's `/etc/hosts` block. |
| `portbay export <id>` | Write `<project_path>/.portbay.json`. |
| `portbay completions <shell>` | Generate shell completion scripts. |
| `portbay login [--email <addr>]` | Sign in to PortBay Cloud (GitHub OAuth by default; `--email` for a magic link). |
| `portbay license` | Show the current account, tier, and entitlement limits. |
| `portbay logout` | Sign out and clear the saved session. |

## `add`

If the target folder contains a `.portbay.json`, `add` reads it and imports the project from that file. Otherwise it registers the project from the supplied flags.

```bash
portbay add <PATH> \
  --id <id> \
  --name <name> \
  --hostname <hostname> \
  --kind next|vite|php|static|node|flutter|xcode|android|custom \
  --port <port> \
  --start-command <command> \
  --document-root <relative-path> \
  --php-version <version> \
  --web-server caddy|nginx|apache \
  --https true|false \
  --auto-start
```

`--kind` defaults to `custom`. `--https` defaults to `true`. `--web-server` defaults to `caddy` and is only applied when `--kind php` is set and no `--start-command` is given.

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

`--limit` defaults to 200. `--offset` defaults to 0 (newest).

## `doctor`

```bash
portbay doctor
portbay --json doctor
```

A `flutter doctor`-style environment report. Checks are grouped into categories; each category header shows the worst verdict among its rows (`[✓]` ok · `[!]` warning · `[✗]` fatal), and every row carries an inline fix hint.

| Category | Checks |
| --- | --- |
| **Core** | Registry loads (project count, schema version, domain suffix); Process Compose daemon reachability; `/etc/hosts` managed entries reconciled against the registry. |
| **Web routing & TLS** | Caddy and mkcert (bundled sidecars — see note); local certificate count under the certs directory. |
| **PHP runtimes** | Every detected PHP install (version, path, source), flagged when `php-fpm` is missing so it can't serve sites. |
| **Services** | dnsmasq resolver routing for the wildcard suffix; Mailpit (bundled sidecar); available database engines (MySQL, MariaDB, Postgres, Redis, Mongo, Memcached). |
| **Account & sharing** | Signed-in account, tier, and project cap; active tunnel count. |

PortBay-bundled sidecars (Caddy, mkcert, dnsmasq, Mailpit, cloudflared) are **never resolved from `$PATH`** — they ship with the app, so a foreign install (e.g. another local dev tool) is never mistaken for PortBay's own. Their live state isn't observable from outside the daemon, so they report as informational rather than a guessed binary path, mirroring `portbay sidecar status`.

`doctor` exits `0` even when warnings are present; it returns a non-zero code only when a check is fatal (e.g. the registry fails to load). With `--json` it prints an array of categories, each with a `verdict` and a `checks` array of `{ check, verdict, detail }` objects.

## `hosts`

```bash
portbay hosts list
portbay hosts add <hostname> --ip 127.0.0.1
portbay hosts remove <hostname>
portbay hosts clear
portbay hosts reconcile
```

`hosts add` defaults `--ip` to `127.0.0.1`. All write operations (`add`, `remove`, `clear`, `reconcile`) try the bundled `portbay-hosts-helper` sidecar first; they fall back to direct `/etc/hosts` writes, which require elevated privileges.

## `completions`

```bash
portbay completions bash
portbay completions zsh
portbay completions fish
portbay completions powershell
```

## `login`

```bash
portbay login               # GitHub OAuth (opens browser)
portbay login --email <addr>  # email magic link
```

Drives the GitHub OAuth or email magic-link flow from the terminal, then stores the session in the OS keychain shared with the GUI. Polls for up to 5 minutes. Prints the signed-in username and tier on success.

## `license`

```bash
portbay license
```

Prints the cached effective entitlement: account login, tier (`anonymous` / `free` / `pro`), project cap, sync, and mail entitlements.

## `logout`

```bash
portbay logout
```

Clears the saved session and cached entitlement from the OS keychain.

## Exit Codes

| Code | Meaning |
| --- | --- |
| `0` | Success |
| `1` | Generic failure |
| `2` | User input error (bad project id, missing argument) |
| `3` | Daemon unreachable |
| `4` | Port conflict |
| `5` | Readiness timeout (reserved) |
| `6` | Permission denied (hosts write requires elevated privileges) |
