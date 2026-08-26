---
title: PortBay CLI Reference — Commands, Flags & Exit Codes
description: "Full reference for the portbay CLI: add, start, stop, logs, doctor, hosts, export, login, and license commands with all flags, defaults, and exit code meanings."
---

# CLI Reference

The `portbay` CLI shares the same `portbay_lib` core as the Tauri GUI. It acts as a client: lifecycle commands (`start`, `stop`, `restart`) require the PortBay daemon (the GUI app, or a future `portbay daemon` subcommand) to be running and exposing Process Compose on a discoverable port.

Typed without a management subcommand, `portbay` starts the local PortBay agent engine, similar to `codex` or `claude`:

```bash
portbay
portbay "inspect this repo and summarize the next fix"
portbay --json -P ollama -m qwen2.5-coder:latest "run the task"
```

The command is lowercase on disk and case-insensitive wherever PortBay parses it as an agent id.

## Install

The app ships the CLI inside its bundle. To put it on `$PATH`, open **Settings → Advanced** and enable the **Command-line tool** row — this symlinks the bundled binary to `/usr/local/bin/portbay` (one OS authorization prompt at most, VS Code's "install `code` command" model). The agent engine ships in the same bundle and is resolved through that symlink automatically; no second install step is needed. Homebrew installs (`brew install portbay-app/portbay/portbay`) land on `$PATH` directly.

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

## Agent mode — retired

`portbay` used to double as an agent CLI: anything that wasn't a management
subcommand was handed to a bundled `portbay-agent` engine, so
`portbay "fix the failing test"`, `portbay -p "plan the refactor"` and
`portbay --json "run the task"` all started an agent run.

**That engine no longer ships.** PortBay runs agent work on its own native
in-process runtime instead of bundling a second engine, and there is no CLI
entry point that takes a bare prompt. The invocations above now print what
happened and exit `2` rather than doing something else quietly:

```
$ portbay -p "plan the refactor"
portbay: `-p` is not a PortBay command.

The bundled `portbay-agent` engine that used to run free-text prompts and
agent flags typed at `portbay` has been retired. PortBay runs agent work on
its own native runtime now instead of shipping a second engine, and there is
no CLI entry point that takes a bare prompt.

  portbay                       open the PortBay terminal
  portbay tui --project <id> --card <id>
                                run a board card on the native runtime
  portbay tasks list <project>  read the board from the shell
  portbay --help                every command this build has
```

What replaces each old use:

| Was | Now |
| --- | --- |
| `portbay` (interactive session) | `portbay` still opens PortBay's own terminal. |
| `portbay "<prompt>"` / `-p` / `-i` | Put the work on the board and dispatch the card — `portbay tui --project <id> --card <id>` runs it on the native runtime. |
| `portbay --json "<prompt>"` (headless NDJSON) | Headless board dispatch, over the MCP server or the app. The old `agent_event` / `run_result` NDJSON contract belonged to the retired engine. |
| `PORTBAY_AGENT_BIN` | Nothing. No code path resolves an agent sidecar any more. |

Local-model board dispatches are unaffected in the place that matters: an
**Ollama** or **Cloud (BYOK)** card still runs a full agentic read/edit/run
loop, on the native runtime rather than through the bundled engine.

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

PortBay-bundled sidecars (Caddy, mkcert, Mailpit, cloudflared) are **never resolved from `$PATH`** on macOS — they ship with the app, so a foreign install is never mistaken for PortBay's own. On Linux, `dnsmasq` is intentionally distro-managed: PortBay's sidecar wrapper invokes the system package. Live sidecar state isn't observable from outside the daemon, so these checks report as informational rather than a guessed process state, mirroring `portbay sidecar status`.

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
