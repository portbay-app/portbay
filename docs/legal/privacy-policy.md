# PortBay Privacy Policy

**Operator / data controller:** Tribal House LLC, a limited liability company registered
in Ghana ("Tribal House", "we", "us").
**Contact:** privacy@portbay.app
**Last updated:** 2026-06-04
**Governing scope:** This policy is written GDPR-first and also describes your rights
under the California Consumer Privacy Act (CCPA/CPRA). See §11 for the applicable-law
note.

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

**Local storage on your device.** When you sign in, the app stores your session tokens in
the macOS Keychain. If the Keychain is unavailable, it falls back to a restricted-
permission file in PortBay's local data directory on your device. Either way the tokens
stay on your machine.

**Connections the app makes on your behalf.** Two local features contact third parties
directly from your machine (we never see this traffic):

- **Certificates:** if you configure a site to use a publicly-trusted certificate, the
  bundled Caddy server contacts an ACME certificate authority (Let's Encrypt, ZeroSSL, or
  Google Trust Services). The CA receives the domain name being certified and your IP
  address, under that CA's own privacy policy. Local `.test` certificates issued by the
  built-in local CA involve no external contact.
- **GitHub avatar:** if you sign in with GitHub, the app fetches your profile picture
  from GitHub's image CDN; GitHub receives your GitHub account id and your IP address for
  that request.

---

## 3. Data we process when you create an account

Creating an account is optional and only needed to exceed the anonymous 3-project limit,
to sync, or to hold a Pro license.

| Data | Why | Lawful basis (GDPR) |
|---|---|---|
| **GitHub:** account id, login, avatar URL, and email (if you sign in with GitHub) | Identify your account; the join key for license + sync | Contract (Art. 6(1)(b)) |
| **Email address** (if you sign in with an email magic-link) | Identify your account; deliver the sign-in link | Contract (Art. 6(1)(b)) |
| **Sessions:** a salted hash of your refresh token, issue/expiry timestamps | Keep you signed in securely | Contract |
| **License:** tier, source (purchase / contribution / manual), timestamps, revocation status | Grant and verify your entitlement | Contract |
| **Devices:** a device name and platform you register, last-seen time. The device name defaults to your computer's hostname; you can rename it | Show and let you revoke your synced devices | Contract |
| **Sync metadata:** a version number, size, and timestamp of your encrypted blob | Reconcile multi-device sync | Contract |
| **Operational logs / IP address** (transiently, via our host Cloudflare) | Security, abuse prevention, delivering the API | Legitimate interest (Art. 6(1)(f)) |

We do **not** sell personal data, and we do **not** use it for advertising or profiling.

**Payments are handled by Paddle, not by us.** If you buy a Pro subscription, the
purchase is processed by **Paddle**, our Merchant of Record (§6). Paddle collects your
name, billing address, and payment details under its own privacy policy; **card data
never touches our systems**. We receive only a purchase reference, the account it
belongs to, and the amount — enough to grant your entitlement.

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

## 6. Sub-processors & payment partner

We keep third parties to a minimum:

