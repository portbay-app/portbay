---
title: Workflows — visual automation builder
description: Build, run, and schedule node-based workflows in PortBay. Agents, browsers, databases, connectors and MCP tools on one canvas, with triggers, run history, and an App mode for people who never open the canvas.
---

# Workflows

**Workflows** is PortBay's node-based automation builder. You wire a graph of
steps on a canvas — call an agent, query a database, drive a browser, post to
Slack, run an MCP tool — then run it by hand, on a schedule, from a webhook, or
when a task-board card moves.

Open it from **Workflows** in the sidebar. It is project-scoped: pick a project
in the top-left switcher and you are editing that project's workflows. The
switcher remembers where you left off.

<ThemeImage name="workflows" alt="The PortBay workflow canvas — a card trigger feeding an agent step and a decision branch" />

::: tip The same builder appears in two places
`/workflows` is the project-wide entry point. The task board's **Recipes** view
embeds the identical builder scoped to a single card. Anything below works in
both; the handful of project-wide-only options are called out where they occur.
:::

## Your first workflow

1. **Workflows → pick a project → New workflow.** Start from the template
   gallery, describe what you want in the AI start modal, or open an empty
   canvas.
2. **Add nodes.** Drag from the palette, double-click empty canvas to search, or
   press <kbd>⌘</kbd><kbd>K</kbd> for the command palette.
3. **Connect them.** Drag from a node's output handle to another node's input.
   Dropping a connection *on* a node connects it to the first compatible port.
4. **Configure each node** in the inspector on the right.
5. **Run it** with the Run button. Inputs the graph declares are collected in a
   modal first.

A workflow is saved per project, versioned on every save, and can be restored
from **Version history** at any point.

## Nodes

Every node has a **capability** — what kind of work it does. The capability
decides its colour, its configuration form, and which ports it exposes.

