# PortBay Security Whitepaper

**Audience:** security reviewers, procurement, and IT teams evaluating PortBay.
**Last updated:** 2026-07-17
**Operator:** Tribal House LLC (Ghana) — see the [Privacy Policy](../legal/privacy-policy.md).
**Scope:** This document describes PortBay's security architecture and the honest
limits of that architecture. It is written to be shared. It contains no secrets,
no private-infrastructure detail, and no material from PortBay's proprietary Cloud
backend (see [repository boundaries](../architecture/repo-boundaries.md)).

> **Compliance framing, stated plainly.** PortBay is **built to satisfy enterprise
> security review**. We are **not** claiming any certification we do not hold.
> No framework certifies *code*: SOC 2 and ISO 27001 certify an *organization's*
> audited controls over a time window; GDPR/CCPA bind a *controller's* practices;
> PCI binds a *merchant's* cardholder environment. This paper documents the
> technical controls those audits test. We will stand up a formal **SOC 2 program
> when we sell to teams**; until then we describe what the software actually does
> and where it falls short, and we would rather under-claim than over-claim.

---

## 1. Why the architecture starts from a strong position

Two structural choices remove most of the attack surface a typical SaaS carries,
and they are the first things an enterprise reviewer should weigh.

### 1.1 Local-first data residency — your data never touches our infrastructure

PortBay runs on your machine. Projects, source code, environment files, database
contents, hostnames, certificates, logs, and secrets are created, stored, and used
**on your device**. In the default anonymous mode PortBay has no account and sends
us nothing. This is not a policy promise layered on top of a cloud pipeline; it is
the shape of the product. The practical consequences for a reviewer:

- **There is no processor relationship to audit for your project data.** We cannot
  disclose, subpoena-produce, or breach data we never receive.
- **Data residency is on-device by default.** For teams that require it, the app
  can run fully offline.
- The only data that ever leaves the machine does so through paths the user
  explicitly enables — multi-device sync on Pro (encrypted on the device before
  upload; we hold the key, see §7.1), optional opt-in crash/telemetry (scrubbed,
  no project data, no per-user identifier), and connections the app makes *on your
  behalf* to third parties you configure (an ACME certificate authority, a GitHub
  avatar CDN). Each of these is enumerated in the [Privacy Policy](../legal/privacy-policy.md).

### 1.2 Paddle merchant-of-record — PortBay is PCI SAQ-A / out of PCI scope

All payments are processed by **Paddle**, acting as Merchant of Record and an
independent controller. Card data is collected by Paddle directly and **never
touches PortBay systems**; we receive only a purchase reference, the account it
belongs to, and the amount. Because no cardholder data enters our environment,
PortBay's payment exposure is **SAQ-A / PCI-out-of-scope**, and it stays that way
because the app never proxies or renders card fields.

---

## 2. Threat model

PortBay is a local developer tool with legitimate, deep access to the developer's
own machine: project folders, local processes, sidecar binaries, hostname routing,
and a local certificate authority. The threat model reflects that.

### 2.1 What PortBay trusts

- **The user**, and the project folders they explicitly add. A normal project
  `startCommand` runs on the user's machine, with the user's privileges, through
  the supervised process manager. PortBay is not a boundary between *your own*
  trusted project code and *your own* machine — treat running a project in PortBay
  the same way you would treat running its commands in your terminal.
- **The operating system keychain**, which holds account session tokens and
  connector/MCP secrets.

### 2.2 Adversaries the design defends against

