# PortBay performance — interaction latency

> Scope: this document owns the **interaction layer** — how fast the app
> *feels* when you click Play, Stop, or open the command palette, and how
> quickly the UI reflects real backend state. Cold-start time, installer size,
> and idle RAM are a separate concern owned by the *Performance + memory budget
> regression CI* card and are **not** covered here.

Speed is PortBay's headline property. A small native app can still feel slow if
a click waits on an IPC round-trip or the status lags reality. The discipline
here is the one Bun made famous: **speed is the feature, and it's measured.**

---

## Interaction-latency budgets

| Interaction | Budget | How it's met |
|---|---|---|
| Play clicked → row shows **Starting…** | < 100 ms | Optimistic overlay, flipped synchronously on click (see below) |
| Stop clicked → row shows **Stopping…** | < 100 ms | Same optimistic overlay |
| Command palette open | < 50 ms | Client-only render; no IPC on open |
| Status reflects real PC/Caddy state | < 1 s after change | 750 ms status poller → `portbay://status` event |
| Add-project detect → wizard L1 populated | < 300 ms | Filesystem probe; measure via the dev instrument |

The first two are the load-bearing ones — they're what a user does dozens of
times a day. They are now met **by construction**, not by hoping the backend is
fast: see "Optimistic lifecycle" below.

---

## Optimistic lifecycle (the main win)

Before this work, clicking Play spun a button while the lifecycle command did
its slow work — preflight port check, `client.start()`, and a full reconcile
tick (write Process-Compose YAML, reload Caddy, update `/etc/hosts`) — and the
status cell only changed when the next 750 ms poll emitted an event. Perceived
latency was therefore tied to backend latency.

Now the UI **leads** the backend:

1. On click, the row flips to an optimistic display state (`starting` /
   `stopping`) **synchronously, before any `await`** — the store write happens
   in the same event-handler turn as the click, so the row repaints on the next
   frame (~16 ms), well inside the 100 ms budget.
2. The real `portbay://status` event reconciles the row to true state when it
   arrives. A stale, wrong-direction poll reading (e.g. a `stopped` tick that
   fires after a Play but before the process boots) is **ignored** so the row
   doesn't flicker backwards.
3. If the action fails — or the user declines a port-conflict force-quit — the
   overlay is rolled back and the error envelope is shown. A 12 s TTL is a
   final safety net so a row can never get wedged mid-transition.

The logic is a pure, framework-free state machine in
[`src/lib/lifecycle/optimistic.ts`](../src/lib/lifecycle/optimistic.ts), wrapped
in `$state` by the projects store. It applies to every Play/Stop surface: the
table rows, the card grid, the detail panel, the keyboard shortcuts, and Stop
All (which flips every running project to `stopping` at once).

### Backend latency is unchanged — on purpose

This card deliberately did **not** alter the lifecycle commands' return timing.
The reconcile tick on start and the 750 ms reap delay on stop exist for
correctness (route readiness; reaping orphaned dev-server workers — see the
lifecycle epic). Making the commands return before that work risks the
regressions that epic fixed. The optimistic overlay decouples *perceived*
latency from backend latency without touching that ordering, which is the
lower-risk way to hit the budget. If the backend hot path is later trimmed, it
only improves how fast the overlay resolves to real state.

---

## Measuring the baseline

There is a **dev-only** IPC timing instrument
([`src/lib/perf.ts`](../src/lib/perf.ts)) wired into the canonical invoke
wrappers. It compiles away in release builds. To read real numbers from a live
`pnpm tauri dev` session, open the webview dev console:

```js
__portbayPerf.table()    // console.table of p50 / p95 / max per command
__portbayPerf.summary()  // same data as an object
__portbayPerf.samples    // raw ring buffer (last 300 calls)
__portbayPerf.clear()
```

### Analytical baseline (from code paths — confirm in-app)

These figures are derived from reading the lifecycle command paths
(`src-tauri/src/commands/lifecycle.rs`, `reconciler/`), **not** yet measured in
a running build. Capture the real numbers with the instrument above and replace
this section.

| Command (IPC round-trip) | Estimated blocking time | Dominated by |
|---|---|---|
| `start_project` | ~200–700 ms | `client.start()` + full reconcile tick (Caddy reload, hosts write) |
| `stop_project` | ~810 ms | `client.stop()` + 750 ms orphan-reap delay |
| `restart_project` | ~860–1350 ms | restart + 750 ms delay + preflight + reconcile |
| `stop_all` | ~800 ms + ~60 ms/project | per-project stop + single 750 ms reap delay |

The point of the optimistic overlay: the user feels none of this on the Play /
Stop click — only when the row resolves to its true final state, which the
750 ms poller surfaces within the 1 s status-freshness budget.

---

## Guarding the budget in CI

The interaction budget rests on the optimistic flip being **synchronous** and
on stale events not clearing a fresh overlay. Both are pinned by unit tests in
[`tests/optimistic.test.ts`](../tests/optimistic.test.ts), run by `pnpm test`
(Vitest) in the `frontend` CI job. Because the transition core is a pure
function, "issuing an action yields its display with no async step" is a
structural property the tests lock in.

The **end-to-end** assertion — drive a real build with WebDriver, click Play,
assert the row paints `Starting…` within the budget — depends on the shared
WebDriver harness owned by the *Performance + memory budget regression CI*
card. That harness does not exist yet; when it lands, add the click-to-paint
timing assertion there rather than duplicating it.

---

## No-op spinner audit

Every loading affordance must map to real in-flight work and resolve to a real
terminal state — no spinner may hide a no-op. Audited 2026-05-24:

| Spinner | File | Maps to |
|---|---|---|
| Row Play/Stop spinner | `ProjectRow.svelte` | the in-flight `start_project` / `stop_project` IPC (`busy` state) |
| Card Play/Stop spinner | `ProjectCard.svelte` | same |
| Stop All "Stopping…" | `StopAllButton.svelte` | the in-flight `stop_all` IPC |

No lying spinners found. The optimistic overlay is not a spinner — it's a real,
reconciled status that rolls back on failure.
