# UX Design — PortBay GUI

> Companion to `docs/ARCHITECTURE.md`. The Phase 2 GUI cards reference this doc by section.
> Lean version of the §5 chapter from the original assessment plan; full plan to be reconstituted as it accretes.

---

## Target users (dual)

PortBay's hardest design constraint is serving two audiences without patronising either.

### Vince — the vibecoder
- New to local development. Uses Cursor / Claude Code / v0.
- Doesn't understand "127.0.0.1 vs localhost", "PHP-FPM vs nginx", "what's a reverse proxy."
- Gives up when a tool throws a stack trace or asks them to edit a config file.
- Will switch tools the moment yours feels intimidating.

### Sam — the senior engineer
- Has been hand-rolling nginx configs since 2014.
- Escaping ServBay's free-tier paywall and Herd's PHP-only scope.
- Wants the UI to *get out of the way* once they understand it.
- Will abandon if you hide what's happening or make simple things require clicks.

### Resolution
**Progressive disclosure with strong defaults.** A UI that delights Vince usually patronises Sam; one that empowers Sam usually overwhelms Vince. We resolve this by:
- Simple presentation by default.
- An obvious reveal for advanced detail.
- Strong, sensible defaults that mean Vince rarely needs the reveal.

---

## Information architecture (§5.1)

```
┌─────────────────────────────────────────────────────────────┐
│  PortBay                                      [+] [⚙] [⊙]   │  ← top bar
├─────────────────────────────────────────────────────────────┤
│ Sidebar         │  Main panel                                │
│                 │                                            │
│ ▸ Projects (4)  │  ┌──────────────────────────────────────┐  │
│ ▸ Services      │  │ Nour Beiruti         ● Running       │  │
│ ▸ Domains       │  │ next · :3010 · nour-beiruti.test     │  │
│ ▸ Logs          │  │              [▶]  [■]  [↻]  [↗]    │  │
│                 │  └──────────────────────────────────────┘  │
│ Footer:         │  ┌──────────────────────────────────────┐  │
│ caddy ● dns  ● │  │ Tribal House CMS     ○ Stopped       │  │
│ php-fpm ● mysql●│  │ php · tribal-house.test              │  │
│                 │  │              [▶]  [■]  [↻]  [↗]    │  │
└─────────────────┴──────────────────────────────────────────┘
```

Top bar — right-aligned actions:
- **[+]** Add Project (single, prominent, primary CTA).
- **[⚙]** Settings.
- **[⊙]** Universal Stop-All — red, always reachable, the most important reliability promise in the product.

Sidebar — Projects (the home), Services (shared service health), Domains, Logs.

Footer — shared service health pills (caddy, dns, php-fpm, mysql, etc.). Live status.

Main panel — list of projects as cards. Each card shows status badge, project id (bold), framework/type, hostname, port, and per-project action buttons.

---

## Progressive disclosure (§5.2)

Every screen has three depth levels:

| Level | For | Example: Add Project flow |
|---|---|---|
| **L1 — Simple** | Vince | "Drop your project folder here." Auto-detects framework, picks a port, generates a `.test` URL. One button: **Add**. |
| **L2 — Standard** | Most users | After detection, show: project name, URL, port, start command. Editable. |
| **L3 — Advanced** | Sam | "Show raw config" toggle reveals the JSON registry entry. Editable inline. Round-trips back to the form. |

This pattern applies to: service config, proxy rules, SSL certificates, environment variables — every input surface.

---

## Status taxonomy (§5.3)

A project (or service) is in exactly one of these states. Word and colour must always match — colour-blind users rely on the word, fast scanners on the colour.

| State | Word | Color | Icon | Meaning |
|---|---|---|---|---|
| Stopped | Stopped | gray | ○ | Not running, no error |
| Starting | Starting… | blue/cyan | ◐ | Process up, readiness pending |
| Running | Running | green | ● | Readiness passing |
| Unhealthy | Needs attention | amber/yellow | ⚠ | Process up but readiness failing |
| Crashed | Crashed | red | ✕ | Process exited unexpectedly |
| Port conflict | Port in use | orange | ⊘ | Couldn't start — another process holds the port |