| Adversary | Concern | Primary control |
|---|---|---|
| A dispatched or embedded **AI agent** acting on the user's project | Reads or exfiltrates secrets; takes destructive actions; enumerates other projects | Approval-gated value access, values-free-by-construction injection, per-run egress deny-by-default, allowlist child-environment scrub, tamper-evident audit (§4–§5) |
| **Untrusted third-party project code** run under Sandboxed Run (Pro) | Escapes its project folder; opens unexpected network egress | Fail-closed OS sandbox on all platforms (macOS `sandbox-exec`, Linux `bwrap`), write-scoping, network-policy modes, sandbox-denial surfacing (§6) |
| A **co-resident local process** or a **malicious website** the user visits | Drives the app's local HTTP surfaces to spend cloud keys or exfiltrate | Loopback-only binds, tight CSP, per-session bearer token on the optional inference proxy, scheme/executable allowlists on URL/exec paths (§6) |
| A **network attacker** on update/model-fetch paths | Ships a malicious update or model | minisign-signed updates and a signed model catalog verified *before parse*, plus content-digest installs (§6, and the [release signing key custody statement](./release-signing-key-custody.md)) |
| A **tampering party with data-directory write access** | Rewrites or truncates the vault or the audit log to hide activity | Keyed-HMAC chained audit with checkpoints, whole-vault integrity MAC (§4–§5) |
| **PortBay's own infrastructure being compromised** | Exposes customer project data | Local-first: source, files, secrets and databases are never uploaded at all. Sync content (registry, board, preferences) IS recoverable by an attacker holding **both** our storage and our key store — see §7.1 |

### 2.3 Explicit non-goals

PortBay does **not** claim to make malicious third-party code safe to promote to
unrestricted execution; Sandboxed Run is a *containment and inspection* layer, not
a proof of safety. It does not defend against an attacker who already has your
unlocked machine, and it does not replace your machine's own security posture —
whoever controls your device controls the local CA key and the OS keychain.

---

## 3. Cryptographic design — secret management

> **Availability note.** The advanced secret-management subsystem described in this
> section is part of **PortBay Pro's** enterprise capability set. PortBay Community
> ships with no bundled secrets and a deliberately minimal cloud surface (see
> [`SECURITY.md`](../../SECURITY.md)). Where a control below is still being sequenced
> into a general release, this paper says so rather than implying it is universally
> shipping today.

Secrets are protected with an **envelope-encryption** scheme rather than a single
master key over a blob.

- **Per-secret data encryption keys (DEKs).** Each secret value is sealed under its
  own DEK using **AES-256-GCM** (authenticated encryption). Compromise or rotation
  of one secret does not require re-sealing the whole vault, and there is no single
  key whose exposure reveals every value at once.
- **Key-encryption key (KEK) with working rotation.** DEKs are wrapped by a KEK.
  The KEK can be **rotated** without decrypting and re-encrypting every secret value
  — rotation re-wraps the DEKs, not the payloads — so rotation is cheap enough to do
  on a schedule (see the [key rotation policy](./key-rotation-policy.md)).
- **Password-derived key material via Argon2id.** Where a vault is protected by a
  user secret, the key is derived with **Argon2id** (memory-hard) rather than a fast
  hash, raising the cost of offline guessing.
- **Injective, canonically-encoded AAD.** GCM's associated data (AAD) binds each
  ciphertext to its context (identity, version) so a value cannot be silently
  transplanted from one record to another. The AAD is encoded injectively
  (length-prefixed / unambiguously delimited) so distinct contexts can never collide
  into the same AAD string.
- **Whole-vault integrity MAC.** Beyond per-record authentication, a keyed MAC is
  computed over the vault's structural invariants (record count and the ordered set
  of identities) so that *deleting* a record or *rolling back* to an older retained
  version is detectable, not just tampering *within* a record.
- **Memory hygiene.** Key and plaintext buffers are wrapped in zeroizing containers
  so they are wiped from memory on drop rather than lingering for a later heap or
  crash-dump scrape.

The OS keychain is used for account session tokens and for connector/MCP secrets;
transport is TLS throughout.

#### 7.1 Multi-device sync is encrypted, but not end-to-end

Sync content is encrypted **on the device** with AES-256-GCM before upload, and the
document labels the server indexes by are blinded HMAC tags it cannot invert. The
account key, however, is held by us and released to any device signed in to the
account — that is what lets a newly signed-in Mac sync with no setup step, and it
means **we can decrypt sync content**. Earlier versions of this document described
sync as end-to-end encrypted; that was accurate then and is not now.

