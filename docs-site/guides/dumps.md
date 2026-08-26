---
title: PortBay Dumps — Universal dump()/dd() Viewer
description: PortBay's opt-in local dumps listener for Symfony/Laravel dump()/dd() and any language's POST /dump, with a live ring-buffer viewer.
---

# Dumps

The **Dumps** page (`/dumps`) is a universal, live, time-ordered viewer for `dump()`/`dd()` calls from any project — the same snapshot-then-listen shape as the Log Viewer: an initial snapshot on load, then a live event stream for everything after. Each entry shows a project badge, the `file:line` it was called from, and an expandable value tree.

## Enabling It

Dumps is **opt-in and off by default** — enable it from [Integrations](/guides/integrations) → Apps tab. Once enabled, the listener starts immediately (no restart needed) and stays on across app restarts until you disable it again. You can still open `/dumps` while it's disabled — it just shows a disabled-state card instead of an empty state, so an old bookmark or nav history doesn't look like a bug.

Per-project, there's a separate `dumps` toggle: turning it on injects `VAR_DUMPER_SERVER` and `VAR_DUMPER_FORMAT=server` into that project's environment, so Symfony/Laravel's `dump()`/`dd()` reach PortBay's listener instead of rendering in-page or in a browser response.

## How It Works

The listener is **always-on while enabled** — an in-process pair of accept loops on `AppState` (not a Process Compose child, not a bundled sidecar binary — the same shape as PortBay's OpenAI-compatible server), so `dump()` never blocks waiting for a viewer: every frame is parsed and pushed into a bounded ring buffer (500 entries, oldest dropped first) whether or not the `/dumps` page is open.

- **VarDumper wire protocol** — the raw protocol Symfony/Laravel's `ServerDumper` speaks, on `127.0.0.1:9912` by default (scanning upward if taken).
- **`POST /dump` HTTP intake** — a minimal HTTP endpoint on `127.0.0.1:9913` by default, for any other language to push a dump manually. Request bodies are hard-capped at 1 MiB, enforced by bounding the actual read rather than trusting a `Content-Length` header.

**Loopback only.** Neither listener ever accepts a connection from outside `127.0.0.1` — this is a local debugging aid, not a network service.

When PortBay itself isn't running, the TCP connect from `ServerDumper` is simply refused and Symfony falls back to its own in-page dumper automatically — no PortBay-side change is needed for that fallback to work.

## Clearing

The **Clear** action wipes the ring buffer only — it doesn't affect anything already delivered to a listening client, and matches the Log Viewer's "Clear" semantics (view-only; there's no on-disk dump history to delete, since dumps are never persisted to a file).

## See Also

- [Services](/guides/services) — the dumps listener's live status (running, bound ports) is one of the sidecar cards there.
- [Integrations](/guides/integrations) — where the feature is enabled/disabled.
