---
title: "PortBay for Running Parallel AI Agents"
description: Run parallel AI coding agents on macOS with a repo-versioned task board and handoff notes that persist across Claude Code, Cursor, and Codex.
---

# Run parallel AI coding agents with a board that outlives the session

Cursor's parallel agents and Claude Code's Agent Teams coordinate several agents inside one tool and one session. PortBay adds the layer they leave out: a task board whose cards are Markdown files in your repo, shared across the PortBay app, the `portbay` CLI, and an MCP server. Move a card to *To Do* and the agent you assigned works it on your machine. When the run ends it appends to `.portbay/HANDOFF.md`, a short newest-first brief the next run reads first, whether that next run is the same agent, a different tool, or a teammate tomorrow. Because the cards and the handoff note version with your git history, the work survives a closed session, a switched tool, and a second machine. You keep using Cursor or Claude Code for the orchestration you like; PortBay holds the durable record of what each agent did and what comes next.

## In-tool orchestration vs a durable board

| | In-tool orchestration (Cursor, Claude Code) | PortBay task board |
|---|---|---|
| Scope | One tool's own agents | Any agent: Claude Code, Cursor, Codex, Gemini, Aider |
| Where tasks live | The tool's session and UI | Markdown cards in your repo (`.portbay/tasks/`) |
| When a session ends | Context resets | Handoff note persists in the repo |
| Across machines | Tied to the tool | Versions with your git repo |
| Interface | The tool's UI | GUI, CLI, and MCP, one shared source of truth |

## What Cursor and Claude Code already do well

Cursor shipped a rebuilt interface for orchestrating parallel agents in April 2026, and Claude Code shipped Agent Teams where several agents coordinate through shared task lists over MCP. Both are good at fanning out work inside their own world, and if one tool covers your whole workflow you may not need more. Run them. PortBay does not replace them.

## What PortBay adds on top

PortBay makes the work portable. The board is not locked in a vendor's session; it is plain Markdown in your repository, so a card written for Claude Code can be picked up by Cursor next week, and the handoff note travels with the code through every clone. A card can be blocked on others until they land, an optional *Review* column holds agent-done work for a human, and a run whose process dies is reclaimed for the next attempt. One board is read and written by the GUI, the CLI, and any MCP-connected agent, so nothing has to be re-derived when you change tools.

## How they work together

Keep orchestrating inside Cursor or Claude Code. Point each at PortBay's [MCP server](/agents/) so the agent can claim the next card, record the files it touched, and move the card to *Review* or *Done*. The tool runs the agents; PortBay keeps the score, in your repo, across every tool and machine.

## Start in a few minutes

Install PortBay on macOS via [DMG or Homebrew](/getting-started/install), connect your agent in [60 seconds](/agents/#add-portbay-to-your-agent-in-60-seconds), and write your first card. PortBay is open source under AGPL-3.0; the task board and agent dispatch are Pro features, free for anyone who merges a pull request. More on the workflow in the [task board guide](/guides/task-board) and [running agents locally](/guides/run-ai-coding-agents-locally).

_Last updated: 2026-06-15._