What still holds: the storage bucket alone is not sufficient to read the data (the
key lives in a separate store), and sync carries the project *registry*, task board,
preferences and workflow definitions — never source code, project files, environment
variables, secrets, or database contents, which are not uploaded at all.

---

## 4. Tamper-evident audit design

Sensitive secret operations are recorded in an **append-only, keyed-HMAC chained**
audit log — each entry incorporates the HMAC of the previous entry, so the log is a
hash chain rather than a flat file.

- **Chaining detects any edit, reorder, or deletion.** Removing, altering, or
  reordering an entry breaks the chain from that point forward; the break is
  detectable by re-verifying the chain against a periodic **checkpoint**.
- **Values-free by construction.** Audit entries record *that* an operation happened
  (who/what/when, the identity acted on, the decision) but never the secret value —
  the log is safe to export and review.
- **Approval decisions are themselves authenticated.** When a reveal/rotate/delete/
  export/deploy is approved, the decision is recorded with a MAC, so an "it was
  approved" claim in the log is verifiable, not merely asserted.
- **Auditor-usable verify & export.** The chain can be verified and exported to a
  verifiable append-only (JSONL) artifact for an outside auditor, rather than the
  tamper-evidence being reachable only from internal test code.
- **Retention & rotation.** The default local retention target and the archival /
  off-box options are documented in the [audit retention policy](./audit-retention-policy.md).

This is a stronger posture than an unchained audit table: an attacker who can write
the log file still cannot rewrite history without leaving a detectable break.

---

## 5. Agent-safety model

PortBay can run AI agents that act on a developer's projects. The design assumes the
agent is *capable but not trusted* and keeps secret **values** out of its reach by
policy and approval, while being honest (see §7) about where that protection is a
hard boundary and where it is defense-in-depth.

- **Scoped, per-run access ("grant / lease").** An agent run receives secrets
  resolved into its process environment *scoped to that run* — a lease bound to the
  run rather than a standing grant of the whole vault.
- **Approval gate on every value-exposing operation.** Reveal, rotate, delete,
  export, and deploy of a secret value all route through a single approval gate. No
  agent-reachable path returns a secret value without a human approving it. This is
  the core, and it is true today.
- **Values-free-by-construction injection.** Secrets are injected into a child
  process at spawn time without the value passing back through an agent-readable
  channel; the injection is recorded in the audit log without the value.
