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
  <!-- TODO(maintainers): confirm a monitored security@portbay.app mailbox (or
       GPG-enabled alias) is provisioned before the first public release. -->

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

PortBay trusts the user and the project folders they explicitly add. It does not sandbox arbitrary project code. A project `startCommand` runs on the user's machine with the user's privileges through Process Compose.

PortBay must not leak or upload:

- Project paths.
- Hostnames.
- Environment variables.
- Registry contents.
- Logs.
- Crash reports unless the user explicitly opts in.

## Out Of Scope

- Bugs requiring physical access to an unlocked machine.
- Vulnerabilities in third-party project code launched by PortBay.
- Social engineering against maintainers or users.
- Denial of service by intentionally malformed local project commands.

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

## GPG

A project GPG key is not published yet. Do not send secrets until one is listed here.
