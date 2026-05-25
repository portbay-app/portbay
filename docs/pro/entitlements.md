# PortBay Pro — Entitlement Specification

> **Status:** v2 (2026-05-24). This is the **single source of truth** for the
> Anonymous/Free/Pro split. The backend issuer, the client entitlement layer, every
> feature gate, the Pro License screen, and the docs all read from this file. Change
> tiers or limits **here first**, then propagate.
>
> **v2 supersedes the v1 flat "Community = 6" model.** The cap is now a **three-tier,
> auth-gated** ladder (Anonymous 3 / Free 6 / Pro ∞) and email magic-link auth joins
> GitHub OAuth. All open decisions were confirmed 2026-05-24 (§9).

Tracked on the board as *P1 — Pro — initiative overview, decisions & feature matrix*.

---

## 1. Principles

1. **Open-source first.** PortBay stays open-source (Apache-2.0). Pro is a way to *support* the project and
   get hosted convenience (sync), not a wall around the software.
2. **Frictionless first run.** A freshly installed app is fully usable immediately — no
   sign-up, no account, no friction. Anonymous users get a generous starting allowance
   (3 projects) before any prompt.
3. **Not DRM.** The app is recompilable — any check is bypassable by rebuilding. We do
   not pretend otherwise, and we cap effort on "hardening" the check accordingly. The
   real, non-bypassable value lives in the hosted account + sync.
4. **Earn it, with money or code.** Two honest acquisition paths to Pro: **Donate** or a
   **merged PR**. (FlyEnv's third "write a post → Pro" path is deliberately dropped —
   astroturf risk outweighs revenue.)
5. **Differentiate, don't clone.** The model is FlyEnv-inspired and shares ServBay's
   *free↔pro feature split*, but the *presentation* is PortBay's own — own copy, own
   visual, leading with PortBay's thesis (open · native · small · container-free).
   PortBay keeps a **perpetual, pay-what-you-want** model; it does **not** adopt
   ServBay's subscription pricing, and it is deliberately **more frictionless** than
   ServBay (which requires an account even for its free tier).
6. **Never punish a downgrade.** Expiry, revocation, or being over a cap never deletes
   or disables a user's existing projects/data — it only blocks *new* gated actions. A
   signed-in user whose Pro lapses falls back to **Free (6)**, never to Anonymous (3).
7. **Offline is sacred.** PortBay is a local tool. A Pro user must never lose function
   because the license server is unreachable (see the grace window in §6).

---

## 2. Tiers & acquisition

| | Anonymous | Free | Pro |
|---|---|---|---|
| Auth | none (fresh install) | signed in (GitHub **or** email) | signed in **+** Pro license |
| Price | free | free | pay-what-you-want |
| Projects | **3** | **6** | **unlimited** |
| How to get it | install & run | sign up free (GitHub or email magic-link) | **Donate** (any amount) **or** land a **merged, non-trivial PR** |

- **Anonymous → Free** is a pure sign-up (no payment). It is triggered when an anonymous
  user tries to add their **4th** project: the cap prompt offers *sign up / sign in*,
  which unlocks 6 (Free) — or unlimited if they already hold Pro.
- **Free → Pro** is triggered when a Free user tries to add their **7th** project: the
  prompt offers the two Pro acquisition paths.
- Auth supports **GitHub OAuth** *or* **email magic-link** (passwordless). Both resolve
  to an account keyed on a stable identity (GitHub id, or verified email). That identity
  is the join key across donation, PR, license, and synced devices.

---

## 3. Canonical feature matrix

| Feature | Anonymous | Free | Pro | Gating card |
|---|---|---|---|---|
| Core lifecycle / routing / certs / DNS / metrics | ✅ full | ✅ full | ✅ full | — |
| Websites & virtual hosts (projects) | **up to 3** | **up to 6** | unlimited | *gate project cap* |
| Multi-device sync (account login) | ❌ | ❌ | ✅ | *sync client* / *sync service* |
| Custom port & CORS configuration | defaults only | defaults only | ✅ custom | *gate port & CORS* |
| Built-in mail server (Mailpit) | **limited** (§4) | **limited** (§4) | full access | *gate mail* |
| Early access to new features | stable channel only | stable channel only | opt-in | *early-access flags* |
| Priority bug fixes & support | community | community | priority | (process, not code) |

The cap counts **projects** (registry records). A project may expose more than one host;
the cap is on projects, not hosts (confirmed 2026-05-24). **Free and Anonymous share the
same feature set** — the only difference is the project cap (6 vs 3) and that a Free user
has an account (the prerequisite for ever upgrading to Pro and for sync).