- **Allowlist child-environment scrub.** When PortBay spawns a child process, the
  environment is built from a **known-good allowlist** (`env_clear()` plus an
  enumerated safe set plus the run's resolved project secrets), not from a denylist
  of "things we remembered to strip." An allowlist fails closed: a variable nobody
  anticipated is absent by default rather than leaked by omission.
- **Per-run egress proxy, deny-by-default.** Outbound network for a run can be
  routed through a per-run egress proxy that is **deny-by-default** with an explicit
  host allowlist, so a run that holds credentials cannot silently reach an arbitrary
  endpoint.
- **Lethal-trifecta awareness.** The design explicitly treats the dangerous
  combination of (a) access to private data, (b) exposure to untrusted content, and
  (c) an outbound channel as the thing to break up: the approval gate constrains (a),
  the audit + values-free injection limit what (b) can turn into, and the egress
  allowlist constrains (c). We do **not** claim this eliminates prompt-injection
  risk — see §7.

---

## 6. Application hardening

Controls that apply to the whole desktop app, independent of the secret subsystem:

- **Signed updates and a signed model catalog.** Updates are **minisign-signed**;
  the model catalog is signed and its signature is verified **before the catalog is
  parsed**, and model installs are pinned by content digest. Custody of the signing
  keys is described in the [release signing key custody statement](./release-signing-key-custody.md).
- **Fail-closed sandbox on all platforms.** Sandboxed Run (Pro) wraps an untrusted
  project in an OS sandbox — `sandbox-exec` on macOS, `bwrap` on Linux — constrains
  writes to the selected project folder plus temp/cache, supports loopback / outbound
  / full / blocked network policies, and surfaces sandbox-denial lines from the
  process logs. It fails **closed**.
- **Tight Content-Security-Policy.** The webview runs under a CSP with no
  `unsafe-eval` and no `unsafe-inline`. This matters because the webview renders
  LLM/agent output; the CSP is the containment that keeps rendered output from
  becoming executable script.
- **Loopback-only local surfaces.** The app's local HTTP/IPC surfaces bind to
  `127.0.0.1`. The optional OpenAI-compatible inference proxy is **opt-in** (the user
  starts it), loopback-bound, and requires a **per-session bearer token** minted at
  start, so a co-resident process or a visited website cannot drive inference through
  the user's stored provider keys.
- **Scheme and executable allowlists.** Frontend-triggered URL opening is restricted
  to `http`/`https`/`file`; custom schemes are blocked in the webview. Native flows
  that must open OS settings or a developer tool route through Rust handlers where the
  allowed scheme/executable is chosen by PortBay code, not by webview input.
- **UID-gated privileged helper.** The privileged hosts helper (which edits the
  system hosts file) is UID-gated and validates the hostname suffix it is asked to
  touch.
- **Redacted telemetry, crash, and inspector.** Opt-in telemetry carries no project
  data, file paths, hostnames, or account/device identifiers; opt-in crash reports
  are scrubbed of obvious paths and never include project contents; the local request
  inspector redacts `Authorization` and `Cookie`.
- **Governance CI.** Every push and PR runs license-denylist enforcement, a
  full-history [gitleaks](https://github.com/gitleaks/gitleaks) secret scan, a
  repository-boundary check, `cargo-audit`, `pnpm audit`, and CycloneDX SBOM
  generation, with Dependabot updates. See [`SECURITY.md`](../../SECURITY.md) and the
  [vendor FAQ](./vendor-faq.md).

---

## 7. Honest residual limits

This is the section a serious reviewer should read first. Where a control is
containment or defense-in-depth rather than a hard boundary, we say so.

- **Per-run agent identity is defense-in-depth, not a hard boundary — yet.** An agent
  run is identified by a run scope that the agent's own process can influence. That
  scope is backed by the sandbox attestation and by the approval gate, so it raises
  the bar and it is audited — but it is **not** a cryptographically unspoofable
  identity boundary. It becomes one only under the full **credential-broker identity
  model** (a non-spoofable run token issued outside the agent's control), which is on
  the roadmap. Until then, do not read "per-run isolation" as a hard tenant boundary.
  **What is a hard property today** is value-confidentiality: no agent-reachable path
  reveals a secret value without human approval. The residual risk of a spoofed run
  scope is *integrity and metadata* (an agent could act under the wrong run label or
  enumerate identities), **not** value disclosure.

- **The egress allowlist is containment, not DLP.** The per-run deny-by-default
  egress proxy constrains *where* a run can connect. It is **not** data-loss
  prevention: it does not inspect payloads for sensitive content, and on platforms
  that cannot enforce per-host filtering without the proxy enabled, containment
  depends on the proxy being on. Treat it as "this run can only talk to these hosts,"
  not "this run cannot leak."

- **Redaction is a best-effort backstop, not a guarantee.** The scrubbing applied to
  logs, telemetry, crash reports, and the request inspector removes known-sensitive
  fields (paths, `Authorization`/`Cookie`, obvious secrets). It is a backstop layered
  under the values-free-by-construction design — not a promise that no sensitive
  string can ever slip through a channel we did not anticipate. The primary
  protection is that values are not placed on those channels in the first place;
  redaction catches the residue.

- **First-contact SSH host-key trust on some deploy paths.** Interactive SSH verifies
  host keys (trust-on-first-use prompt, strict on known hosts, reject on change).
  Certain automated deploy-over-SFTP paths have historically trusted a *new* host key
  on first contact without a prompt; hardening these to prompt-or-require-known is
  tracked. If you deploy secrets over SFTP to hosts you have not previously pinned,
  verify the fingerprint out of band.

- **Durability of the vault depends on the OS keychain.** Loss of the keychain entry
  that protects the KEK makes the vault unrecoverable. This is a *durability* limit,
  not a confidentiality weakness; a KEK escrow / recovery-key option is on the
  roadmap. Keep your own backup of anything you cannot afford to re-enter.

- **Federated cloud credentials (OIDC) are not yet live-validated.** OIDC federation
  to cloud providers (AWS STS / GCP WIF / Azure AAD) exists in code but has not been
  validated end-to-end against real provider endpoints. We do not claim production
  federation until that smoke test is done.

---

## 8. Compliance posture & roadmap

**Today, honestly:**

- Local-first data residency; no processor relationship for your project data (§1.1).
- Paddle MoR ⇒ PCI **SAQ-A / out of scope** (§1.2).
- GDPR-first privacy program with a documented sub-processor list, CCPA/CPRA
  coverage, self-service export and deletion, and international-transfer safeguards —
  see the [Privacy Policy](../legal/privacy-policy.md).
- U.S. export compliance: PortBay uses standard published encryption; the
  §742.15(b) notification was filed with BIS and NSA, and the source and binaries are
  **not subject to the EAR** under 15 CFR §734.3(b)(3)/§734.7(b) — see the [NOTICE](../../NOTICE).
- A published vulnerability-disclosure process with RFC 9116 `security.txt` and a
  PGP key — see [`SECURITY.md`](../../SECURITY.md) and the
  [vulnerability disclosure policy](./vulnerability-disclosure-policy.md).
- Governance CI: secret scanning, dependency audit, license enforcement, boundary
  enforcement, and a Rust CycloneDX SBOM (§6).

**On the roadmap (named so you can hold us to it):**

- A formal **SOC 2 program** engaged with an auditor when we sell to teams.
- The **credential-broker identity model** that upgrades per-run agent identity from
  defense-in-depth to a hard boundary (§7).
- **Default-on egress** for runs that hold credentials.
- **A single multi-ecosystem SBOM** (Rust + npm + model licenses) as a release asset,
  plus `cargo-deny` bans/advisories/sources gating.
- Moving from the current **VPAT 2.5 partial self-assessment** ([accessibility statement](../accessibility.md), backed by a11y CI) to a **full WCAG 2.1 AA** conformance claim validated by assistive-technology testing.
- **Audit-log rotation that carries the HMAC chain across archives**, plus optional
  WORM / off-box shipping (see the [audit retention policy](./audit-retention-policy.md)).
- A **"regulated / air-gapped" master outbound kill-switch** that provably disables
  crash upload, telemetry, and web search for HIPAA-adjacent / classified-network
  tenants, with a visible indicator.

---

## 9. References

- [`SECURITY.md`](../../SECURITY.md) — reporting, threat model, secret scanning, GPG key, `security.txt`.
- [Privacy Policy](../legal/privacy-policy.md) — data handling, sub-processors, transfers, retention, rights.
- [Vendor security FAQ](./vendor-faq.md) — SIG/CAIQ-style pre-answered questionnaire.
- [Key rotation policy](./key-rotation-policy.md) · [Audit retention policy](./audit-retention-policy.md) · [Vulnerability disclosure policy](./vulnerability-disclosure-policy.md) · [Release signing key custody](./release-signing-key-custody.md).
- [Repository boundaries](../architecture/repo-boundaries.md) — the public/private line.
- [NOTICE](../../NOTICE) — third-party components and export-control filing.

**Security contact:** `security@portbay.app` · GitHub private advisory (preferred) ·
PGP key at `https://portbay.app/pgp-key.txt` (referenced from
`https://portbay.app/.well-known/security.txt`).
