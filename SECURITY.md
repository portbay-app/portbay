# Security Policy

PortBay is a local developer tool with access to project folders, local processes, sidecars, and hostname routing. Security reports are handled privately first.

## Supported Versions

| Version | Supported |
| --- | --- |
| `main` | Yes |
| Tagged pre-1.0 releases | Best effort |

## Reporting A Vulnerability

Email security@portbay.dev with:

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

## GPG

A project GPG key is not published yet. Do not send secrets until one is listed here.
