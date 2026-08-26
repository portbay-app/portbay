---
title: Visual Editor — Auto-Verify Loop, History & MCP
description: How PortBay's visual editor checks a dispatched edit card's changes against the live page after the agent finishes, persists the run, and lets you re-run or read it back over MCP.
---

# Auto-Verify Loop

When the visual editor dispatches a card of style/text edits to a coding agent (Tier-2 — the edits a page's `data-pb-loc` can't resolve deterministically enough for an in-place source patch), PortBay doesn't just trust the agent's "Done" status. Once the card reaches **Done**, the workbench:

1. Reloads the live preview (without replaying the workbench's own prototype overlay).
2. Asks the in-page agent to check each requested edit's computed value against the reloaded page (`verifyEdits`).
3. Counts any new console errors that appeared since the reload.
4. Appends a `## Preview verification` section — PASS/FAIL, per-property expected vs. actual, the console-error delta — back onto the card file.
5. Drops the edits that verified from the workbench's pending ledger (only the ones that didn't land stay pending).

This is what the **Verify report** modal (opened from the "View report" link in the footer while a card is watched) shows: a PASS/FAIL header, a per-property table, and — when a screenshot backend is available (macOS) — before/after thumbnails of each edited element.

## Persisted history

Every completed run — a dispatched card reaching Done, or a manual re-run (below) — is written to `.portbay/verifications/<id>/` in the project: the requested-edit spec, the per-property results, the console-error delta, and copies of the before/after element screenshots. Like [visual edit history](/guides/visual-editor), this is plain JSON on disk (`manifest.json` + a `shots/` subdir), not a hidden database — it version-controls with the project if you choose to commit `.portbay/`, and survives a PortBay restart.

The workbench's toolbar has a **Verifications** button (shield icon, next to History) that opens the run list: pass/fail, edits checked, new console errors, and which card drove the run, newest first. Runs are kept for 30 days (newest 30 kept), the same prune discipline as visual edit history.

Clicking a run opens the same **Verify report** view used for a live watch, over the stored data.

## Re-running a stored verification

A stored run's requested-edit spec (the selectors/properties/values it checked) is enough to re-check it against whatever the page looks like *right now* — no card, no dispatch, no reload. Use **Re-run** on a row in the Verifications panel, or the **Re-run** button in the detail view. The result is persisted as a new run referencing the original (so the history stays append-only), with `kind: "rerun"` distinguishing it from a card-driven auto-verify. A re-run has no "before" reload to diff console errors against, so its new-error count is always 0.

## Reading run summaries over MCP

The `portbay_verify_runs` MCP tool returns read-only run summaries for a project — id, timestamp, pass/fail, edits checked, new console errors, and the card path, without the per-property rows, spec, or screenshot paths (open the app's Verifications panel for the full detail). It's part of the opt-in `verify` toolset, alongside `browser`:

```bash
portbay-mcp --toolsets verify
# or combined with the browser companion
portbay-mcp --toolsets tasks,verify
```

Like the browser companion tools, `portbay_verify_runs` only exists on a Pro build compiled with both the `tasks` and `visual-editor` features — it needs the task board (a card is what triggers a run) and the visual editor (the verify loop itself).
