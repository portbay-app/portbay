# Drive PortBay from an AI Agent (MCP)

PortBay ships an [**Model Context Protocol**](https://modelcontextprotocol.io) server, `portbay-mcp`. Any MCP-aware agent — Claude Code, Cursor, Continue, Zed, Windsurf, and others — can drive PortBay directly: register projects, start and stop them, read logs, and diagnose failures, without you clicking through the GUI or remembering CLI flags.

The agent spawns `portbay-mcp` as a subprocess and talks to it over stdio. The process boundary **is** the trust boundary — there's no port to open and no extra auth to configure.

> [!TIP]
> New to this? The fastest path: install PortBay, add the two-line config below for your editor, then tell your agent: *"Set up the app I just scaffolded at `~/code/blog`."* The agent detects the framework, registers it, starts it, and replies with the `https://blog.test` URL.

## What it's good for

- **Vibe coders** — "set this up for me" in one sentence. The agent detects your framework, gives you a real hostname with HTTPS, and starts it. One tool call (`portbay_setup`) does detect → register → start.
- **Senior / platform engineers** — every tool has a typed JSON Schema, structured output, and honest behavior annotations (`readOnlyHint`, `destructiveHint`). Lock the surface down with [read-only mode and toolsets](#governance-read-only-and-toolsets) for repeatable, auditable automation.

## Install

`portbay-mcp` is installed alongside the PortBay app and CLI.

::: code-group
```bash [Homebrew]
brew install portbay-app/portbay/portbay
# installs `portbay`, `portbay-mcp`, and the PortBay app
which portbay-mcp
```
```bash [From source]
cargo build --release --features mcp --bin portbay-mcp
# binary at target/release/portbay-mcp
```
:::

Confirm it runs (it waits for an MCP client on stdio — press <kbd>Ctrl-C</kbd>):

```bash
portbay-mcp --help
```

## Configure your agent

Point your agent at the `portbay-mcp` binary. Use the absolute path from `which portbay-mcp` (commonly `/opt/homebrew/bin/portbay-mcp` on Apple Silicon).

### Claude Code

```bash
claude mcp add portbay -- /opt/homebrew/bin/portbay-mcp
```

Or edit `~/.claude.json` (or a project `.mcp.json`) directly:

```json
{
  "mcpServers": {
    "portbay": {
      "command": "/opt/homebrew/bin/portbay-mcp"
    }
  }
}
```

### Cursor

`~/.cursor/mcp.json` (global) or `.cursor/mcp.json` (per-project):

```json
{
  "mcpServers": {
    "portbay": {
      "command": "/opt/homebrew/bin/portbay-mcp"
    }
  }
}
```

### Continue

`~/.continue/config.yaml`:

```yaml
mcpServers:
  - name: PortBay
    command: /opt/homebrew/bin/portbay-mcp
```

### Zed

`settings.json` → `context_servers`:

```json
{
  "context_servers": {
    "portbay": {
      "command": { "path": "/opt/homebrew/bin/portbay-mcp", "args": [] }
    }
  }
}
```

### Windsurf

`~/.codeium/windsurf/mcp_config.json`:

```json
{
  "mcpServers": {
    "portbay": {
      "command": "/opt/homebrew/bin/portbay-mcp"
    }
  }
}
```

After adding the config, restart the agent. PortBay's tools (prefixed `portbay_`) will appear in its tool list.

## Try it

Once configured, talk to your agent in plain language:

- *"Set up `myapp.test` for the folder I just scaffolded at `~/code/myapp`."*
  → `portbay_detect_project` → `portbay_add_project` → `portbay_start` → reports the URL.
- *"Why isn't my Laravel project starting?"*
  → `portbay_status` → `portbay_logs` → `portbay_doctor`, then explains the fix.
- *"Show me the logs for `blog`."* → `portbay_logs`.
- *"Add the project at `~/code/api` with HTTPS but don't start it yet."* → `portbay_add_project`.
- *"Stop everything."* → `portbay_stop_all`.
- *"Scaffold a new Next.js app called `dashboard` in `~/code` and set it up."* → `portbay_setup_from_template`.
- *"Set up a Laravel app for the folder at `~/code/blog`."* → `portbay_list_recipes` → `portbay_setup_from_recipe`.

See the [Tool Reference](./tools) for the full inventory.

## Stack recipes

Recipes are named blueprints — `laravel`, `next`, `vite`, `astro`, `node`, `static`, `php`, `symfony`, `statamic` — that compose a project's framework, language version, document root, and HTTPS in one step. The agent maps your sentence to a recipe; PortBay applies it deterministically (no server-side model involved). Browse them with `portbay_list_recipes` (or the `portbay://recipes` resource), then apply one with `portbay_setup_from_recipe`.

Recipes that also recommend a database or local mail catcher (e.g. `laravel`) report `composes_fully: false`: the project is registered with everything PortBay can wire today, and the recommended service is surfaced as a note to add from the app — automatic provisioning of those services is on the roadmap.

## How it coordinates with the app {#coordination}

`portbay-mcp` is a *client* of the same system the GUI and CLI use — it never runs its own copy of the engine:

- **Registry changes** (add / update / remove / import / export) are written to PortBay's registry file. They take effect **even if the PortBay app isn't running** — the app applies them when it next launches (its reconcile loop converges certs, Caddy routes, and `/etc/hosts`).
- **Lifecycle actions** (start / stop / restart / logs) talk to the running daemon. If the app isn't running you'll get a `SIDECAR_DOWN` error telling you to open it. Pass `auto_launch: true` on `portbay_start` (or `portbay_setup`) to have the server open the PortBay app for you first — use this only when you're at your machine.

## Governance: read-only and toolsets {#governance}

Two controls scope what an agent can do. Both have flag and environment-variable forms (the env var wins, matching common MCP servers); set them in the `args`/`env` of your agent config.

### Read-only mode

Exposes **only inspection tools** — every tool that mutates state (add/update/remove, start/stop/restart, import/export, scaffolding) is removed entirely. Ideal for "let the agent look but never touch."

```json
{
  "mcpServers": {
    "portbay": {
      "command": "/opt/homebrew/bin/portbay-mcp",
      "args": ["--read-only"]
    }
  }
}
```

Equivalent env var: `PORTBAY_MCP_READ_ONLY=1`.

### Toolsets

Expose only the groups you want. Comma-separated; valid values are `projects`, `lifecycle`, `diagnostics`, `scaffold`, and `all` (the default).

| Toolset | Tools |
| --- | --- |
| `projects` | list, status, detect, add, update, remove, export, import, setup |
| `lifecycle` | start, stop, restart, stop_all |
| `diagnostics` | logs, doctor, sidecar_status |
| `scaffold` | setup_from_template (runs upstream scaffolders; needs network) |

```json
{
  "mcpServers": {
    "portbay": {
      "command": "/opt/homebrew/bin/portbay-mcp",
      "args": ["--toolsets", "projects,diagnostics"]
    }
  }
}
```

Equivalent env var: `PORTBAY_MCP_TOOLSETS=projects,diagnostics`.

Read-only and toolsets compose: `--read-only --toolsets projects,diagnostics` exposes only the read tools within those two groups.

## Error handling

Tool failures come back as MCP tool-execution errors (`isError: true`) carrying PortBay's standard envelope, so the agent can read the failure and recover:

```json
{
  "code": "SIDECAR_DOWN",
  "whatHappened": "process-compose is not running",
  "whyItMatters": "Projects can't start until process-compose is running again.",
  "whoCausedIt": "system",
  "actions": [{ "label": "Restart process-compose", "command": "sidecars.restart_pc" }]
}
```

Common codes: `PROJECT_NOT_FOUND`, `SIDECAR_DOWN`, `PORT_CONFLICT`, `PROJECT_CAP_REACHED`, `BAD_INPUT`. Project caps apply to agent-driven adds exactly as they do in the app (anonymous 3 / free 6 / Pro unlimited).

## All flags

| Flag | Env var | Default | Purpose |
| --- | --- | --- | --- |
| `--read-only` | `PORTBAY_MCP_READ_ONLY` | off | Inspection tools only. |
| `--toolsets <list>` | `PORTBAY_MCP_TOOLSETS` | `all` | Comma-separated tool groups. |
| `--pc-port <port>` | `PORTBAY_PC_PORT` | `9999` | Process Compose daemon port. |
| `--registry <path>` | — | app data dir | Override the registry file location. |
| `--log-level <level>` | `RUST_LOG` | `info` | stderr log verbosity. |

All diagnostic logging goes to **stderr**; stdout carries only the MCP protocol stream.
