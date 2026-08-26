# PortBay Vendor Security FAQ

**Audience:** procurement and security teams running a SIG / CAIQ / bespoke vendor
questionnaire against PortBay.
**Last updated:** 2026-07-17
**How to read this:** each answer points at concrete, checkable evidence — a public
document, a CI workflow, a config file, or a named subsystem. Where a control is
partial or on the roadmap, the answer says so. For the reasoning behind these
controls, see the [Security Whitepaper](./whitepaper.md); for honest limits, see
its §7.

> **The single most important fact for this questionnaire:** PortBay is
> **local-first**. Your source code, project files, environment variables, secrets,
> and database contents are created, stored, and used **on your machine** and are
> **never received by PortBay**. Most rows in a standard SaaS questionnaire that ask
> "how is *our* data handled in *your* cloud" resolve to "we never hold it." See
> [Privacy Policy §1–§2](../legal/privacy-policy.md) and [Whitepaper §1.1](./whitepaper.md).

---

## A. Data handling & residency

**Q. What customer data do you store, and where?**
For your project data: **none.** PortBay runs on your device; project contents never
leave it. If you create an optional account, we store minimal account/license/device
metadata and, on Pro, an **encrypted** registry/board blob. We hold the encryption
key, so we *can* decrypt it — sync is encrypted in transit and at rest, but it is not
end-to-end encrypted. Source code, project files, environment variables, secrets and
database contents are never uploaded at all. Hosting is Cloudflare (global edge).
*Evidence:* [Privacy Policy §2–§4, §6](../legal/privacy-policy.md).

**Q. Where is data processed / can we get EU-only residency?**
Account metadata and encrypted sync blobs are placed and replicated across
Cloudflare's global network (not region-pinned); sync data is encrypted at rest with
the key held in a separate store, so the storage alone does not expose it — but we do
hold the key. Organizations that
require EU-only storage should contact `privacy@portbay.app`. *Evidence:* [Privacy
Policy §6a "Data residency"](../legal/privacy-policy.md).

**Q. Do you sell or share personal data, run ads, or profile users?**
No. No sale, no "sharing" in the CCPA sense, no advertising SDKs, no third-party
trackers in the desktop app. *Evidence:* [Privacy Policy §2, §3, §9](../legal/privacy-policy.md).

---

## B. Encryption

**Q. How is data encrypted in transit?**
TLS throughout for all network transport (rustls / standard TLS). *Evidence:*
[Privacy Policy §10](../legal/privacy-policy.md); [NOTICE "Export compliance"](../../NOTICE).

**Q. How is data encrypted at rest?**
- **Sync content:** AES-256-GCM, encrypted on your device before upload; the key
  never leaves your machines; the server holds only ciphertext.
- **Secret vault (PortBay Pro):** envelope encryption — each secret value sealed
  under its own **AES-256-GCM** data-encryption key (DEK), DEKs wrapped by a
  rotatable key-encryption key (KEK), password-derived material via **Argon2id**,
  injective AAD binding, and a whole-vault integrity MAC.
- **Account tokens / connector secrets:** OS keychain.
*Evidence:* [Whitepaper §3](./whitepaper.md); [Privacy Policy §4, §10](../legal/privacy-policy.md);
[key rotation policy](./key-rotation-policy.md).

---

## C. Key management

**Q. How are encryption keys managed and rotated?**
Per-secret DEKs under a KEK; the KEK rotates by re-wrapping DEKs (no bulk
re-encryption of values). Sync keys are derived on-device and never transmitted.
A per-installation fingerprint/HMAC key is used for tamper-evidence and is **not**
rotated by design (rotating it would invalidate the existing audit chain and vault
fingerprint). *Evidence:* [key rotation policy](./key-rotation-policy.md);
[Whitepaper §3–§4](./whitepaper.md).

