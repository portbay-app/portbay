---
name: portbay-board
description: >-
  Work a PortBay project's task board over MCP. Use when a PortBay project is
  open and you've been dispatched a card, or whenever you should read/update the
  project's tasks or session hand-off. Triggers on "portbay", a dispatched run
  id, or a `.portbay/` directory in the repo.
---

# PortBay board protocol

PortBay is the **work-state authority** for this project. The board (not your
memory) is the source of truth. PortBay never runs you autonomously — you run
under your own tool's approval prompts.

## On starting / picking up a card (resume-aware)

1. **Read the continuation brief first** — `portbay_handoff_get` (or the
   "Continuation" block in `AGENTS.md`). Trust it over your own memory; don't
   redo items it lists as done.
2. Read the live environment — `portbay://projects/{id}/context` (URL, ports,
   runtime, web server, DB env-var references, services).
3. **Acknowledge** the card — `portbay_task_ack(id, run_id)` using the run id
   from your dispatch prompt.

## During work

- Work **only** the assigned card; stay in scope.
- Break the work into your own tracking points with
  `portbay_task_checklist_add(id, items=["P0: …", "P1: …"])`, then tick each as
  you finish — `portbay_task_check(id, idx, done=true)` — so the human sees real
  sub-progress.
- Post short progress notes on meaningful steps —
  `portbay_task_update(id, run_id, note)` (also a heartbeat). Record decisions or
  ask the human with `portbay_task_comment(id, text)` (shows in the card thread).
- Stuck / need a human? `portbay_task_update(id, run_id, status="Blocked", reason)`.
- Discover new work? `portbay_task_create(..., labels=[…])` into Backlog — don't
  bury it in chat.

## On finishing (order matters)

1. **Update the hand-off first** — `portbay_handoff_update(narrative)` with a
   MINIMAL brief: what changed · the next concrete step · open items · pointers.
2. **Then** mark the card — `portbay_task_update(id, run_id, status="Done")`
   (or `Review` if review is required). PortBay runs any acceptance check on
   Done; a failure sends the card to `Blocked`.

## Hard rules

- You may **never** set a card to `Rejected` — that's a human-only decision;
  the server will refuse it.
- Only your current `run_id` may advance a card; a stale session's updates are
  rejected.
- Secrets are referenced by env-var name only — never inline a credential value
  into a card, the hand-off, or a context file.
