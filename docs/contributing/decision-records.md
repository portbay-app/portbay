# Architecture Decision Records

PortBay uses a lightweight ADR (Architecture Decision Record) process to capture significant decisions in a discoverable, durable way. The goal is a record that a contributor joining six months later can read to understand why things are the way they are.

---

## Where ADRs live

```
docs/decisions/NNNN-short-title.md
```

Examples:
```
docs/decisions/0001-agpl-3-license.md
docs/decisions/0002-declarative-project-registry.md
docs/decisions/0003-dnsmasq-for-wildcard-test-dns.md
```

Numbering is sequential and zero-padded to four digits. Do not renumber existing ADRs even if gaps appear.

---

## ADR template

Copy this template when creating a new ADR:

```markdown
# NNNN — Title

**Status:** Proposed | Accepted | Superseded by [NNNN](NNNN-title.md)

## Context

What situation prompted this decision? What constraints, requirements, or
options were in play? Keep this factual — not a justification for the decision.

## Decision

What was decided? State it clearly and directly.

## Consequences

What follows from this decision? Include trade-offs, things that become
easier, things that become harder, and any follow-up decisions this creates.
```

Populate all three sections. "Context" is for facts; "Decision" is for the choice; "Consequences" is for honest trade-offs.

---

## Status values

| Status | Meaning |
|---|---|
| `Proposed` | Under discussion; not yet agreed |
| `Accepted` | Agreed and in effect |
| `Superseded by NNNN` | Replaced by a later ADR; link to the replacement |

Do not delete superseded ADRs. Update their status and link to the replacement so the history is traceable.

---

## When to write an ADR

An ADR is warranted when the decision:

- Changes the repository's license or contribution terms
- Changes the cloud/Pro boundary (what lives in this repo vs. `portbay-cloud`)
- Adds or removes a bundled sidecar
- Changes the declarative project registry schema in a backwards-incompatible way
- Introduces a new Rust crate or frontend framework with significant scope implications
- Changes the build toolchain or CI platform
- Establishes a pattern that future contributors will be expected to follow

Not every PR needs an ADR. Bug fixes, feature additions within existing patterns, and documentation updates do not. If you are unsure, ask in the PR or Discussion.

---

## Process

1. Draft the ADR as `docs/decisions/NNNN-title.md` with `Status: Proposed`.
2. Open a PR or Discussion pointing to the draft.
3. Once maintainers agree, update `Status: Accepted` and merge.
4. If the decision is later reversed or replaced, update the old ADR's status to `Superseded by NNNN` and link to the new one.

For decisions that are clearly uncontroversial (e.g. a minor tooling update that no one disputes), a maintainer may write and merge an ADR without a separate Discussion, as long as the context and consequences are documented honestly.

---

## Relationship to other docs

- [architecture.md](architecture.md) — contributor-facing orientation to the current architecture
- [docs/ARCHITECTURE.md](../../ARCHITECTURE.md) — full architecture reference
- [license-policy.md](license-policy.md) — license and boundary policy (itself a candidate for an ADR)
- [docs/architecture/repo-boundaries.md](../architecture/repo-boundaries.md) — repo boundary specification