**Q. Is there key escrow / recovery?**
Not yet — loss of the OS-keychain entry protecting the KEK makes the vault
unrecoverable (a durability limit, not a confidentiality one). A recovery-key /
escrow option is on the roadmap. Keep an independent backup of anything you cannot
re-enter. *Evidence:* [Whitepaper §7](./whitepaper.md); [key rotation policy](./key-rotation-policy.md).

---

## D. Access control & revocation

**Q. How is access to secret values controlled?**
Every value-exposing operation (reveal, rotate, delete, export, deploy) passes a
single **human approval gate**. No agent-reachable path returns a secret value
without approval; injection into a child process is values-free-by-construction and
uses an allowlist-built environment. *Evidence:* [Whitepaper §5](./whitepaper.md).

**Q. How do we revoke access (device, session, account)?**
Sessions expire automatically (refresh tokens ≤ 90 days, access tokens ≤ 15 min) and
are removed on sign-out; you can revoke synced devices; deleting your account signs
you out everywhere and schedules erasure. Removing PortBay's local mkcert CA
(`mkcert -uninstall`) revokes locally-issued certificate trust. *Evidence:* [Privacy
Policy §8, §9](../legal/privacy-policy.md); [`SECURITY.md` "Local Certificate"](../../SECURITY.md).

**Q. How do you handle agent / automation identity?**
Agent runs get scoped, per-run secret leases behind the approval gate. **Honest
limit:** the per-run identity is defense-in-depth (an agent can influence its own run
scope), not a cryptographically hard boundary, until the credential-broker identity
model lands. Value-confidentiality *is* a hard property today. *Evidence:*
[Whitepaper §5, §7](./whitepaper.md).

---

## E. Audit & logging

**Q. Do you keep a tamper-evident audit trail of sensitive operations?**
Yes — an append-only, **keyed-HMAC-chained** log where each entry incorporates the
prior entry's HMAC, verified against periodic checkpoints. Entries are
values-free; approval decisions are MAC'd. The chain can be **verified and exported**
to a verifiable JSONL artifact for an outside auditor. *Evidence:* [Whitepaper §4](./whitepaper.md).

**Q. What is the retention policy for audit logs?**
Default local retention target of **365 days**, with archival and an optional off-box
export path; chain-carrying rotation and WORM/off-box shipping are being delivered.
*Evidence:* [audit retention policy](./audit-retention-policy.md).

---

## F. Vulnerability management & disclosure

**Q. How do we report a vulnerability, and what are your response targets?**
GitHub private advisory (preferred) or `security@portbay.app` (PGP available).
Targets: acknowledgement 3 business days, reproduction 10 business days, fix plan
after reproduction, public disclosure after a patched release. RFC 9116
`security.txt` is published. *Evidence:* [`SECURITY.md`](../../SECURITY.md);
[vulnerability disclosure policy](./vulnerability-disclosure-policy.md);
`https://portbay.app/.well-known/security.txt`.

**Q. How do you find vulnerabilities in your own dependencies?**
Governance CI on every push/PR: `cargo-audit`, `pnpm audit`, a license denylist,
repository-boundary enforcement, and full-history `gitleaks` secret scanning, plus
Dependabot updates. *Evidence:* [Whitepaper §6](./whitepaper.md);
[`SECURITY.md` "Secret Handling And Scanning"](../../SECURITY.md);
`.gitleaks.toml`; the `governance` CI workflow.

**Q. Do you run a bug bounty?**
Not at this time; coordinated disclosure with reporter credit in release notes.
*Evidence:* [`docs/pages/security.md`](../pages/security.md).

---

## G. Software supply chain / SBOM

**Q. Can you provide an SBOM?**
A **CycloneDX** SBOM for the Rust workspace is generated in CI today. A single
multi-ecosystem SBOM (Rust + npm + model licenses) as a release asset, plus
`cargo-deny` (bans / advisories / sources), is on the roadmap. Bundled sidecar
binaries and statically-linked AI libraries, with pinned versions and SHA-256
verification, are enumerated in the [NOTICE](../../NOTICE). *Evidence:* [NOTICE](../../NOTICE);
[Whitepaper §6, §8](./whitepaper.md).