No other states. No mystery spinners that spin forever.

The Rust core's status derivation rule (already implemented in `process_compose::types::Process::portbay_status`): a process is `Running` only when `is_running && (is_ready == "Ready" || no_probe)`. `is_ready` is stale after termination — never trust it alone.

---

## Error UX (§5.4)

Every error follows the same shape:

```
What happened:      Port 3010 is already in use.
Why it matters:     Nour Beiruti can't start until the port is free.
Who's using it:     node (PID 12345), started 2h ago from another project.
What you can do:    [Stop that process]   [Use port 3011 instead]   [Show details]
```

This single pattern handles 80% of beginner pain. Sam can collapse the explanatory bits via a density toggle (`UI density: comfortable | compact`).

The structure is mandatory: every Tauri command's error response renders into this template. The CLI mirrors it in plain text.

---

## Onboarding (§5.5)

First run, one screen, two paths:

1. **"I have an existing project"** → folder picker → framework auto-detect → done.
2. **"I'm starting fresh"** → template gallery (Next.js, Laravel, Vite, Astro, plain PHP) → scaffold + register.

Health check on first launch: verify mkcert, Caddy, dnsmasq are installed; offer to install missing pieces with one button (one privileged prompt, then never again).

**No tour. No "tip of the day."** If the UI needs explaining, redesign it.

---

## CLI parity (§5.6)

Every GUI action has a CLI equivalent (and vice-versa). The CLI binary is already shipped (`portbay <subcommand>` — see commit `4aaea90` and the kanban "P1 — CLI surface" outcome). Tauri commands wrap the same underlying functions.

This serves three audiences:
- Sam, who wants to script things.
- Vince, who eventually graduates and discovers the CLI organically (via tooltips: "Tip: you can also run `portbay start nour-beiruti`").
- Future automation (CI, hooks).

---

## Visual references (do not pixel-copy)

Study, do not lift:
- **[Coolify](https://coolify.io)** — service dashboard density, status cards.
- **[Tilt](https://tilt.dev)** — "status at a glance, errors can't scroll off-screen."
- **[Linear](https://linear.app)** — keyboard-first command palette (`Cmd-K`).
- **[Raycast](https://raycast.com)** — native macOS feel.

For visual lift (legally OK under their respective MIT licenses):
- **Lerd (MIT)** — `internal/ui/web/src/components/` Svelte atoms: `StatusPill`, `StatusDot`, `Badge`, `Icon`, `DashboardCard`. Plus store templates: `theme`, `commandPalette`, `route`. See `research-lerd.md` for the full lift list (to be reconstituted).

---

## Density toggle

`UI density: comfortable | compact`. User-pref persisted to local storage.

- **Comfortable** (Vince default): spaced cards, full status labels, friendly empty states with hints.
- **Compact** (Sam default after first use): tighter rows, icon-only status, no empty-state explainers.

---

## Component palette (Phase 2 scope)

Components to ship, in order of dependency:

1. **Atomic primitives** — `StatusPill`, `StatusDot`, `Badge`, `Icon`, `DashboardCard`. Lifted from Lerd under MIT.
2. **Project list** + per-project action row (Play / Stop / Restart / Open / Logs).
3. **Top bar** with `[+] [⚙] [⊙]`. Universal Stop-All wired to the `stop_many` Tauri command.
4. **Add Project wizard** with L1/L2/L3 progressive disclosure.
5. **Project detail panel** (right pane when a project is selected — full controls + log preview).
6. **Service status footer** — live pills for caddy, dns, php-fpm, etc.
7. **Log viewer** modal with per-project static tail (WS streaming follows).
8. **Onboarding** — first-run paths.

---

## Out of scope for Phase 2

- Command palette (`Cmd-K`) — Phase 3 polish.
- Tray icon / menu bar mode — Phase 3.
- Custom themes — defer; ship light/dark via `prefers-color-scheme` only.
- Multi-window — not needed.
- Mobile / touch — not a target.
- i18n — single-language v1.