| Party | Role | Purpose | Data | Location | Transfer safeguard |
|---|---|---|---|---|---|
| **Cloudflare** (Workers, D1, R2, Analytics Engine) | Processor | Hosting, database, encrypted blob storage, aggregate telemetry ingest | §3 data; encrypted sync blobs; opt-in telemetry (§7) | Global edge (US et al.) | EU–US Data Privacy Framework + SCCs (Cloudflare DPA) |
| **Paddle** | **Independent controller** (Merchant of Record) | Payment processing, tax, invoicing, refunds for Pro purchases | Buyer identity, billing address, payment details — collected by Paddle directly, never by us | UK / US | Paddle's own GDPR safeguards (DPF / SCCs); see [Paddle's privacy policy](https://www.paddle.com/legal/privacy) |
| **AcumbaMail** | Processor | Transactional + lifecycle email delivery | Your email address, message content | EU | EU-based (no third-country transfer) |
| **GitHub** (only if you choose GitHub sign-in) | Processor / IdP | OAuth identity; avatar CDN (§2) | OAuth profile; IP on avatar fetch | US | EU–US Data Privacy Framework |
| **PostHog** (only if you enable usage telemetry) | Processor | Aggregate product analytics dashboard | Opt-in telemetry only (§7); no account, device, or project identifiers | US | EU–US Data Privacy Framework / SCCs (PostHog DPA) |
| **ACME CAs** (Let's Encrypt, ZeroSSL, Google Trust Services) | Independent controllers | Certificate issuance, contacted directly from your device (§2) | Certified domain names, your IP | US | Disclosure only — traffic goes from your device, not through us |

## 6a. International data transfers

Tribal House LLC is registered in Ghana and our infrastructure runs on Cloudflare's
global network, so personal data may be processed outside your country, including in the
United States and other third countries. Where data of EEA/UK users is transferred to a
third country, we rely on the safeguards in the table above: the **EU–US Data Privacy
Framework** for certified US providers (Cloudflare, GitHub, PostHog) and **Standard
Contractual Clauses** incorporated in each provider's data-processing agreement where DPF
certification does not apply. Paddle, as an independent controller, maintains its own
GDPR transfer safeguards. Where we access account data ourselves from outside the EEA,
that access is protected by the same contractual safeguards and the security measures
described in this policy (encryption in transit and at rest, end-to-end encryption for
sync content).

**Data residency.** Our Cloudflare database and storage are not pinned to a single
region: Cloudflare places and replicates them across its global network. We have
assessed and accepted this global placement rather than EU-pinned hosting because every
transfer is covered by the safeguards above and the most sensitive content we hold —
your sync data — is end-to-end encrypted with keys that never leave your devices, so it
is unreadable wherever it is stored. We will revisit this decision if the legal basis
for these safeguards materially changes; if your organisation requires EU-only storage,
contact privacy@portbay.app.

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

- **Account, license, device, and sync records:** kept while your account exists.
  Requesting deletion starts a 30-day grace window (during which you can sign back in
  and choose **Cancel deletion** in Settings → Account); after it, everything is
  permanently erased.
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

- **Deletion (self-service):** Settings → Account → **Delete account** signs you out
  everywhere and schedules the erasure of your account, license, devices, sessions, and
  the encrypted sync blob. The purge completes **30 days** after the request; to cancel, sign back in
  within those 30 days and choose **Cancel deletion** in Settings → Account. You can also email
  privacy@portbay.app and we will do it for you.
- **Export (self-service):** Settings → Account → **Export my data** downloads the
  account data we hold (account, devices, license and purchase records, sync metadata)
  as a machine-readable JSON file. Also available by emailing privacy@portbay.app.
- **Requests:** we respond within the statutory window (30 days GDPR / 45 days CCPA).
- For payment and billing data, contact **Paddle**, the controller for purchase records
  (§6), or email us and we will pass the request along.

You may also lodge a complaint with your local supervisory authority (GDPR) or the
California Privacy Protection Agency.

---

## 10. Security

Sync content is end-to-end encrypted on your device (AES-256-GCM; the key never leaves
your machine). Transport is TLS throughout. Server-side tokens are stored hashed; magic
links are single-use and short-lived. No system is perfectly secure, but the design goal
is that a compromise of our infrastructure cannot expose your project contents.

---

## 11. Applicable law & changes

This service is operated by Tribal House LLC (Ghana). Data-protection obligations are met
under the GDPR and CCPA/CPRA as applicable to you, and nothing in this policy limits the
rights you have under the data-protection law of your country of residence. The terms
governing the service itself are in the [Terms of Service](terms-of-service.md).

We may update this policy; material changes will be noted in-app and dated here. Continued
use after an update constitutes acceptance.
