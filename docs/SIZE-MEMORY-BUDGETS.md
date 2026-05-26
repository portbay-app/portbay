# Size & memory budgets

> Scope: this document owns the **footprint** budgets — DMG installer size,
> idle RAM, and cold-start time — and which of them CI enforces. Interaction
> *latency* (how fast Play/Stop feel) is a separate concern owned by
> [`PERFORMANCE.md`](./PERFORMANCE.md).

PortBay's positioning is *lightweight*: a slim installer and a small idle
footprint relative to Electron-based alternatives. These are the budgets we
commit to and the levers for clawing a number back if it regresses.

## Budgets

| Budget | Target | What it measures | Source |
|---|---|---|---|
| DMG installer size | **< 100 MB** | The compressed `.dmg` a user downloads | [`ARCHITECTURE.md`](./ARCHITECTURE.md) bundle-size budget |
| Idle RAM | **< 80 MB** | RSS with an empty registry, nothing running | `ARCHITECTURE.md` |
| Cold startup | **< 1500 ms** | Spawn → first paint of the projects table | Planning-phase budget |

The installer budget is the **compressed DMG**, not the uncompressed `.app`
(they differ ~2.3× — the bundle is dominated by prebuilt Go sidecars that
compress to ~⅓ their size). See the component breakdown in `ARCHITECTURE.md`.

## What CI enforces today

| Guard | Status | Where |
|---|---|---|
| **DMG installer < 100 MB** | **Live** | [`release.yml`](../.github/workflows/release.yml) → *Guard DMG size budget* step |
| Idle RAM < 80 MB | Blocked — see below | — |
| Cold startup < 1500 ms | Blocked — see below | — |

The DMG-size guard runs on every release build, where the signed `.dmg` is
genuinely produced. A regression (a stray dependency, an un-stripped binary, an
extra bundled sidecar) fails the release immediately rather than shipping and
drawing a "look how slim the competitor is" complaint.

### Why idle-RAM and startup guards are not yet wired

Both require launching the real Tauri app and observing it. The blocker is the
same constraint the E2E suite hit: **macOS has no WKWebView WebDriver**, so the
GUI can't be driven in CI, and a headless RSS/startup sample of the native shell
isn't reliable on the hosted macOS runners. Rather than ship CI jobs that flake
or measure the wrong thing, these are parked until a viable harness exists:

- **Cold startup** is resumable as a *proxy* via the Playwright SPA harness the
  E2E suite already uses (frontend time-to-projects-table against the web
  build) — a proxy for, not a measurement of, native cold start.
- **Idle RAM** needs a way to launch the packaged `.app`, settle, and sample
  RSS via `ps` without a driven GUI session.

Until then, both are measured manually at release time and recorded below.

## Baselines (measured)

| Metric | Measured | Date | Notes |
|---|---|---|---|
| DMG installer | **~77 MB** | 2026-05-25 | arm64, UDZO/zlib; ~23 MB of headroom under budget |
| `.app` (uncompressed) | ~175 MB | 2026-05-25 | arm64; see `ARCHITECTURE.md` component table |
| Idle RAM | _not yet measured_ | — | pending a headless sampling harness |
| Cold startup | _not yet measured_ | — | pending the Playwright proxy harness |

## Levers if a budget regresses

**DMG size** — in rough order of cheapness:

1. Confirm `[profile.release]` stripping is in effect (`strip`, `lto`,
   `codegen-units = 1`); an un-stripped shell binary alone is ~9 MB of bloat.
2. Lazy-download the *optional* sidecars instead of bundling them — tunnels
   (`cloudflared`, ~39 MB) and mail (`mailpit`, ~24 MB) are the obvious
   candidates, since not every user needs them. This is also the planned
   delivery model for the PHP/web-server runtimes.
3. Audit `cargo tree` for a dependency that pulled in a large transitive crate.

**Idle RAM** — profile with Instruments; the usual suspect is a sidecar left
running when it should be lazy, or a poll loop holding buffers.

**Cold startup** — defer non-critical work off the first-paint path (sidecar
boots, registry scans) behind the projects-table render.

## Regression history

| Date | Metric | Before → After | Cause | Resolution |
|---|---|---|---|---|
| _none yet_ | | | | |
