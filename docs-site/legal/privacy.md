---
title: Privacy Policy — PortBay
description: How PortBay handles your data. PortBay is local-first — project data and secrets stay on your machine, telemetry is opt-in, and Pro sync is end-to-end encrypted.
---

# Privacy Policy

_Last updated: June 4, 2026_

**Data controller:** Tribal House LLC, a limited liability company registered in Ghana
("Tribal House", "we", "us"). Privacy questions or data requests:
<privacy@portbay.app>.

PortBay is a local-first desktop app. The short version: your projects, their
configuration, and your secrets live on your own machine. We collect as little
as possible, we never sell your data, and the data that does reach our servers —
only when you turn on a hosted feature — is minimal and, for sync, end-to-end
encrypted.

This policy covers the PortBay desktop app and the optional hosted PortBay
account ("Pro"). The PortBay software is open source under AGPL-3.0-only; the
hosted account and sync are operated by Tribal House LLC.

## What stays on your device

By default, everything. PortBay manages local domains, certificates, runtimes,
databases, and project processes entirely on your Mac. Project records,
environment variables, and credentials are read and written locally and are not
sent to us unless you explicitly enable a hosted feature described below.

When you sign in, your session tokens are stored in the macOS Keychain; if the
Keychain is unavailable, the app falls back to a restricted-permission file in
its local data directory. Either way, tokens stay on your machine.

Two local features contact third parties directly from your device (we never
see this traffic):

- **Certificates.** If a site needs a publicly-trusted certificate, the bundled
  Caddy server contacts an ACME certificate authority (Let's Encrypt, ZeroSSL,
  or Google Trust Services), which receives the domain name and your IP address
  under its own privacy policy. Local `.test` certificates are issued by the
  built-in local CA with no external contact.
- **GitHub avatar.** If you sign in with GitHub, the app fetches your profile
  picture from GitHub's image CDN; GitHub receives your GitHub account id and
  IP address for that request.

## Telemetry and crash reports

Telemetry is **off by default** and runs only if you opt in from Settings. When
enabled, PortBay sends anonymous usage events and crash reports so we can find
bugs and prioritize work. These contain app and environment metadata — versions,
feature usage, and error traces — not your project contents, file paths, or
secrets. You can turn it off again at any time.

## PortBay accounts (Pro)

Creating an account is optional and only needed for hosted features. You sign in
with GitHub or an email magic link. What we store, and the legal basis for it
(GDPR Art. 6):

| Data | Why | Lawful basis |
|---|---|---|
| GitHub identity (id, login, avatar URL, email) or your email address | Identify your account | Contract (Art. 6(1)(b)) |
| Session records (a salted hash of your refresh token, timestamps) | Keep you signed in securely | Contract |
| License (tier, source, timestamps, revocation status) | Grant and verify your entitlement | Contract |
| Registered devices (name — defaults to your computer's hostname — platform, last seen) | Show and let you revoke synced devices | Contract |
| Sync metadata (version, size, timestamp of your encrypted blob) | Reconcile multi-device sync | Contract |
| Operational logs / IP address (transiently, via Cloudflare) | Security, abuse prevention | Legitimate interest (Art. 6(1)(f)) |

## Payments (Paddle)

Pro subscriptions are sold through **Paddle**, our Merchant of Record. Paddle
collects your name, billing address, and payment details under
[its own privacy policy](https://www.paddle.com/legal/privacy) and is an
independent controller for that data — **your card details never reach our
systems**. We receive only a purchase reference, the account it belongs to, and
the amount, so we can grant your entitlement.

## Encrypted sync

If you enable sync, your project registry is **encrypted on your machine before
it leaves it**. Only encrypted data reaches our servers — we cannot read it.
Synced records cover project metadata (host, command, port, and references to
environment variables). Machine-specific absolute paths and raw secret values
are **not** synced. The recovery key that decrypts your data is held only by
you; if you lose it, we cannot recover your synced data.

## Sub-processors and international transfers

| Party | Purpose | Location | Transfer safeguard |
|---|---|---|---|
| Cloudflare (Workers, D1, R2) | Hosting, database, encrypted blob storage | Global (US et al.) | EU–US Data Privacy Framework + SCCs |
| Paddle | Payments (independent controller, see above) | UK / US | Paddle's own DPF / SCC safeguards |
| AcumbaMail | Transactional + lifecycle email | EU | EU-based |
| GitHub (only with GitHub sign-in) | OAuth identity, avatar CDN | US | EU–US Data Privacy Framework |
| PostHog (only if you enable telemetry) | Aggregate analytics dashboard | US | EU–US Data Privacy Framework / SCCs |

Tribal House LLC is registered in Ghana and our infrastructure runs on
Cloudflare's global network, so data may be processed outside your country.
Transfers of EEA/UK data rely on the safeguards above; where we access account
data from outside the EEA, the same contractual safeguards and encryption
measures apply.

Our database and storage are deliberately not pinned to a single region: we
accept Cloudflare's global placement because all transfers carry the safeguards
above and sync content is end-to-end encrypted with keys that never leave your
devices. If your organisation requires EU-only storage, contact
privacy@portbay.app.

## What we don't do

- We don't sell or rent your data.
- We don't use your project contents for advertising or model training.
- We don't transmit your secrets or local file paths to our servers.

## Your rights

Under the GDPR you may access, rectify, erase, restrict, or port your data and
object to legitimate-interest processing; under the CCPA/CPRA you may know,
delete, and correct. We don't sell or "share" personal information.

- **Delete your account:** Settings → Account → **Delete account** signs you
  out everywhere and schedules the erasure of your account, license, devices,
  sessions, and encrypted sync blob. The purge completes **30 days** after the
  request; to cancel, sign back in within those 30 days and choose **Cancel
  deletion** in Settings → Account. You can also email <privacy@portbay.app> and we'll do it for you.
- **Export your data:** Settings → Account → **Export my data** downloads the
  account data we hold as a machine-readable JSON file. Also available by
  emailing <privacy@portbay.app>.
- We respond within the statutory window (30 days GDPR / 45 days CCPA). For
  billing data, contact Paddle or ask us to pass the request along.

Deleting your account removes your hosted data; your local projects are
untouched. A lapsed or revoked Pro entitlement only blocks new gated actions —
it never deletes or locks the projects already on your machine. You may also
complain to your local supervisory authority (GDPR) or the California Privacy
Protection Agency.

## Data retention

Account, license, device, and sync records are kept while your account exists.
Requesting deletion starts a 30-day grace window (cancel it any time from
Settings → Account after signing back in);
after it, everything is permanently erased. Sessions expire automatically
(refresh tokens within 90 days). Magic-link and login-flow records live for
minutes. Aggregated, anonymous telemetry may be retained for analysis.

## Changes

We'll update this page when our practices change and revise the date above.
Material changes to the hosted service are surfaced in-app.

## Contact

Privacy questions or data requests: <privacy@portbay.app>. Security reports:
<security@portbay.app>.
