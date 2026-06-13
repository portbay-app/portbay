---
title: Drive PortBay from an AI Agent via MCP
description: Connect Claude Code, Cursor, Zed, or any MCP-aware agent to PortBay's MCP server to register projects, start servers, and diagnose failures without touching the GUI.
---

# Drive PortBay from an AI Agent (MCP)

PortBay ships an [Model Context Protocol](https://modelcontextprotocol.io) server, `portbay-mcp`. Any MCP-aware agent — Claude Code, Cursor, Codex, Continue, Zed, Windsurf, and others — can drive PortBay directly: register projects, start and stop them, read logs, diagnose failures, and work a project's task board, without clicking through the GUI or remembering CLI flags.

The agent spawns `portbay-mcp` as a subprocess over stdio. The process boundary **is** the trust boundary — there is no port to open and no extra auth layer.

## How it's built and shipped

`portbay-mcp` is its own Rust workspace crate at `src-tauri/crates/mcp/`. It depends on `portbay_lib` with the `mcp` feature gate, which is what pulls in the `rmcp` and `schemars` stacks. Those dependencies are compiled only for the MCP binary, never for the GUI app.

The binary is built by `scripts/build-mcp.sh`, which compiles the crate and drops the result at `src-tauri/binaries/portbay-mcp-<target-triple>` — the location Tauri's bundler reads from its `externalBin` list. The finished PortBay.app includes `portbay-mcp` as a sidecar alongside the other bundled binaries.

To build from source:

```bash
# From the repo root — produces src-tauri/binaries/portbay-mcp-<triple>
./scripts/build-mcp.sh
```

Or build the crate directly (useful when iterating without Tauri):

```bash
cargo build --release -p portbay-mcp
# binary at src-tauri/target/release/portbay-mcp
```

## Install

`portbay-mcp` is installed alongside the PortBay app.

::: code-group
```bash [Homebrew]
brew install portbay-app/portbay/portbay
# installs portbay, portbay-mcp, and the PortBay app
which portbay-mcp
```
```bash [From source]
cargo build --release -p portbay-mcp
# binary at src-tauri/target/release/portbay-mcp
```
:::

Confirm it runs (it blocks waiting for a client on stdin — press <kbd>Ctrl-C</kbd>):

```bash
portbay-mcp --help
```

## Configure your agent

Point your agent at the `portbay-mcp` binary. Use the absolute path from `which portbay-mcp` (commonly `/opt/homebrew/bin/portbay-mcp` on Apple Silicon).

### Claude Code

```bash
claude mcp add portbay -- /opt/homebrew/bin/portbay-mcp
```

Or edit `~/.claude.json` (global) or a project `.mcp.json` directly:

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

### Continue

`~/.continue/config.yaml`:

```yaml
mcpServers:
  - name: PortBay
    command: /opt/homebrew/bin/portbay-mcp
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

### Codex

`~/.codex/config.toml`:

```toml
[mcp_servers.portbay]
command = "/opt/homebrew/bin/portbay-mcp"
```

After adding the config, restart the agent. PortBay's tools (all prefixed `portbay_`) appear in its tool list.

## End-to-end walkthrough

This shows how an agent brings a freshly-scaffolded Next.js app online and diagnoses a crash, step by step.

**Scenario:** you ran `pnpm create next-app` into `~/code/dashboard` and want it running at `https://dashboard.test`.

**1. Register and start in one call**

```text
You: Set up the app I just scaffolded at ~/code/dashboard.
```

The agent calls `portbay_setup` with `path: "/Users/me/code/dashboard"`. The tool:

1. Auto-detects the framework (`next`) from the folder contents.
2. Registers it in the PortBay registry with hostname `dashboard.test` and HTTPS via mkcert.
3. Starts it via the Process Compose daemon.
4. Returns `{ ok: true, project: { url: "https://dashboard.test", status: "running" }, detail: "Registered dashboard at dashboard.test (HTTPS). Started dashboard." }`.

The agent reports: *"Your app is running at https://dashboard.test."*

**2. Diagnosing a crash**

```text
You: My dashboard project stopped with an error.
```

The agent runs three tools in sequence:

1. `portbay_status` with `id: "dashboard"` → sees `status: "crashed"`, `restarts: 3`.
2. `portbay_logs` with `id: "dashboard", lines: 50` → reads the tail of the process log.
3. (If the environment looks suspect) `portbay_doctor` → checks the registry, daemon, and required tooling.

It then explains what the log says and suggests a fix. If the fix involves a config change, it calls `portbay_update_project` and `portbay_restart`.

**3. Registering without starting**

```text
You: Add the API at ~/code/api with HTTPS but don't start it yet.
```

The agent calls `portbay_detect_project` to preview the suggested defaults, confirms them with you, then calls `portbay_add_project` with `https: true`. It does **not** call `portbay_start`. You start it later from the app or by asking again.

**4. Scaffolding from scratch**

```text
You: Create a new Laravel project called blog in ~/code and set it up.
```

The agent calls `portbay_list_recipes` to confirm the `laravel` recipe exists, then `portbay_setup_from_recipe` with `recipe: "laravel", path: "/Users/me/code/blog"`. The recipe registers the project at `blog.test` with PHP-FPM, Caddy, and HTTPS. Because Laravel expects a MySQL database that PortBay can't provision yet, the result includes a warning — the project is registered and usable, with a note to add a database from the app's Databases panel.

For a brand-new scaffold (no folder yet), the agent calls `portbay_setup_from_template` instead, which runs the upstream scaffolder (`pnpm create` or `composer create-project`) before registering.

## How it coordinates with the app {#coordination}

`portbay-mcp` is a *client* of the same system the GUI and CLI use. It never runs its own copy of the engine.

- **Registry changes** (add / update / remove / import / export) write to PortBay's registry file and take effect **even if the app is not running**. The app's reconcile loop picks them up on next boot and converges certs, Caddy routes, and `/etc/hosts`.
- **Lifecycle actions** (start / stop / restart / logs) talk to the running Process Compose daemon over HTTP. If the app is not running you get a `SIDECAR_DOWN` error telling you to open it. Pass `auto_launch: true` on `portbay_start` or `portbay_setup` to have the server open the PortBay app and wait for the daemon to come up — use this only when you are at your machine.

## Stack recipes

Recipes are named blueprints that compose a project's framework, language version, document root, and HTTPS in one step. The agent maps your intent to a recipe id; PortBay applies it deterministically — no server-side model involved.

Current catalog (browse live with `portbay_list_recipes` or the `portbay://recipes` resource):

| Recipe id | Stack | Notes |
| --- | --- | --- |
| `next` | Next.js | Node dev server, HTTPS |
| `vite` | Vite | Node dev server, HTTPS |
| `astro` | Astro | Node dev server, HTTPS |
| `node` | Generic Node | HTTPS |
| `static` | Static HTML/CSS/JS | Caddy-served, HTTPS, no dev server |
| `php` | Plain PHP | Caddy + PHP-FPM 8.3, HTTPS |
| `laravel` | Laravel | PHP-FPM from `public/`, HTTPS; recommends MySQL 8.0 + Mailpit |
| `symfony` | Symfony | PHP-FPM from `public/`, HTTPS; recommends MySQL 8.0 + Mailpit |
| `statamic` | Statamic | PHP-FPM from `public/`, HTTPS; no database required |

Recipes with `composes_fully: false` (Laravel, Symfony) also recommend a database or mail service. The project is still registered with everything PortBay can wire today; the recommended service surfaces as a warning to add from the app.

## Drive the task board {#tasks}

When a project has a [task board](/guides/task-board), an agent can work it directly over MCP. The board is the shared queue between you and the human: cards are Markdown in the repo (`.portbay/tasks/`), and the GUI, the `portbay` CLI, and these tools all read and write the same files — there is no separate database to drift out of sync.

A dispatched run follows a simple loop:

1. **Pick up context.** `portbay_handoff_get` reads the continuation brief; `portbay_task_next` returns the top `Todo` card (or `portbay_task_get` re-reads the one you were dispatched to).
2. **Claim it.** `portbay_task_ack` with the `run_id` from your prompt proves you engaged — distinct from the process merely launching — and refreshes the run's lease.
3. **Work and report.** Post progress with `portbay_task_update` (status + `touchpoints`), tick checklist items with `portbay_task_check`, and record decisions with `portbay_task_comment`.
4. **Hand off.** `portbay_handoff_update` appends a minimal "where we left off" entry, then move the card to `Done` — or `Review` if the board requires human approval — with `portbay_task_update`.

The board enforces its rules even when an agent gets them wrong: an agent can't set `Rejected` (human-only), a card blocked on others won't dispatch until they land, and a run whose process dies is reclaimed so the card returns to the queue. The full tool list is in the [Tasks toolset](./tools#tasks-toolset); the human-facing setup is in the [Task Board guide](/guides/task-board).

Three resources expose board state for the agent to read without tool calls: `portbay://projects/{id}/context` (the derived environment), `portbay://projects/{id}/tasks` (the whole board), and `portbay://projects/{id}/handoff` (the rolling brief).

## Governance: read-only and toolsets {#governance}

Two flags scope what an agent can do. Both have flag and environment-variable forms; the env var wins over the flag (matching the GitHub MCP server convention). Set them in the `args` / `env` block of your agent config.

### Read-only mode

Removes every mutating tool (add / update / remove, start / stop / restart, import / export, scaffolding, group mutations, runtime mutations, database mutations, DNS suffix change, cert reissue, sandbox enable/disable, remote SSH execute, task-board writes, request log clear). The agent can inspect but never change anything.

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

Env var equivalent: `PORTBAY_MCP_READ_ONLY=1`.

In read-only mode the server appends a note to its system instructions telling the agent that mutations are disabled.

### Toolsets

Expose only the tool groups you want. Comma-separated list; valid values are `projects`, `lifecycle`, `diagnostics`, `scaffold`, `groups`, `tunnels`, `runtimes`, `databases`, `dns`, `sandbox`, `inspector`, `certs`, `migrate`, and `all` (the default, which covers all of those). Two special cases: `tasks` and `connectors` exist only in the Pro build, and `ssh-exec` is **never** in `all` — name it explicitly to let an agent run remote commands (see the [SSH Exec toolset](./tools#ssh-exec-toolset)).

| Toolset | Tools included |
| --- | --- |
| `projects` | list\_projects, status, detect\_project, detect\_workspace\_apps, list\_recipes, add\_project, update\_project, remove\_project, export\_config, import\_config, setup, setup\_from\_recipe |
| `lifecycle` | start, stop, restart, stop\_all |
| `diagnostics` | logs, doctor, sidecar\_status |
| `scaffold` | setup\_from\_template (runs upstream scaffolders; requires network) |
| `groups` | list\_groups, create\_group, update\_group, remove\_group, start\_group, stop\_group, restart\_group |
| `tunnels` | list\_tunnels, tunnel\_status, list\_ssh\_tunnels, ssh\_tunnel\_status, list\_ssh\_connections (read-only; manage tunnels and hosts from the app) |
| `runtimes` | list\_runtimes, set\_default\_runtime, add\_runtime\_path, remove\_runtime\_path |
| `databases` | list\_database\_engines, list\_databases, database\_connection, db\_schema, db\_query, db\_explain, db\_execute, create\_database, remove\_database, start\_database, stop\_database, restart\_database, link\_database, unlink\_database, set\_database\_auto\_start |
| `dns` | dns\_status, list\_dns\_records, set\_domain\_suffix |
| `sandbox` | sandbox\_status, sandbox\_violations, enable\_sandbox, disable\_sandbox |
| `inspector` | recent\_requests, clear\_requests |
| `certs` | cert\_info, reissue\_cert |
| `migrate` | detect\_import\_sources, preview\_import, import\_projects |
| `ssh-exec` | ssh\_execute — run one command on a saved SSH host. **Off by default**, never in `all`; enable explicitly. |
| `tasks` _(Pro)_ | tasks\_list, task\_next, task\_get, task\_create, task\_ack, task\_update, task\_check, task\_checklist\_add, task\_comment, task\_complete, handoff\_get, handoff\_update, learning\_add, connectors\_status (the per-project board) |
| `connectors` _(Pro)_ | connector\_accounts, connector\_search, connector\_get, connector\_create, connector\_update, connector\_comment (external task sources; writes need human approval) |

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

Env var equivalent: `PORTBAY_MCP_TOOLSETS=projects,diagnostics`.

Read-only and toolsets compose: `--read-only --toolsets projects,diagnostics` exposes only the read tools within those two groups. Filtered tools do not appear in `tools/list` and cannot be called.

## Error handling

Tool failures return as MCP tool-execution errors (`isError: true`) carrying PortBay's standard error envelope. The agent reads the envelope and can recover or explain what to do next.

```json
{
  "code": "SIDECAR_DOWN",
  "whatHappened": "process-compose is not running",
  "whyItMatters": "Projects can't start until process-compose is running again.",
  "whoCausedIt": "system",
  "actions": [{ "label": "Restart process-compose", "command": "sidecars.restart_pc" }]
}
```

Common error codes:

| Code | Meaning |
| --- | --- |
| `PROJECT_NOT_FOUND` | No project with that id in the registry. |
| `SIDECAR_DOWN` | The Process Compose daemon is not reachable — open the PortBay app. |
| `PORT_CONFLICT` | The configured port is in use by another process. |
| `PROJECT_CAP_REACHED` | The project limit for the current tier was reached. Sign in or upgrade. |
| `BAD_INPUT` | An argument was invalid or a required path was missing. |

Project caps apply to agent-driven adds exactly as they do in the GUI (anonymous: 3 / free: 6 / Pro: unlimited).

## All flags

| Flag | Env var | Default | Purpose |
| --- | --- | --- | --- |
| `--read-only` | `PORTBAY_MCP_READ_ONLY` | off | Inspection tools only; all mutations removed. |
| `--toolsets <list>` | `PORTBAY_MCP_TOOLSETS` | `all` | Comma-separated tool groups to expose. |
| `--pc-port <port>` | `PORTBAY_PC_PORT` | `9999` | Process Compose daemon port. |
| `--registry <path>` | — | app data dir | Override the registry file location. |
| `--log-level <level>` | `RUST_LOG` | `info` | stderr log verbosity (`error` / `warn` / `info` / `debug` / `trace`). |

All diagnostic output goes to **stderr**. Stdout carries only the MCP JSON-RPC stream.

See the [Tool Reference](./tools) for the full tool and resource inventory.
