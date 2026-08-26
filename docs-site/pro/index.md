---
title: PortBay Pro — Pricing, Features & How to Get Pro
description: PortBay Pro is $10/mo — unlimited projects, encrypted multi-device sync (up to 2 devices), and early-access features. OSS contributors earn Pro by merging a pull request.
---

# PortBay Pro

PortBay is free and open source under AGPL-3.0. **Pro is optional.** It is a
**$10/mo subscription** that unlocks the hosted conveniences — a synced account
across your machines, higher project limits, and a few power-user features.
Pro activates on up to **2 devices**, renews monthly, and you can cancel
anytime.

We're up front about this because the alternative — quietly bolting a paywall
onto an "open, no-paywall" tool — would be a bait-and-switch. So here's exactly
what's free, what Pro adds, and how the system works under the hood.

## Tier model

PortBay uses three tiers. The tier is determined by whether you are signed in
and whether your account holds a Pro entitlement.

| | Anonymous | Free | Pro |
|---|---|---|---|
| Auth | None — fresh install | Signed in (GitHub or email) | Signed in + Pro license |
| Project cap | **3** | **6** | **Unlimited** |
| Devices | 1 | 1 | Up to 2 |
| Multi-device sync | — | — | Automatic, encrypted |
| Custom ports & CORS | Defaults | Fully configurable | Fully configurable |
| Mail server | Catch & view | Catch & view | Full SMTP access |
| Early access | — | — | Yes |
| Priority support | Community | Community | Priority |

**Anonymous and Free share the same feature set** — the only difference between
them is the project cap (3 vs 6) and that a Free account is the prerequisite for
ever upgrading to Pro and for sync.

The project cap counts **registry records** (projects), not hostnames. A project
may expose more than one virtual host; only the number of projects matters.

## How sign-in works

Sign-in supports two auth methods:

- **GitHub OAuth** — authorizes via GitHub and resolves to your GitHub account
  id. This is the stable identity key for donations, merged PRs, and license
  issuance.
- **Email magic-link** — passwordless. Enter an email address; PortBay Cloud
  sends a one-time link. The account is keyed on the verified email address.
  `github_id` is `null` for email-auth accounts.

Sign in from **Settings → Account** or the user menu in the app. The CLI and GUI
share one keychain session — signing in from either unlocks both.

```bash
portbay login      # GitHub or email — opens a browser or prints a URL
portbay license    # show tier, source, expiry/grace, and signed-in account
portbay logout
```

### Anonymous → Free

When an anonymous user tries to add their 4th project, the cap prompt offers
sign up / sign in. Completing sign-up issues a signed Free entitlement, raising
the cap to 6.

### Free → Pro

When a Free user tries to add their 7th project, the prompt offers the two Pro
acquisition paths (see below). Pro activates on the next license refresh.

## How to get Pro

### Buy Pro ($10/mo)

Pro is $10/mo with a 7-day free trial, activates on up to 2 devices, renews
monthly, and you can cancel anytime — cancelling during the trial costs nothing.

1. Sign in to PortBay (**Settings → Account**, or the user menu).
2. Open the upgrade sheet and choose **"Get Pro — $10/mo"**.
3. Complete checkout in your browser — a secure Paddle checkout opens; Pro
   activates on your next license refresh.

### Contribute

Get Pro by improving PortBay. When a qualifying pull request you authored is
merged, Pro is issued to your GitHub account automatically.

1. Find an issue or improvement — see [Contributing](/contributing).
2. Open a pull request; once it's merged, Pro unlocks on your next refresh.

### Sponsor

