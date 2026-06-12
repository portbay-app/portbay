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

## Agent mode

Anything that isn't a management subcommand proxies to the bundled `portbay-agent` engine — the same parity you get from typing `codex` or `claude`. Bare `portbay` opens an interactive session; a quoted free-text argument runs that prompt; `--json` runs headless for automation. Local-model board dispatches use the same engine internally: the visible card agent can stay **Ollama**, while PortBay launches the engine with `-P ollama -m <selected-model>`.

```bash
portbay                          # interactive session (TTY required)
portbay -i                       # force the full TUI
portbay "fix the failing test"   # one-shot run, act mode
portbay -p "plan the refactor"   # plan mode: propose before touching files
portbay --json "run the task"    # headless NDJSON for scripts and CI
```

The engine resolves `portbay-agent` beside the `portbay` binary (following the install symlink back into the app bundle). Set `PORTBAY_AGENT_BIN` to point at a different engine build.

Agent-mode flags:

| Option | Meaning |
| --- | --- |
| `--json` | Emit NDJSON events (one JSON object per line) instead of styled text. |
| `--auto-approve <bool>` | Tool auto-approval for the run. Defaults to `true` for one-shot prompts; pass `false` to gate each tool use. |
| `-p, --plan` | Plan mode — propose an approach before acting. Act is the default. |
| `-i, --tui` | Open the full terminal UI for an interactive session. |
| `-P, --provider <id>` | Provider id. PortBay defaults to `ollama`. |
| `-m, --model <id>` | Model id, such as `qwen2.5-coder:latest`. |
| `-c, --cwd <path>` | Working directory for the run. |
| `--id <session-id>` | Resume an existing session. |
| `--thinking <level>` | Reasoning effort: `none\|low\|medium\|high\|xhigh`. |
| `--worktree` | Run the task in an auto-created detached git worktree. |
| `--retries <n>` | Max consecutive mistakes before exiting (default 6). |
| `-t, --timeout <seconds>` | Hard run timeout (default 0 = none). |

### Headless `--json` contract

With `--json`, the run emits newline-delimited JSON on stdout. Every line has a `ts` timestamp and a `type`:

| `type` | Meaning |
| --- | --- |
| `hook_event` | Lifecycle hooks (`agent_start`, `agent_error`, …) with `agentId` / `taskId`. |
| `agent_event` | The run stream: `iteration_start`, content and tool events, and `error` (with `recoverable`). |
| `run_result` | Terminal summary: `finishReason`, `iterations`, token `usage` and `totalCost`, `durationMs`, the final `text`, and the resolved `model` (`{id, provider}`). |
| `error` | Terminal failure message (also reflected in `run_result.finishReason: "error"`). |

A failed run still ends with a well-formed `run_result`, so automation can always key off the last `run_result` line:

```json
{"ts":"2026-06-10T17:41:12.541Z","type":"run_result","finishReason":"error","iterations":1,"usage":{"inputTokens":0,"outputTokens":0,"cacheReadTokens":0,"cacheWriteTokens":0,"totalCost":0},"durationMs":6221,"text":"Can't connect to Ollama at http://localhost:11434/v1 — …","model":{"id":"qwen2.5:7b","provider":"ollama"}}
```

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
