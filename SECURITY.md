# Security Policy

PortBay is a local developer tool with access to project folders, local processes, sidecars, and hostname routing. Security reports are handled privately first.

## Supported Versions

| Version | Supported |
| --- | --- |
| `main` | Yes |
| Tagged pre-1.0 releases | Best effort |

## Reporting A Vulnerability

Report privately through either channel:

- **GitHub private advisory (preferred):**
  [Report a vulnerability](https://github.com/portbay-app/portbay/security/advisories/new)
- **Email:** security@portbay.app
  <!-- Provisioned via Cloudflare Email Routing → the monitored Tribal House
       support inbox (routing verified live 2026-06-04; details in the private
       portbay-cloud repo, docs/dpa-register.md "Role mailboxes"). -->

Include:

- PortBay version or commit SHA.
- macOS version.
- Clear reproduction steps.
- Impact assessment.
- Whether the issue requires a malicious project folder, local user access, or remote input.

Do not file public issues for vulnerabilities.

## Response Targets

| Stage | Target |
| --- | --- |
| Initial acknowledgement | 3 business days |
| Reproduction or clarification | 10 business days |
| Fix plan | After reproduction |
| Public disclosure | After a patched release is available |

## Threat Model

PortBay trusts the user and the project folders they explicitly add. A normal project `startCommand` runs on the user's machine with the user's privileges through Process Compose.

PortBay Pro adds a Sandboxed Run path for untrusted external projects. That mode wraps the supervised process in a PortBay-generated macOS `sandbox-exec` profile, constrains writes to the selected project folder plus temp/cache locations, supports loopback/outbound/full/blocked network policies, and surfaces sandbox denial lines from process logs. Treat Sandbox as a containment and inspection layer, not as a guarantee that malicious code is safe to promote to unrestricted local execution.

PortBay must not leak or upload:

- Project paths.
- Hostnames.
- Environment variables.
- Registry contents.
- Logs.
- Crash reports unless the user explicitly opts in.

## Out Of Scope

- Bugs requiring physical access to an unlocked machine.
- Vulnerabilities in third-party project code launched outside PortBay's Sandboxed Run containment.
- Social engineering against maintainers or users.
- Denial of service by intentionally malformed local project commands.

## Webview And URL Opening

Frontend-triggered URL opening is restricted to `http:`, `https:`, and `file:` URLs by `$lib/security/openUrl`. Custom schemes are deliberately blocked in the webview. Native-only flows that must open macOS settings or trusted local developer tools route through Rust command handlers where the allowed scheme or executable is selected by PortBay code, not by arbitrary webview input.

## Secret Handling And Scanning

PortBay Community ships no secrets. The only cloud endpoint it knows is a public
domain, and the only embedded key is the **public** entitlement-verification key
(the matching private key lives only in the private `portbay-cloud` repo). To
keep it that way:

- **Never commit** real credentials, tokens, private keys, or `.env` files.
  `.env` is gitignored; `.env.example` holds only safe placeholders.
- **Secret scanning** runs in CI via [gitleaks](https://github.com/gitleaks/gitleaks)
  (config: `.gitleaks.toml`). Enable it locally too:
  ```bash
  brew install gitleaks pre-commit
  pre-commit install        # see .pre-commit-config.yaml
  ```
  This scans staged changes for credentials on every commit.
- **Boundary scanning** (`scripts/check-repo-boundaries.sh`, config
  `.repo-boundary-denylist`) blocks proprietary Cloud/Pro markers and private
  endpoints from entering this repo. See
  [repo boundaries](./docs/architecture/repo-boundaries.md).
- If you discover a committed secret, treat it as compromised: rotate it, then
  report via the private channel above so we can scrub history.

## Local Certificate And HTTPS Considerations

PortBay uses [mkcert](https://github.com/FiloSottile/mkcert) to mint
locally-trusted certificates for `*.test` hostnames. This installs a local
development Certificate Authority into your system/browser trust stores. The CA
private key stays on your machine and is never uploaded. Removing PortBay's
mkcert CA (`mkcert -uninstall`) revokes that local trust. Treat the local CA key
like any other private key on your device.

## Session Environment Override

`PORTBAY_SESSION_JSON` lets headless/CI environments inject account session
tokens through the environment instead of the OS keychain (lookup order:
keychain → environment → `~/.config/PortBay/session.json` with `0600`
permissions; see `src-tauri/src/auth/mod.rs`). Treat it as
**security-sensitive**: anything that can read the process environment (shell
history, CI logs, crash dumps, `ps e`) can read the tokens. Prefer the keychain
on interactive machines; if you must use the override, scope it to the single
process invocation and never persist it in dotfiles or CI variables shared
across jobs. The app logs a warning whenever a session is loaded from the
environment so its use is never silent.

## GPG

Encrypted vulnerability reports are welcome. The project security key:

- **Key:** `PortBay Security <security@portbay.app>`
- **Fingerprint:** `608D 45E3 02D1 A94A 1C0F  710F 8B54 3B23 8488 57AD`
- **Algorithms / expiry:** Ed25519 (sign) + Curve25519 (encrypt), expires 2028-06-04
- **Download:** <https://portbay.app/pgp-key.txt> — also referenced from
  <https://portbay.app/.well-known/security.txt> (RFC 9116)

Encrypt to that key and email security@portbay.app. Plaintext reports remain
fine when no sensitive details are involved.
