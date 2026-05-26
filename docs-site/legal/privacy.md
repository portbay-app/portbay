---
title: Privacy Policy — PortBay
description: How PortBay handles your data. PortBay is local-first — project data and secrets stay on your machine, telemetry is opt-in, and Pro sync is end-to-end encrypted.
---

# Privacy Policy

_Last updated: May 26, 2026_

PortBay is a local-first desktop app. The short version: your projects, their
configuration, and your secrets live on your own machine. We collect as little
as possible, we never sell your data, and the data that does reach our servers —
only when you turn on a hosted feature — is minimal and, for sync, end-to-end
encrypted.

This policy covers the PortBay desktop app and the optional hosted PortBay
account ("Pro"). The PortBay software is open source under AGPL-3.0-only; the
hosted account and sync are operated by Tribal House and the PortBay
contributors.

## What stays on your device

By default, everything. PortBay manages local domains, certificates, runtimes,
databases, and project processes entirely on your Mac. Project records,
environment variables, and credentials are read and written locally and are not
sent to us unless you explicitly enable a hosted feature described below.

## Telemetry and crash reports

Telemetry is **off by default** and runs only if you opt in from Settings. When
enabled, PortBay sends anonymous usage events and crash reports so we can find
bugs and prioritize work. These contain app and environment metadata — versions,
feature usage, and error traces — not your project contents, file paths, or
secrets. You can turn it off again at any time.

## PortBay accounts (Pro)

Creating an account is optional and only needed for hosted features. You sign in
with GitHub or an email magic link. We store the minimum needed to run the
account: a stable identifier, the GitHub or email identity you sign in with, and
your entitlement (whether your account holds Pro).

## Encrypted sync

If you enable sync, your project registry is **encrypted on your machine before
it leaves it**. Only encrypted data reaches our servers — we cannot read it.
Synced records cover project metadata (host, command, port, and references to
environment variables). Machine-specific absolute paths and raw secret values
are **not** synced. The recovery key that decrypts your data is held only by
you; if you lose it, we cannot recover your synced data.

## What we don't do

- We don't sell or rent your data.
- We don't use your project contents for advertising or model training.
- We don't transmit your secrets or local file paths to our servers.

## Your rights

You can export or delete your account data at any time from Settings → Account.
Deleting your account removes your hosted data; your local projects are
untouched. A lapsed or revoked Pro entitlement only blocks new gated actions —
it never deletes or locks the projects already on your machine.

## Data retention

Account and entitlement records are kept while your account exists. Encrypted
sync data is retained until you remove the device, the record, or your account.
Aggregated, anonymous telemetry may be retained for analysis.

## Changes

We'll update this page when our practices change and revise the date above.
Material changes to the hosted service are surfaced in-app.

## Contact

Privacy questions or data requests: <privacy@portbay.app>. Security reports:
<security@portbay.app>.
