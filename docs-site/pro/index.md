# PortBay Pro

PortBay is free and open source under Apache-2.0. **Pro is optional.** It's a
pay-what-you-want, perpetual way to support the project and unlock the hosted
conveniences — a synced account across your machines, higher limits, and a few
power-user features. There is no subscription, and there is no feature you can't
also build yourself from the source.

We're up front about this because the alternative — quietly bolting a paywall
onto an "open, no-paywall" tool — would be a bait-and-switch. So here's exactly
what's free, what Pro adds, and how to get Pro with either money or code.

## What's free

A fresh install is fully usable immediately — no account, no sign-up:

- **3 projects** with no account, **6** once you create a free account.
- Automatic `.test` domains, trusted local HTTPS, wildcard DNS.
- Multi-runtime support (Node, PHP, Python, Go, Ruby, …).
- A built-in mail catcher (catch and view local mail).
- The full Rust core and CLI — the GUI is a client, not the source of truth.

## What Pro adds

| | Free | Pro |
|---|---|---|
| Projects | Up to 6 | **Unlimited** |
| Multi-device sync | — | **End-to-end encrypted** |
| Custom ports & CORS | Defaults | **Fully configurable** |
| Mail server | Catch & view | **Full SMTP access** |
| Early access to new features | — | **Yes** |
| Support | Community | **Priority** |

Sync is the one Pro feature that genuinely needs a server — your registry is
end-to-end encrypted with a recovery key only you hold, so we can't read your
project paths or environment. Everything else is an honest limit, not DRM:
PortBay is recompilable, and we'd rather be honest about that than pretend
otherwise.

## How to get Pro

Two honest paths — pay with **money** or with **code**. Either one issues a
perpetual Pro license tied to your GitHub account.

### 💰 Donate

Support the project financially and Pro unlocks automatically.

1. [Donate to PortBay](https://opencollective.com/portbay).
2. Sign in to PortBay with the GitHub account you want Pro on
   (**Settings → Account**, or the user menu).
3. Pro activates on your next license refresh. Already donated and don't see it?
   Open the upgrade sheet and choose **"Already done it? Refresh my license."**

### 💻 Contribute

Get Pro by improving PortBay. When a qualifying pull request you authored is
merged, Pro is issued to your GitHub account automatically.

1. Find an issue or improvement — see [Contributing](/contributing).
2. Open a pull request; once it's merged, Pro unlocks on your next refresh.

> Contributions are how an OSS tool earns a paywall honestly: if you'd rather
> spend an afternoon than a dollar, that's a first-class way in.

## Why pay for open source?

**You don't have to.** PortBay is Apache-2.0 — clone it, build it, and every Pro
capability is yours for free. Pro exists so that people who'd rather not do that
can fund the project's upkeep and get the hosted account + sync without the
legwork. Funding keeps PortBay independent, maintained, and free for everyone
else.

## Sync setup

Sync is Pro and end-to-end encrypted:

1. **Settings → Sync → Set up sync.** PortBay generates a **recovery key** that
   encrypts your registry before it leaves your Mac.
2. Save the recovery key somewhere safe (a password manager) — it's the only way
   to read your synced data or add another device. We can't recover it for you.
3. On a second machine: **Settings → Sync → Add this device with a key**, paste
   the recovery key, and your projects pull in.

Project records sync (host, command, port, env references); machine-specific
absolute paths and raw secrets do not. Manage and revoke devices from the same
panel.

## CLI: login & license

The CLI honors the same account and entitlements as the GUI:

```bash
portbay login        # GitHub or email — opens a browser / prints a URL
portbay license      # show tier, source, expiry/grace, and signed-in account
portbay logout
```

The CLI and GUI share one keychain session, so signing in from either unlocks
both. The project cap is enforced identically whether you add via the GUI or
`portbay add`.

## Your data & your rights

See the [Privacy Policy](https://docs.portbay.app/legal/privacy) and
[Terms of Service](https://docs.portbay.app/legal/terms). In short: the software
stays Apache-2.0; the hosted Pro service has terms. You can export or delete your
account data at any time, and a lapsed or revoked license only blocks new gated
actions — never your existing projects.