| Capability | What it does |
|---|---|
| **Trigger** | Entry point. What starts this workflow (see [Triggers](#triggers)). |
| **Agent** | Dispatches a coding agent (Claude Code, Codex, Gemini, a local Ollama model…) with a prompt. |
| **Extract** | Asks a model for a structured value without the dispatch apparatus — the same question as Agent, minus the agent. |
| **Race agents** | Runs several agents on the same input and takes the first (or best) to finish. |
| **Decision** | Branches on a condition. |
| **Loop** | Repeats a branch over a list. |
| **Merge** | Joins branches back together. |
| **Guardrail** | Blocks the run unless a condition holds. |
| **Review** | Pauses for a human to approve before continuing. |
| **Browser** | Drives a real browser — navigate, fill a form, click, extract, screenshot. |
| **Database** | Runs a query against one of your PortBay databases. |
| **Service** | Starts, stops, or checks a bundled service. |
| **SSH** | Runs a command on a saved remote host. |
| **Deploy** | Publishes a build. |
| **Connector** | Calls a connected external account (Slack, Gmail, GitHub, Linear, …). |
| **MCP tool** | Calls any tool on a connected MCP server. |
| **Tool** | Calls one of PortBay's own tools. |
| **Web search** | Searches the web through the local search service. |
| **Capture** | Takes a screen capture. |
| **Image** | Generates an image locally. |
| **Speech to text** | Transcribes audio. |
| **Notify** | Sends a notification. |
| **Secret** | Reads a secret into the run without printing it. |
| **Card source** / **Select task** / **Create task** | Read from and write to the task board. |
| **Subworkflow** | Runs another saved workflow as one step. |
| **Output** | Declares what the workflow returns. |

### Typed ports

Sockets and wires are colour-coded by capability, so an incompatible connection
is visible before you make it. A node's **output contract** — the shape of what
it produces — is shown in the inspector, and the **reference picker** on any
config field lets you insert a reference to an upstream node's output rather
than typing a path by hand. The picker only offers nodes that actually run
upstream of the one you are editing.

### Node options

Right-click a node, or use the node toolbar:

- **Disable** — the node is skipped and its branch does not run.
- **Bypass** — the node is skipped but input passes straight through to its
  output, so downstream nodes still run.
- **Pin** — keeps the node's last result while you iterate on the rest.
- **Collapse** — folds the node to its header.
- **Colour** — set a per-node accent for visual organisation.
- **Cache result** — opt-in, off by default. Re-uses the last result instead of
  re-running the step.

## The canvas

| | |
|---|---|
| <kbd>⌘</kbd><kbd>K</kbd> | Command palette |
| <kbd>⌘</kbd><kbd>Z</kbd> / <kbd>⌘</kbd><kbd>⇧</kbd><kbd>Z</kbd> | Undo / redo |
| <kbd>⌘</kbd><kbd>C</kbd> / <kbd>⌘</kbd><kbd>V</kbd> / <kbd>⌘</kbd><kbd>D</kbd> | Copy / paste / duplicate |
| <kbd>⌫</kbd> | Delete selection |
| Arrows / <kbd>⇧</kbd>+arrows | Nudge 1 px / 10 px |
| <kbd>1</kbd> | Zoom to fit |
| <kbd>F</kbd> | Fit to selection |
| <kbd>Space</kbd> (hold) | Pan |
| <kbd>⇧</kbd>+drag | Marquee-select |
| <kbd>⌥</kbd>+drag | Drag a duplicate off a node |
| Double-click canvas | Node search |
| Double-click title | Rename the workflow |
| Right-click | Node / canvas context menu |
| <kbd>?</kbd> | The cheat sheet, in the app |

Other canvas affordances:

- **Groups** — draw a background frame around nodes. A group is a real
  container: moving it moves its members, and a group can be muted as a unit.
- **Sticky notes** — annotate the graph for whoever opens it next.
- **Tidy** — auto-layout. The button toggles between vertical and horizontal.
- **Alignment guides** appear while dragging.
- **Level of detail** — nodes simplify as you zoom out, so a large graph stays
  readable.
- **Collapse selection to subworkflow** — turn a working cluster into a single
  reusable node.

## Triggers

A workflow can start six ways. Manage them in the **Triggers** panel.

| Trigger | Fires when |
|---|---|
| **Manual** | You press Run. Never fires on its own. |
| **Card** | A task-board card enters a status you choose. |
| **Schedule** | On a cadence, from the background scheduler. |
| **Webhook** | An authenticated inbound delivery arrives at the workflow's endpoint. |
| **Chat** | A message arrives in an opted-in chat target. |
| **Mail** | A message arrives in a connector mailbox you name (Gmail / Outlook). |

::: warning Chat and Mail triggers are consent-gated
Saving a **Chat** trigger is not by itself consent — the channel, workspace and
target must already be admitted by the connector's inbound opt-in. For a
**Mail** trigger, choosing the mailbox *is* the consent.
:::

Each trigger has its own enable switch, so you can leave one configured but off.

## Running a workflow

Press **Run**. If the graph declares inputs, you are asked for them first.

While it runs, the canvas *is* the progress view: each node shows its live
state, and finished nodes show an **inline output preview** so you can read what
a step produced without leaving the canvas.

### Run history

The **Runs** panel lists past runs, newest first. Open one and you get:

- the **trace** — every step, in order, with timing and status;
- each step's **log**;
- the **graph as it was when that run started**, embedded in the run record —
  so a run from three edits ago still reads correctly against the graph that
  produced it.

Runs are searchable: which run touched this file, which run hit this error.

### Retrying and resolving

A failed step can be retried on its own without re-running the whole graph. A
**Review** node that is waiting shows up as a pending decision you resolve from
the run.

## App mode

Not everyone who runs a workflow wants to see a canvas. **App mode** presents a
saved workflow as a plain form: its title, its description, its declared inputs
as labelled fields, and a **Run workflow** button. The node graph is hidden
entirely.

Switch into it from the workflow's toolbar; the exit control brings the canvas
back. This is the mode to hand to a teammate who just needs to press the button.

A workflow can also expose itself **as a tool** — see
[Driving workflows from an agent](#driving-workflows-from-an-agent).

## Code view

The canvas has a text form. Open the **Code** panel and you get PortBay's
workflow DSL, live and editable, docked beside the graph:

- edit the graph and the text refreshes (unless you are typing in it);
- edit the text and, ~400 ms after you stop, valid DSL is merged back into the
  graph with node positions preserved.

Invalid DSL shows its errors and **never** clobbers the graph, so you can type
freely.

## Edit with AI

**Edit with AI** on the canvas toolbar opens a panel that edits the workflow
you have open, in place — describe the change and it rewrites the graph. It
never navigates away from the panel it edits in, so you can iterate. The model
it runs on is the one you pick in the panel.

To generate a workflow from nothing, use the AI option in the start modal
instead.

## Sharing and versions

- **Version history** — every save is a version. Preview an old one and restore
  it.
- **Share sheet** — export a workflow to hand to someone else.
- **Templates** — start from the built-in gallery.
- On Pro, workflow recipes and their triggers **[sync across your
  devices](/pro/#multi-device-sync)**.

## Driving workflows from an agent

Workflows are exposed over [MCP](/agents/), so Claude Code, Codex, Cursor or any
other MCP client can author and run them. The tools worth knowing:

| Tool | Purpose |
|---|---|
| `portbay_workflow_list` | List a project's workflows. |
| `portbay_workflow_generate` | Generate a graph from a description. |
| `portbay_workflow_author` | Author a workflow and self-correct until it validates. |
| `portbay_workflow_edit` | Edit an existing graph. |
| `portbay_workflow_validate_graph` | Check a graph without saving. |
| `portbay_workflow_dry_run` / `portbay_workflow_preflight` | Check a run before making it. |
| `portbay_workflow_run` / `_start` / `_stop` | Run control. |
| `portbay_workflow_run_list` / `_get` / `_step_log` | Read run history. |
| `portbay_workflow_step_retry` / `_step_resolve` | Retry a failed step; resolve a waiting one. |
| `portbay_workflow_trigger_save` / `_list` / `_set_enabled` / `_delete` | Manage triggers. |
| `portbay_workflow_tool_schema` | The workflow's own schema, for calling it as a tool. |

`portbay_workflow_author` is the one to reach for first: it writes the graph,
validates it, and fixes what it got wrong, rather than handing you an invalid
recipe.

## See also

- [Task Board & Agents](/guides/task-board) — the card-scoped side of the same builder
- [Parallel AI Agents](/guides/parallel-ai-agents)
- [Integrations](/guides/integrations) — connecting the accounts a Connector node calls
- [AI Agents (MCP)](/agents/)
