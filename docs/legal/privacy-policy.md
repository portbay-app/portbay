# PortBay Privacy Policy

**Operator / data controller:** Tribal House ("we", "us").
**Contact:** privacy@portbay.app
**Last updated:** 2026-05-24
**Governing scope:** This policy is written GDPR-first and also describes your rights
under the California Consumer Privacy Act (CCPA/CPRA). See §10 for the governing-law
note.

> ⚠️ **Pre-launch placeholders to confirm before publishing:** the contact address
> (`privacy@portbay.app` must be a monitored inbox) and the governing-law jurisdiction
> in §10 (`{{GOVERNING_LAW_JURISDICTION}}`) — tied to where Tribal House is registered.

---

## 1. The short version

PortBay is a **local-first** developer tool. The app runs on your machine and manages
local websites, certificates, DNS, and databases entirely on your device. **You can use
PortBay anonymously, fully offline, without an account — and in that mode we receive no
personal data from you at all.**

We only process personal data when you choose to **create an account** (to raise your
project limit, sync across devices, or get Pro). Even then, the data is minimal, and your
synced project data is **encrypted on your device before it reaches us** — we cannot read
it.

---

## 2. What PortBay does *not* collect

- We do **not** collect your source code, project files, environment variables, secrets,
  or database contents. These never leave your machine except inside the end-to-end
  encrypted sync blob (§4), which we cannot decrypt.
- We do **not** run analytics, tracking pixels, advertising SDKs, or third-party
  trackers in the desktop app. The app never connects to a third-party analytics
  service; the only host it can send diagnostics to is our own.
- The **anonymous** tier sends us nothing. No account, no telemetry by default.

Optional, opt-in crash reporting and usage telemetry are covered in §7.

---

## 3. Data we process when you create an account

Creating an account is optional and only needed to exceed the anonymous 3-project limit,
to sync, or to hold a Pro license.

| Data | Why | Lawful basis (GDPR) |
|---|---|---|
| **GitHub:** account id, login, avatar URL, and email (if you sign in with GitHub) | Identify your account; the join key for license + sync | Contract (Art. 6(1)(b)) |
| **Email address** (if you sign in with an email magic-link) | Identify your account; deliver the sign-in link | Contract (Art. 6(1)(b)) |
| **Sessions:** a salted hash of your refresh token, issue/expiry timestamps | Keep you signed in securely | Contract |
| **License:** tier, source (donation / contribution / manual), timestamps, revocation status | Grant and verify your entitlement | Contract |
| **Devices:** a device name and platform you register, last-seen time | Show and let you revoke your synced devices | Contract |
| **Sync metadata:** a version number, size, and timestamp of your encrypted blob | Reconcile multi-device sync | Contract |
| **Operational logs / IP address** (transiently, via our host Cloudflare) | Security, abuse prevention, delivering the API | Legitimate interest (Art. 6(1)(f)) |

We do **not** sell personal data, and we do **not** use it for advertising or profiling.

---

## 4. Multi-device sync (Pro)

If you enable sync, PortBay uploads your **project registry** to our storage so your other
devices can pull it. Before upload, the app **encrypts the registry on your device** with
a key derived from your account; the server stores only opaque ciphertext (in Cloudflare
R2) plus the metadata in §3. **We cannot read your synced configuration.** Deleting your
account (or a device) removes the associated blob.

---

## 5. Email we send you

We send email only to account holders, from `no-reply@portbay.app` via our email
sub-processor **AcumbaMail**:

- **Transactional** (always sent, required for the service): your sign-in magic link and
  a one-time welcome message.
- **Lifecycle** (engagement, legitimate interest with opt-out): a one-time check-in about
  a day after you start, and a one-time review request about a week in. Every lifecycle
  email contains a one-click **unsubscribe** link, and you can opt out at any time; opting
  out never affects transactional sign-in emails.

We do not run marketing newsletters or share your email with third parties.

---

## 6. Sub-processors

We keep the third-party processors to a minimum:

| Sub-processor | Purpose | Data | Location |
|---|---|---|---|
| **Cloudflare** (Workers, D1, R2, Analytics Engine) | Hosting, database, encrypted blob storage, and aggregate telemetry ingest | §3 data; encrypted sync blobs; opt-in telemetry (§7) | Global edge; configurable region |
| **AcumbaMail** | Transactional + lifecycle email delivery | Your email address, message content | EU |
| **GitHub** (only if you choose GitHub sign-in) | OAuth identity | OAuth profile | US |
| **PostHog** (only if you enable usage telemetry) | Aggregate product analytics dashboard | Opt-in telemetry only (§7); no account, device, or project identifiers | US |
| Payment processor (only if you donate) | Process a voluntary donation | Handled by the processor; we receive a reference id, not card data | — |

---

## 7. Optional crash reporting & usage telemetry

Both are **off by default** and controlled by a single toggle in Settings. When the
toggle is off, the app sends neither. In all cases the data goes only to **our own**
PortBay Cloud service; the app never contacts a third-party analytics provider directly.

**Crash reports.** PortBay can send crash reports, but only **opt-in** and only when you
confirm a send. Reports contain a stack trace and app/OS version; they are scrubbed of
obvious paths and never include your project contents.

**Usage telemetry.** When enabled, PortBay records a small set of product events — a
command or funnel-step name (e.g. "project_started"), a success flag, and your OS,
architecture, and app version. It carries **no project data, file paths, hostnames, or
account/device identifiers**, and is not tied to your identity. We process it in aggregate
to understand which features are used and where setup fails. Ingest runs through our
Cloudflare Worker into Cloudflare Analytics Engine and is forwarded server-side to PostHog
(§6) for the dashboard; because there is no per-user identifier, this produces counts and
funnels, not individual profiles.

---

## 8. Retention

- **Account, license, device, and sync records:** kept while your account exists; deleted
  when you delete your account.
- **Sessions:** expire automatically (refresh tokens within 90 days; access tokens within
  15 minutes) and are removed on sign-out.
- **Login flow + magic-link records:** minutes-scale TTL, then swept.
- **Operational logs:** short-lived, per our host's defaults.

---

## 9. Your rights

**Under GDPR (EEA/UK)** you may access, rectify, erase, restrict, or port your data, and
object to processing based on legitimate interest. **Under CCPA/CPRA (California)** you
may know what we collect, request deletion, and correct it; we do **not** sell or "share"
personal information, so there is nothing to opt out of in that sense.

- **Self-service deletion:** Settings → Account → Sign out removes the local session;
  deleting your account (`POST /account/delete`, surfaced in-app) erases your account,
  license, devices, sessions, and the encrypted sync blob.
- **Requests:** email privacy@portbay.app. We respond within the statutory window (30
  days GDPR / 45 days CCPA).

You may also lodge a complaint with your local supervisory authority (GDPR) or the
California Privacy Protection Agency.

---

## 10. Governing law & changes

This service is operated by Tribal House. Data-protection obligations are met under the
GDPR and CCPA/CPRA as applicable to you. The governing law for the related Terms of
Service is **{{GOVERNING_LAW_JURISDICTION}}** (to be finalized with the registered entity).

We may update this policy; material changes will be noted in-app and dated here. Continued
use after an update constitutes acceptance.