Get Pro by funding PortBay. A qualifying donation through [GitHub Sponsors](https://github.com/sponsors/portbay-app) earns the same perpetual Pro license on your GitHub account, and it funds ongoing development of the Community edition.

1. Open the [PortBay GitHub Sponsors page](https://github.com/sponsors/portbay-app) and choose a qualifying tier.
2. Pro unlocks on your next license refresh, tied to the GitHub account you sponsored from.

### Tip jar

[Buy Me a Coffee](https://buymeacoffee.com/beiruti) remains open as a voluntary
tip jar — tips are appreciated but do not grant Pro entitlements.

## Managing your subscription

Subscribers manage everything from **Settings → Account → Billing** in the app:

- See your renewal date (or, after cancelling, the date Pro stays active until).
- **Manage billing** opens the secure Paddle customer portal in your browser —
  update your payment method, download invoices and receipts, or cancel.
- **Cancel subscription** jumps straight to the cancellation form. Cancelling
  stops the next renewal; Pro stays active until the end of the period you've
  already paid for, then your account drops to the Free tier. Your projects are
  never touched.
- If a renewal payment fails, the Billing section shows a **payment past due**
  notice with a shortcut to update your card.

Purchases are processed by Paddle (our Merchant of Record), so invoices, taxes,
and refunds are handled there. Pro granted for a merged contribution doesn't
renew and has no billing to manage.

## Devices

A Pro license activates on **up to 2 devices**. Each device is tracked by a
stable per-install identifier that persists across app restarts and updates.

- The 2-device cap is enforced server-side at activation time.
- If you need to free a slot (reinstall on a new machine, decommission an old
  one), go to **Settings → Sync** and deactivate the device you no longer use.
- The server returns a clear `device_limit_reached` message and shows the
  current device list so you can self-manage.

## The signed entitlement

Every signed-in user receives a signed JSON document from PortBay Cloud's
`/license` endpoint. The client verifies the signature locally against an
Ed25519 public key shipped in the binary, caches the document on disk, and
trusts it offline. This is the shape both the backend and the Rust entitlement
layer agree on:

```jsonc
{
  "schema": 3,
  "account":       { "github_id": 12345, "login": "octocat" },
  "tier":          "pro",           // "free" | "pro"  (never "anonymous")
  "source":        "subscription",  // "signup" | "contribute" | "manual" | "subscription" | null
  "issued_at":     "2026-05-24T00:00:00.000Z",
  "recheck_after": "2026-06-23T00:00:00.000Z",
  "grace_days":    21,
  "revoked":       false,
  "entitlements": {
    "max_projects":    null,   // null = unlimited (Pro); Free = 6; anon = 3
    "max_devices":     2,      // Pro = 2; Free = 1; anon = 1
    "sync":            true,
    "custom_port_cors":true,
    "mail":            "full", // "limited" | "full"
    "early_access":    true,
    "priority_support":true
  },
  "sig": "<ed25519 over the canonical payload>"
}
```

The signature covers a canonical form of the payload (keys recursively sorted,
compact, top-level `sig` excluded) that the client reconstructs byte-for-byte
before calling `verify_strict`. Tampering with any field invalidates the
signature.

Anonymous is the client's built-in fallback — it is never fetched and never
signed.

## Offline grace

PortBay is a local tool. Pro access never drops because the license server is
unreachable.

The client computes one effective state from the cached document and the clock:

| Cache | Age | Effective state | Cap | Pro features |
|---|---|---|---|---|
| None / no token | — | `anonymous` | 3 | off |
| Signed `free` | Any | `free` | 6 | off |
| Signed `pro` | ≤ recheck window (default 30 days) | `pro` | ∞ | on |
| Signed `pro` | Recheck < age ≤ recheck + grace | `pro-grace` | ∞ | on |
| Signed `pro` | Age > recheck + grace (offline) | `free` | 6 | off |
| Signed `pro`, revoked | — | `free` | 6 | off |
| Signed `free`, revoked | — | `anonymous` | 3 | off |

The `recheck_after` field in the signed document sets when the client should
re-verify with the server. The default grace window is 21 days beyond recheck
(the `grace_days` field in the signed document). While inside the grace window
the entitlement state is `pro-grace` — Pro features remain active.

**A lapsed or revoked Pro never drops to Anonymous.** The signed-in floor is
always Free (6). Only a revoked Free account (abuse) falls to Anonymous (3).

The `unknown-offline` state is reserved for a future refresh-failure surface and
is not currently emitted.

## Feature gates

### Project cap

The cap is enforced identically by the GUI, the CLI, and every `add_project`
backend path. The backend check is the backstop; the GUI gates it proactively
before the wizard opens.

- Anonymous: 3 projects.
- Free: 6 projects.
- Pro: unlimited (`max_projects: null`).

A lapsed or revoked Pro uses the Free cap (6), never the Anonymous cap.
Existing projects above the cap are never deleted — only new adds are blocked.

### Custom ports & CORS

Custom listen ports and cross-origin policies (`CorsConfig.allowedOrigins`)
are available on the **Free** tier and above — this was Pro-gated before
GSTACK-0718 and has since been un-gated, because a custom port or CORS rule
is table-stakes for running one real project, not a premium convenience.

Only **Anonymous** (no account) still gates the `custom_port_cors`
entitlement: signing up for a free account is the ask there, not paying for
Pro. The basic listen port itself is never gated, on any tier. The gate only
fires when introducing or changing an *active* CORS policy — clearing
origins back to empty is always allowed, and an existing policy is preserved
on downgrade; we only reject the act of changing it.

### Mail server

The `mail` entitlement is either `"limited"` or `"full"`.

**Limited (Anonymous and Free):**
- Catch and view local outbound mail.
- Single default mailbox.
- Rolling retention — last 100 messages, cleared on cleanup.

**Full (Pro):**
- Multiple mailboxes / per-project routing.
- Unlimited retention, search, and export.
- SMTP relay / forwarding to a real address.

### Multi-device sync

The `sync` entitlement gates the sync client. **On Pro there is nothing to
switch on** — signing in starts sync, and signing in on a second Mac pulls your
workspace down. The earlier flow, where you copied a recovery key off the first
machine and pasted it into the second, is gone.

What travels, one document at a time:

- **Projects** — the registry record: name, host, command, port, environment
  *references*, and the project's icon.
- **Task boards** — cards, columns, per-project board config, and card
  **attachments** (including screen recordings), up to the 250 MB a Pro plan
  carries.
- **Workflows** — recipes and their triggers.
- **Preferences** and the masked-domain list.

What does not travel: machine-specific absolute paths, raw secrets, and a
database whose `data_dir` lives on another Mac. A project that syncs to a
machine without its folder still arrives — its board, workflows and cards are
all there, and only the source tree is missing until you point it at a folder.

Manage and revoke devices from **Settings → Sync**.

#### How the encryption actually works

PortBay encrypts your synced documents **on your device** with AES-256-GCM
before upload, and the labels the server indexes by are blinded so it cannot
tell one project or card from another by name.

**This is not end-to-end encryption, and we do not describe it as such.** It
was, back when the account key lived only on your own machines — which is
exactly what made you carry a recovery key to a second Mac by hand. So that
signing in on a new device just works, your account key is now held by your
account and released to devices signed in to it. That means **we hold the means
to decrypt your synced configuration**, and we say so rather than implying
otherwise.

What is still true: the storage bucket on its own is useless without the key
table beside it, and no key material is ever written to a log. The full
treatment is in the [Privacy Policy § 4](/legal/privacy-policy).

### Early access

The `early_access` entitlement opts a Pro account into in-development features
before they reach the stable channel. Features in the early-access stage are
gated at both the Svelte and Rust layers; graduating a feature to stable flips
its stage to `ga` with no call-site changes needed.

Current early-access features include [Sandboxed Projects](/guides/sandbox).

### Priority support

The `priority_support` field is a process distinction, not a code gate — Pro
accounts receive priority on bug reports and support requests.

## Your data & your rights

See the [Privacy Policy](https://docs.portbay.app/legal/privacy) and
[Terms of Service](https://docs.portbay.app/legal/terms). In short: the
software stays AGPL-3.0; the hosted Pro service has its own terms. You can
export or delete your account data at any time. A lapsed or revoked license
only blocks new gated actions — your existing projects are never touched.

PortBay is recompilable, and every Pro check is bypassable by rebuilding from
source. We don't pretend otherwise. The real value in Pro is the hosted account
and sync — the parts that genuinely need a server.
