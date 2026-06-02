# PortBay SSL Parity Report

Date: 2026-06-01

## Audit Scope

PortBay was reviewed across the mkcert wrapper, certificate commands, Caddy config generation, reconciler, sidecar status, Domains page, Certificates page, project detail health checks, CLI/MCP certificate readers, and docs. ServBay was inspected only through accessible app resources and user-visible files under `/Applications/ServBay.app`, `/Applications/ServBay`, and `~/Library/Application Support/Dev.ServBay.macOS.ServBay`; no binaries were decompiled.

## ServBay Observations

Accessible ServBay resources on this machine were limited. The install includes service scripts for Caddy, Nginx, Apache, dnsmasq, and related services. The strongest SSL signal is `/Applications/ServBay/script/init/ca.sh`: it starts Caddy if needed, uses `caddy trust --config /Applications/ServBay/etc/caddy/Caddyfile` to install Caddy's local CA into macOS trust settings, and uses `caddy untrust` for removal. Service scripts expose start, stop, reload, restart, kill, and status workflows with PID files under `/Applications/ServBay/tmp`.

The current `/Applications/ServBay` tree had no active `etc`, `logs`, or generated certificate files to parse. A local customization script referenced trusted mkcert certificates under `/Applications/ServBay/ssl/private/mkcert-certs`, but those files were not present in the inspected tree. That points to a practical pattern rather than a ServBay built-in guarantee: users expect local-dev stacks to support wildcard/local trusted certs and to let generated web-server configs consume externally trusted cert material.

## PortBay Before Changes

Already supported:

- P1 Automatic local HTTPS through bundled mkcert.
- P1 User-driven `mkcert -install` flow.
- P1 Per-project `cert.pem` and `key.pem` storage under the PortBay data directory.
- P1 Caddy TLS integration through `tls.certificates.load_files` and SNI selection.
- P1 Per-project certificate metadata, reissue action, and certificate folder reveal.
- P1 Renewal when auto-renew is enabled and a cert is within 30 days of expiry.
- P1 Exact project hostname SANs and optional `*.hostname` SANs.
- P1 Key permission hardening to `0600` and cert directory hardening to `0700`.
- P2 Cleanup of cert dirs for removed projects.

Gaps found:

- P0 CA status only checked for `rootCA.pem`, not actual macOS trust-store state.
- P0 Untrusted CA and missing CA were collapsed into one broad state.
- P1 Certificates did not include loopback SANs for `localhost`, `127.0.0.1`, and `::1`.
- P1 UI health treated any readable cert as healthy, even expired or untrusted certs.
- P1 Custom certificate mode had UI copy but no schema or Caddy loading path.
- P1 Custom cert/key pair validation was missing.
- P2 SSL modes were implicit rather than explicit.
- P2 Public ACME/AutoSSL was not represented as a future-only mode.
- P2 CLI/MCP can read cert metadata but does not install/uninstall CA because that remains privileged and interactive.

## Implemented Parity Work

- P0 Added mkcert CA trust-state probing: Missing, Untrusted, Trusted.
- P0 Updated sidecar status to distinguish CA missing from CA present but not trusted.
- P1 Added cert statuses: Ready, Missing CA, Expired, Untrusted, Regenerate Needed, Error.
- P1 Added loopback SAN generation for automatic certs: `localhost`, `127.0.0.1`, `::1`.
- P1 Added IP SAN parsing, not just DNS SAN parsing.
- P1 Added explicit SSL modes in the registry: Automatic Local HTTPS, Custom Certificate, Self-Signed fallback, Public ACME placeholder.
- P1 Added custom certificate and key path fields.
- P1 Added custom certificate loading into Caddy when the cert/key pair validates.
- P1 Added validation that custom cert files exist, are parseable, cover required SANs, are not expired, and match the private key when `openssl` is available.
- P1 Updated Domains UI to choose SSL mode and provide custom cert/key paths.
- P1 Updated Certificates UI and project detail health checks to show real certificate status.
- P2 Kept Public ACME visible as a disabled future placeholder for public domains only.

## Not Implemented Yet

- P1 Self-signed fallback certificate generation. The mode is represented and intentionally reports unsupported until generation, warning copy, and lifecycle cleanup are implemented.
- P1 CA uninstall/untrust workflow for PortBay's mkcert CA. ServBay exposes `caddy untrust`; PortBay should add a similarly explicit cleanup action before release.
- P2 Import/export of root CA and leaf certificates from the UI.
- P2 Public ACME/AutoSSL for real public domains.
- P2 Browser-specific trust checks for Firefox/NSS beyond mkcert's own install behavior.

## Security Notes

- Root CA private key contents must never be shown or logged.
- Leaf private key contents are not logged; custom pair validation compares public keys through `openssl`.
- Custom certificates are rejected unless the SAN list covers the project hostname and configured wildcard need.
- Public ACME must remain disabled for `.test` and other local-only names.
- `localhost` and loopback SANs improve local tool compatibility, but PortBay does not route every project on `localhost` because that would create ambiguous multi-project routing.
- Custom cert paths are references only; PortBay does not copy user keys into its managed cert directory.

## Manual QA Checklist

- Fresh install with no mkcert CA: Services shows CA not installed; HTTPS projects show Missing CA.
- Existing trusted CA: Services shows Trusted; automatic cert issuance succeeds.
- Existing untrusted CA: Services shows Untrusted; cert pages show Untrusted.
- Expired cert: certificate status shows Expired or Regenerate Needed.
- Deleted cert files: project shows not issued; reissue recreates certs.
- Invalid custom cert/key pair: cert reconciler reports custom certificate invalid and Caddy does not load it.
- Custom cert valid for project hostname: Caddy loads the provided cert/key.
- Wildcard subdomains enabled: cert SAN includes `*.hostname`.
- `localhost`, `127.0.0.1`, and `::1` SANs exist on newly issued automatic certs.
- Caddy reloads after reissue.
- Browser opens HTTPS project without warning when CA is trusted.
- App restart preserves SSL mode and custom cert paths.
- Project removal cleans PortBay-managed automatic cert dirs.
- Cleanup path does not remove user-provided custom cert/key files.

## Commands To Test

```sh
pnpm exec svelte-check --tsconfig ./tsconfig.json
cd src-tauri && cargo test certs --lib
cd src-tauri && cargo test mkcert --lib
security verify-cert -c "$(mkcert -CAROOT)/rootCA.pem" -l
openssl x509 -in ~/Library/Application\ Support/PortBay/certs/<project-id>/cert.pem -noout -text
curl -vk https://<project>.test
```
