---
title: PortBay Pro — Pricing, Features & How to Get Pro
description: PortBay Pro is $59/yr — unlimited projects, encrypted multi-device sync (up to 2 devices), and early-access features. OSS contributors earn Pro by merging a pull request.
---

# PortBay Pro

PortBay is free and open source under AGPL-3.0. **Pro is optional.** It is a
**$59/yr subscription** that unlocks the hosted conveniences — a synced account
across your machines, higher project limits, and a few power-user features.
Pro activates on up to **2 devices**, renews annually, and you can cancel
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
| Multi-device sync | — | — | End-to-end encrypted |
| Custom ports & CORS | Defaults | Defaults | Fully configurable |
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

### Buy Pro ($59/yr)

> **Coming soon.** The checkout is not yet live. Join the waitlist at
> [portbay.app/pro](https://portbay.app/pro) and you'll hear when it opens.

Pro is $59/yr, activates on up to 2 devices, renews annually, and you can
cancel anytime. Once checkout is live:

1. Sign in to PortBay (**Settings → Account**, or the user menu).
2. Open the upgrade sheet and choose **"Get Pro — $59/yr"**.
3. Complete checkout in your browser; Pro activates on your next license refresh.

### Contribute

Get Pro by improving PortBay. When a qualifying pull request you authored is
merged, Pro is issued to your GitHub account automatically.

1. Find an issue or improvement — see [Contributing](/contributing).
2. Open a pull request; once it's merged, Pro unlocks on your next refresh.

### Tip jar

[Buy Me a Coffee](https://buymeacoffee.com/beiruti) remains open as a voluntary
tip jar — tips are appreciated but do not grant Pro entitlements.

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

The `custom_port_cors` entitlement gates custom cross-origin policies
(`CorsConfig.allowedOrigins`). The basic listen port is never gated. The gate
only fires when introducing or changing an active CORS policy — clearing origins
back to empty is always allowed. An existing policy is preserved on downgrade;
we only reject the act of changing it.

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

The `sync` entitlement gates the sync client. Sync is end-to-end encrypted with
a recovery key only you hold.

1. **Settings → Sync → Set up sync.** PortBay generates a **recovery key** that
   encrypts your registry before it leaves your Mac.
2. Save the recovery key somewhere safe (a password manager) — it is the only
   way to read your synced data or add another device.
3. On a second machine: **Settings → Sync → Add this device with a key**, paste
   the recovery key, and your projects pull in.

Project records sync (host, command, port, env references). Machine-specific
absolute paths and raw secrets do not. Manage and revoke devices from the same
panel.

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