> **ServBay-parity gates considered but deferred (2026-05-24):** multiple simultaneous DB
> instances, automatic backups, and wildcard/multi-domain certs were evaluated as extra
> Pro gates. Decision: **not in v1** — they are filed as separate Pending cards. The v1
> Pro gate set is exactly the four rows above (sync, custom port/CORS, full mail, early
> access) plus the project cap.

---

## 4. "Limited" mail definition — CONFIRMED 2026-05-24

The Mailpit sidecar (`src-tauri/src/mailpit/`) is built. Confirmed split (applies to both
Anonymous and Free):

**Anonymous & Free (limited):**
- Catch and view local outbound mail (the common need: "did my app send the email?").
- Single default mailbox.
- Rolling retention — last **100** messages, cleared on cleanup.

**Pro (full access):**
- Multiple mailboxes / per-project routing.
- Unlimited retention + search + export.
- SMTP relay / forwarding to a real address.

Rationale: the non-Pro tiers are genuinely useful on their own (catching mail is the 90%
case); Pro adds power-user mail, not basic functionality. Not crippleware.

---

## 5. License shape — CONFIRMED 2026-05-24

- **Per-user**, keyed on a stable account identity (GitHub id or verified email). Devices
  are linked under the user.
- **Free is an account, not a license.** Signing up issues a signed **Free** entitlement
  (tier `free`, cap 6, no paid features). Pro is a signed **Pro** entitlement on top.
- **Perpetual, one-time** for Pro. A donation or merged PR grants lasting Pro — no
  subscription in v1 (matches the goodwill, pay-what-you-want model). Recurring
  subscriptions can be added later without breaking this shape.
- **Revocable.** Refunded donation, reverted PR, or abuse → `revoked: true`, honored on
  the next recheck. A revoked **Pro** falls back to **Free**; a revoked **Free** (abuse)
  falls back to **Anonymous**.
- **No hard device cap** in v1 — list devices for visibility, allow revoke; don't block.
- **Re-verified periodically**, not on every launch, and validated **offline** against a
  signed payload (§6).

---

## 6. Entitlement contract (the wire + cache shape)

The backend `/license` endpoint returns this **Ed25519-signed** document for any
signed-in user (Free or Pro). The client caches it, validates the signature locally with
the public key shipped in the binary, and trusts the cache offline. This is the one shape
the **backend**, the **client entitlement layer**, and **every gate** agree on.

```jsonc
{
  "schema": 2,
  "account":   { "github_id": 12345, "login": "octocat" }, // github_id null for email accounts
  "tier":      "pro",            // "free" | "pro"  (never "anonymous" — that's unsigned)
  "source":    "donate",         // "signup" | "donate" | "contribute" | "manual" | null
  "issued_at":     "2026-05-24T00:00:00Z",
  "recheck_after": "2026-06-23T00:00:00Z",  // client re-verifies after this
  "grace_days": 21,              // honor a stale Pro cache this long when server unreachable
  "revoked":   false,
  "entitlements": {
    "max_projects":    null,     // null = unlimited (Pro); Free = 6
    "sync":            true,
    "custom_port_cors":true,
    "mail":            "full",   // "limited" | "full"
    "early_access":    true,
    "priority_support":true
  },
  "sig": "<ed25519 signature over the canonical payload>"
}
```

A signed **Free** entitlement carries `tier: "free"`, `source: "signup"`, and the
non-Pro entitlement block:

```jsonc
{
  "schema": 2,
  "account": { "github_id": 12345, "login": "octocat" },
  "tier": "free", "source": "signup",
  "issued_at": "...", "recheck_after": "...", "grace_days": 21, "revoked": false,
  "entitlements": {
    "max_projects": 6, "sync": false, "custom_port_cors": false,
    "mail": "limited", "early_access": false, "priority_support": false
  },
  "sig": "..."
}
```

**Anonymous default** (no account / no token) is the client's built-in fallback — never
fetched, never signed:

```jsonc
{
  "schema": 2, "tier": "anonymous", "source": null, "account": null,
  "entitlements": {
    "max_projects": 3, "sync": false, "custom_port_cors": false,
    "mail": "limited", "early_access": false, "priority_support": false
  }
}
```

### Effective-state machine (client, offline-aware)

The client computes one **effective state** from the cache + clock. `account` presence
(i.e. "is the user signed in?") decides the *floor* a lapsed Pro falls back to.