**Q. How do you protect the update / model-download supply chain?**
Updates are **minisign-signed**; the model catalog is signed and verified **before
parse**; model installs are content-digest pinned; sidecars are fetched at pinned
versions with SHA-256 verification. *Evidence:* [release signing key custody](./release-signing-key-custody.md);
[Whitepaper §6](./whitepaper.md); [NOTICE](../../NOTICE).

---

## H. Sub-processors & third parties

**Q. Who are your sub-processors?**
Cloudflare (hosting/DB/blob storage/telemetry ingest), Paddle (Merchant of Record —
independent controller), AcumbaMail (transactional/lifecycle email), GitHub (optional
sign-in / avatar), PostHog (only if you enable usage telemetry), and ACME CAs
(contacted from *your* device if you request a public cert). *Evidence:* [Privacy
Policy §6](../legal/privacy-policy.md).

**Q. Are payments PCI-scoped for PortBay?**
No — Paddle is Merchant of Record; card data never touches PortBay systems, placing
PortBay at **PCI SAQ-A / out of scope**. *Evidence:* [Whitepaper §1.2](./whitepaper.md);
[Privacy Policy §3, §6](../legal/privacy-policy.md).

---

## I. Incident response

**Q. What is your incident-response process?**
Security reports are triaged privately first (§F targets). Because PortBay is
local-first, a compromise of *our* infrastructure cannot expose your project contents
(source, files, secrets and databases are never on it). Sync content — registry, task
board, preferences — would be exposed by an attacker who obtained both our storage and
our key store, since we hold the sync key. For
incidents involving account metadata we notify affected account holders and, where
applicable, act under the GDPR breach-notification timelines as controller; Paddle
handles payment-data incidents as the independent controller for that data.
*Evidence:* [Privacy Policy §6, §10, §11](../legal/privacy-policy.md);
[`SECURITY.md` "Response Targets"](../../SECURITY.md).

---

## J. Data deletion & portability

**Q. How is data deleted / exported on request?**
Self-service in-app: **Delete account** signs you out everywhere and schedules
erasure of account, license, devices, sessions, and the encrypted sync blob, purged
30 days after the request (cancellable within the window); **Export my data**
downloads a machine-readable JSON of the account data we hold. Statutory response
windows: 30 days GDPR / 45 days CCPA. Payment records: contact Paddle as controller.
*Evidence:* [Privacy Policy §8, §9](../legal/privacy-policy.md).

---

## K. Accessibility (VPAT)

**Q. Do you have a VPAT / WCAG conformance statement?**
Yes — a **VPAT 2.5 partial-conformance self-assessment** against WCAG 2.1 A/AA, with a
documented remediation backlog and automated a11y CI (`svelte-check` + `@axe-core/playwright`).
We do **not** yet claim full WCAG 2.1 AA conformance, and the statement is explicit about
what has and has not been measured. *Evidence:*
[Accessibility conformance statement](../accessibility.md).

---

## L. Governance & certifications

**Q. Are you SOC 2 / ISO 27001 certified?**
No. PortBay is **built to satisfy enterprise security review**, and the technical
controls those audits test are documented here, but we hold no certification today. A
formal **SOC 2 program** will be engaged when we sell to teams. We would rather tell
you this than imply otherwise. *Evidence:* [Whitepaper §8](./whitepaper.md).

**Q. What legal entity are we contracting with, and under what license?**
Tribal House LLC (Ghana). PortBay Community is AGPL-3.0-only; PortBay Cloud/Pro is a
proprietary commercial service. *Evidence:* [NOTICE](../../NOTICE);
[repository boundaries](../architecture/repo-boundaries.md);
[Privacy Policy header](../legal/privacy-policy.md).
