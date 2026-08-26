---
title: Overview — the agent activity dashboard
description: "PortBay's Overview dashboard: what your agents have been doing across every project — tasks completed, requests handled, agent runs, the models they used, and recent run history."
---

# Overview

**Overview** sits at the top of the sidebar, above Projects. It is the calm,
at-a-glance answer to "what have my agents been doing?", across every project
rather than one at a time.

<ThemeImage name="overview" alt="The PortBay Overview dashboard — agent KPIs, an activity chart, and recent runs" />

## What it shows

**KPI tiles** across the top, each with a delta against the previous period:

- **Tasks Completed** — cards that reached Done, from dispatched runs and from
  board moves alike.
- **Requests Handled** — tool calls made by agents, including the in-app voice
  companion.
- **Agent Runs** — dispatched runs, voice sessions, and audit-only runs.

**Activity chart** — the trend over the selected range.

**Most Used Models** — a leaderboard of the models your agents ran on.
Voice sessions carry no model, so they never appear here; when there is no run
history yet it falls back to the models currently loaded in Ollama.

**Recent Runs** — the newest runs across all projects, with the project, the
card, the model, and the outcome. Click through to the run.

**Agent Activity** — a breakdown of the activity feed by kind.

Use the range picker in the header to move between periods.

## Where the numbers come from

Three lanes are unified on this page, which is why the totals are higher than
any single surface would report:

| Lane | Feeds |
|---|---|
| **Mission Control runs** (cross-project, from the task board) | Tasks Completed, the chart, the runs table, the model leaderboard |
| **Voice-agent audit ledger** | Requests Handled (tool calls), and voice sessions folded into Agent Runs |
| **Board audit trail** — every actor: your own drags, the CLI, MCP sessions, agent run summaries | Done moves into Tasks Completed, audit-only runs and LLM sessions into Agent Runs, run-summary tool calls into Requests Handled |

The board audit trail is de-duplicated against workflow run states on the
backend, so a run that appears in two lanes is counted once.

Every tile has an empty state: a lane that has never been used says so rather
than showing a zero that looks like a failure.

## See also

- [Task Board & Agents](/guides/task-board)
- [Workflows](/guides/workflows)
- [Voice Companion](/guides/voice-companion)