| Cache | Age vs recheck/grace | Effective state | Cap | Paid features |
|---|---|---|---|---|
| none / cleared / no token | — | **anonymous** | 3 | off |
| signed `free` | any (low-stakes; honored offline) | **free** | 6 | off |
| signed `pro` | age ≤ recheck | **pro** | ∞ | on |
| signed `pro` | recheck < age ≤ recheck+grace | **pro-grace** | ∞ | on |
| signed `pro` | age > recheck+grace (offline) | **free** | 6 | off |
| signed `pro`, `revoked` | — | **free** | 6 | off |
| signed `free`, `revoked` | — | **anonymous** | 3 | off |

States the UI must handle: `anonymous` · `free` · `pro` · `pro-grace` · `unknown-offline`
(reserved for a future refresh-failure surface; the cache path resolves to one of the
first four). The fall-back floors implement Principle 6 — a lapsed/ revoked Pro who is
still signed in keeps **Free (6)**, never drops to Anonymous (3).

Signing keys: the **private** key lives only in the backend repo's secrets (§8); the app
ships the **public** key. Never put the private key in this public (Apache-2.0) repo. The dev key in the
tree must be **rotated to a fresh production key before launch** (update the embedded
`PUBLIC_KEY_B64` and the backend `LICENSE_SIGNING_KEY` together).

---

## 7. Backend boundary — CONFIRMED 2026-05-24

- **Separate private repo** `portbay-cloud`, not `/server` in this monorepo — keeps the
  license-signing private key, OAuth secrets, magic-link signing secret, payment webhook
  secrets, and SMTP credentials out of the public (Apache-2.0) tree.
- The public app depends on the backend only over HTTPS + the shipped public key.
- Stack: **Cloudflare Workers + Hono**, **D1** (accounts/licenses/devices), **R2**
  (encrypted sync blobs), **Ed25519** license signing. Email via **AcumbaMail SMTP** sent
  from a Workers-native SMTP client (no nodemailer — the Workers runtime has no Node TCP).
  Production URL: **`https://cloud.portbay.app`** (a Cloudflare custom domain bound to the
  Worker; the `…workers.dev` route is disabled — the custom domain is the only host).

---

## 8. Build sequence & v1 cut line

```
Phase 0 — Foundation
  • this spec  →  accounts backend architecture & infra spike            [done]

Phase 1 — Backend core
  • GitHub OAuth + email magic-link, accounts & sessions
  • license issuance & verification API   (emits §6 payload; free on signup)
  • encrypted registry sync service

Phase 2 — Client core (the keystone)
  • account login (GitHub + email magic-link) + secure token storage
  • entitlement layer with offline grace  ← consumes §6; all gates read from it

Phase 3 — Gates + UI
  • gate: project cap (3 / 6 / ∞) + auth-aware paywalls
  • gate: custom port & CORS
  • gate: mail full vs limited
  • Pro License + Sign-up screens + About License
  • sync client (device mgmt + conflict UX)

Phase 4 — Acquisition
  • Donate webhook → auto-issue Pro
  • Contribute (merged PR) → issue Pro   (needs CONTRIBUTING opened)

Phase 5 — Launch blockers (run in parallel with 3–4)
  • legal: Privacy Policy + ToS + GDPR
  • security review: auth, signing, sync authz, secrets
  • rotate production Ed25519 signing key

Phase 6 — Trailing (post-v1 OK)
  • early-access feature-flag system
  • CLI parity (portbay login / license)
  • abuse prevention & issuance observability
```

**v1 cut line:** Phases 0–5 ship together. **Legal, the security review, and the
production-key rotation are hard launch blockers** — no public Pro launch without them.
Phase 6 (the P3s) may trail the first release.

---

## 9. Decisions — confirmed 2026-05-24

| # | Decision | Confirmed | Blocks |
|---|---|---|---|
| 1 | Cap model | **3-tier: Anonymous 3 / Free 6 / Pro ∞** (auth-gated) | *gate project cap*, client layer, backend `/license` |
| 2 | "Limited" mail meaning | §4 split (same for Anonymous + Free) | *gate mail* |
| 3 | Pro pricing | Perpetual one-time, pay-what-you-want (no subscription; keep PWYW, not ServBay's model) | license API |
| 4 | Auth methods | GitHub OAuth **and** email **magic-link** (passwordless) | backend auth, client login |
| 5 | Device cap | None in v1 (visibility only) | sync service/client |
| 6 | Cap counts projects or hosts | Projects | *gate project cap* |
| 7 | Backend repo + hosting | Separate `portbay-cloud` repo, Cloudflare Workers | infra spike |
| 8 | ServBay-parity extra gates (multi-DB / backups / wildcard certs) | **Not in v1** — filed as separate Pending cards | (future) |

All confirmed 2026-05-24; dependent cards unblocked.
