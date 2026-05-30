---
title: Task Board & AI Agents
description: PortBay's per-project task board hands cards to the coding agent you assigned — Claude Code, Codex, Cursor, Gemini, Aider, and more — which work them in your repo and leave a hand-off note for the next run.
---

# Task Board & AI Agents

Every project in PortBay gets a board. It looks like any Kanban board — columns of cards you drag between states — with one difference: **move a card to _To Do_ and the AI coding agent you assigned picks it up and starts working.** The agent runs in your project, on your machine; when it finishes it writes a hand-off note so the next run (agent or human) can continue without re-deriving context.

<ThemeImage name="tasks" alt="PortBay's per-project task board with AI agents working cards" />

## Cards are Markdown in your repo

A board isn't a database hidden inside the app. Each card is a Markdown file under `.portbay/tasks/` in the project, and the rolling hand-off log is `.portbay/HANDOFF.md`. That means:

- Cards **version with your code** — they show up in `git diff`, travel in branches, and survive without PortBay.
- The board is editable three ways that never drift apart: the **GUI**, the **`portbay` CLI**, and the **MCP server** all read and write the same files.
- You can read a card in any editor. PortBay just gives it a board, a runner, and the agent plumbing.

## The columns

| Column | Meaning |
| --- | --- |
| **Backlog** | Captured, not yet ready to start. |
| **To Do** | Ready to work. Moving a card here is what dispatches its agent. |
| **In Progress** | An agent (or you) is actively working the card. |
| **Blocked** | Waiting on another card or an external answer. Optional column, shown on demand. |
| **Review** | Agent-reported "done", held for a human to approve. Optional column. |
| **Done** | Finished and accepted. |
| **Rejected** | Won't do. **Human-only** — an agent can never move a card here. |

## Assign an agent

PortBay does **not** ship a model of its own. It launches the coding agent you already have installed and points it at the card. Recognised out of the box:

**Claude Code · Codex · Cursor · Gemini · Aider · Copilot · OpenCode · Amp · Qwen · Antigravity** — and a **Custom** option that runs any other CLI from a command template.

Most agents launch as a **CLI** subprocess; a few (e.g. Cursor) open as a **desktop app** on the project folder. PortBay auto-detects which agents are installed and lets you set the binary path manually for anything it can't find. You can:

- **Assign an agent per card** — different cards can go to different agents.
- **Set a board default** — every card without its own agent uses it.

Set both in the board's settings and in **Settings → Integrations**, which also detects your installed agents and writes the MCP config for each client (see [Drive PortBay from an AI Agent](/agents/)).

## Auto-dispatch vs. confirm

A board runs in one of two modes:

- **Auto-dispatch on _To Do_** — the moment a card lands in To Do, its agent is launched. Good for boards you want to run themselves.
- **Manual** — moving a card to To Do queues it, and PortBay asks you to confirm before the agent starts. Good when you want a human in the loop on every run.

A board can also cap how many agents run at once (concurrency), so a busy board doesn't launch ten agents simultaneously.

## The hand-off note

When a run ends, the agent appends a short entry to `.portbay/HANDOFF.md`:

- **Newest first** — each entry is prepended with a `## <timestamp> · <author>` heading.
- **Size-capped** — the log is bounded; once it hits the cap, the oldest entries are pruned, so the brief stays short.
- **Attributed** — every entry is signed by the agent, the CLI, you, or PortBay.
- **Pointers, not payloads** — it records what changed, the next concrete step, and open items; it points at cards and files rather than pasting them.

The next run reads the hand-off **first**, so work continues from "where we left off" instead of from a cold start. This is what lets one agent pick up where another (or you) stopped.

## It stays out of trouble

The board is designed to fail safe when an agent gets something wrong:

- **Dependencies.** A card can be blocked on other cards; it won't dispatch until they reach a terminal column.
- **Review gate.** Turn on _Require review_ and agent-reported "done" lands in **Review** for a human to approve — it never jumps straight to Done.
- **Crash recovery.** A running card holds a lease with a heartbeat. If the agent's process dies, the lease expires and the card is reclaimed back to the queue instead of getting stuck "in progress".
- **Rejected is human-only.** Agents can advance cards to In Progress, Blocked, Review, or Done — but only a person can reject one.
- **Optional auto-branch.** A card can create or switch to its own git branch on dispatch, so parallel agents don't fight over the working tree.

## Activity and notifications

Agent actions surface as activity on each card — progress notes, comments, blocked reasons, and warnings — and PortBay's notification bell shows unread items so you can jump straight to the card that needs you.

## Driving it from an agent (MCP)

The board is also the coordination channel for agents connected over MCP. A dispatched run reads the hand-off and the next card, acknowledges its run id, posts progress and touched files, then updates the hand-off and moves the card on. The full loop and the eleven board tools are documented in the [Tasks toolset](/agents/tools#tasks-toolset), and the workflow walkthrough is in [Drive PortBay from an AI Agent](/agents/#tasks).

Three MCP resources expose board state read-only: `portbay://project/{id}/context`, `portbay://project/{id}/tasks`, and `portbay://project/{id}/handoff`.
