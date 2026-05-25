# PortBay — funding case

> Internal-first working document for grant and institutional-funding
> applications. Keep it honest: no invented metrics. Update the traction section
> with real numbers once v0.1.0 ships and the repo is public-facing.

## One line

PortBay is an open-source (Apache-2.0), container-free, native local-development
manager for macOS — an honest alternative to the closed, paid tools developers
currently rely on to run their local sites.

## The problem (public good)

Local development environments are fragmented and heavy. Developers juggle a
"daemon zoo" (a DNS resolver, a web server, PHP-FPM, databases, a mail catcher)
or pay for closed tools (Laravel Herd, ServBay) or run everything in containers
that idle at gigabytes of RAM. None of the polished options are open source.

This matters beyond one developer's convenience:

- **Reduces fragmentation.** One tool that manages mixed Node/PHP/static/DB
  stacks with trusted local HTTPS and `.test` domains, instead of a per-stack
  pile of scripts.
- **Lower resource use.** Native, container-free; small idle footprint vs.
  always-on VMs/containers — a real energy and hardware-longevity argument.
- **Open alternative to paid closed tools.** The capable local-dev managers are
  proprietary. PortBay is the open one, so the ecosystem isn't gated behind a
  single vendor.
- **Privacy by default.** Local-first; the optional hosted sync is end-to-end
  encrypted, so even the funded service can't read user project data.

## What's built (readiness, not hype)

- Rust core with full CLI parity (the GUI is a client, not the source of truth).
- Process Compose, Caddy (admin API), mkcert/HTTPS, wildcard DNS (dnsmasq),
  Mailpit, multi-runtime detection (Node/PHP/Python/Go/Ruby).
- A green test suite (Rust unit tests + frontend `svelte-check`) and CI.
- An optional Pro tier (donate **or** contribute) for sustainability, with the
  software staying fully Apache-2.0.

## Traction & community readiness

*(Fill with real figures before submitting — do not invent.)*

- Repository: `github.com/portbay-app/portbay` (public).
- Stars / forks / issues: **TBD — record actual counts at submission time.**
- Releases / downloads: **TBD — populate after v0.1.0 ships.**
- Community: GitHub Discussions enabled; CONTRIBUTING opens code contributions
  with a clear path (and a contribute-to-earn-Pro incentive).

## Roadmap (tie to public milestones)

Phases, to be published as dated GitHub Milestones:

1. **v0.1.0 — installable, signed macOS build.** (Funder non-negotiable.)
2. **v0.2.0 — auto-update + Homebrew distribution.**
3. **v0.3.0 — bundled databases, mail server depth.**
4. **v0.4.0 — Linux support.**
5. **v0.5.0 — Windows support.**

## Budget / ask (line items)

A lump-sum grant would fund delivery of the roadmap above:

- **Apple Developer ID** signing cert + notarization (annual).
- **CI minutes** for cross-platform build + test matrices.
- **Cross-platform engineering** (Linux, then Windows) — the largest line.
- **Maintainer time** — issue triage, review, releases, security response.
- **Security** — dependency scanning, a periodic external review.

Indicative range: aligns with **NLnet NGI0** grants (€5k–50k).

## Comparison to closed tools

| | PortBay | Laravel Herd | ServBay |
|---|---|---|---|
| Open source | ✅ Apache-2.0 | ❌ | ❌ |
| Container-free | ✅ | ✅ | ✅ |
| Multi-runtime | ✅ | PHP-first | ✅ |
| Funding model | OSS + optional PWYW Pro | Paid Pro | Paid |

The differentiator funders care about: PortBay is the **open** one. A grant keeps
it independent and free for everyone, rather than pushing it toward the
subscription model its closed competitors use.

## Program shortlist (fit + timing)

| Program | Fit | Timing | Notes |
|---|---|---|---|
| **NLnet NGI0** | High — dev tooling, open infra, privacy | Rolling calls | Best first target; €5k–50k; lightweight application. |
| **Sovereign Tech Fund / Agency** | Medium-High — maintenance of widely-used OSS infra | Rolling intake | Needs demonstrated usage; revisit once downloads exist. |
| **GitHub Accelerator / Open Source Pledge** | Medium — cohort-based | Cohort dates (verify yearly) | Needs traction + a maintainer story. |
| **Smaller dev-tool grants** (e.g. foundation microgrants) | Medium | Varies | Good for CI/signing line items. |

*Confirm each program's current deadlines and criteria at application time — these
move year to year.*
