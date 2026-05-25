# Security

## Reporting a Vulnerability

**Do not open a public GitHub issue for a security vulnerability.**

Email **security@portbay.app** with a description of the issue, steps to reproduce it, and the version of PortBay you are using. If you want to encrypt your report, ask for a PGP key in your initial email and we will provide one.

### What to expect

| Stage | Target timeline |
|---|---|
| Acknowledgment | 3 business days |
| Reproduction and initial assessment | 10 business days |
| Coordinated disclosure | After a patched release is available |

We will keep you informed at each stage. If you need to share additional information during the process, reply to the thread.

We do not offer a bug bounty program at this time.

---

## PortBay's Threat Model

PortBay Community is a local-first tool. Understanding what it does and does not do is important for evaluating its security properties.

### What PortBay does

- Reads and writes to your local hosts file (with privilege escalation via a helper where required)
- Installs and trusts local TLS certificates via mkcert — these are trusted only in your local trust store, not publicly
- Runs the start/stop commands defined in your project configuration
- Listens on local ports and routes traffic between them
- Reads project directories, environment variable files, and configuration as you configure it
- Stores project state and settings locally on your machine

### What PortBay must never do (and does not)

- **Upload your project paths, environment variables, logs, or configuration registry without explicit opt-in.** If you have not connected a PortBay Cloud account and explicitly enabled sync, nothing leaves your machine.
- Phone home silently. Any network request from the Community app is opt-in or user-initiated.
- Sandbox your project's code. PortBay runs your project commands with your privileges. It is not a security boundary between your project code and your system. Treat it the same way you would treat running those commands directly in your terminal.

### Local certificate trust (mkcert)

PortBay uses [mkcert](https://github.com/FiloSottile/mkcert) to issue certificates for `.test` domains and local HTTPS. These certificates are signed by a local CA that mkcert installs in your system and browser trust stores.

Key properties:

- The local CA is generated on your machine and is unique to it.
- Certificates issued by it are trusted only by that machine's applications.
- The CA private key stays on your machine; it is not uploaded anywhere.
- Removing mkcert's CA from your trust store invalidates all certificates it issued.

If someone gains access to the mkcert CA private key on your machine, they could issue certificates trusted by your local browsers. This is a reason to treat your machine's security as a prerequisite — PortBay does not change that calculus.

---

## Responsible Disclosure

We follow coordinated disclosure: we aim to have a fix in a release before vulnerability details are made public. We will credit reporters in release notes unless they prefer to remain anonymous.

If you discover a vulnerability and we have not responded within the timelines above, send a follow-up to the same address.

---

## Scope

Reports are in scope if they describe:

- Vulnerabilities in the PortBay Community codebase (Tauri/Rust/Svelte)
- Vulnerabilities in PortBay's local IPC, API, or certificate handling
- Unintended data exfiltration from the Community app

Out of scope (for the Community repo):

- Vulnerabilities in PortBay Cloud or Pro backend infrastructure (report these to the same address — they are handled by the same team)
- Social engineering or phishing
- Issues that require physical access to the machine
- Bugs in third-party dependencies — report those upstream; we will track and update our dependency if a fix is released

---

**Contact:** security@portbay.app
