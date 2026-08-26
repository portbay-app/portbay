---
title: PortBay Troubleshooting — Error Codes & Diagnostic Guide
description: Fix PortBay errors fast — complete error code reference, port conflict resolution, sidecar restart steps, hostname failure checklist, and registry corruption recovery.
---

# Troubleshooting

## Error Envelope

Every Tauri command returns either a result value or a structured `CommandError` object. The shape is defined in `src/lib/types/error.ts` and mirrors the Rust `AppError` serializer:

```json
{
  "code": "SIDECAR_DOWN",
  "whatHappened": "caddy is not running",
  "whyItMatters": "Projects can't start until caddy is running again.",
  "whoCausedIt": "system",
  "actions": [
    { "label": "Restart Caddy", "command": "sidecars.restart_caddy" }
  ]
}
```

| Field | Type | Description |
| --- | --- | --- |
| `code` | `string` | Machine-readable error code — stable across releases. |
| `whatHappened` | `string` | One-sentence description of the failure. |
| `whyItMatters` | `string` | Why it blocks you and what the next step is. |
| `whoCausedIt` | `"user"` \| `"system"` | `user` means bad input or a limit you can fix; `system` means PortBay or the OS got into a bad state. |
| `actions` | `ErrorAction[]` | Buttons the error UI wires directly to frontend commands. May be empty. |
| `details` | `string?` | Inner error chain or stack trace (optional, shown in "Show details" expander). |

---

## Error Codes

The following codes are the complete set emitted by the backend across the IPC boundary. No other `code` values are produced.

| Code | `whoCausedIt` | What it means | Remedy |
| --- | --- | --- | --- |
| `REGISTRY_FAILURE` | system | PortBay could not read or write the project registry file. | Check file permissions under `~/Library/Application Support/PortBay`. If the JSON is corrupt, move it aside (after saving a copy) and restart PortBay to rebuild from scratch. |
| `PROCESS_COMPOSE_FAILURE` | system | A call to the process-compose daemon failed or was rejected. | Restart process-compose from Services, then retry the project action. If it immediately fails again, check that `process-compose` is in the binary bundle. |
| `CADDY_FAILURE` | system | Caddy's admin API rejected or did not receive the route update. Routes may be out of sync. | Restart Caddy from Services, then reconcile routes (or retry the project action). See [Caddy and HTTPS](/guides/caddy-https) for a full diagnostic checklist. |
| `DNSMASQ_FAILURE` | system | dnsmasq did not start or failed to apply wildcard DNS for `.test`. | Restart dnsmasq from Services. Confirm its resolver file (`/etc/resolver/test`) is installed. If dnsmasq is unavailable, fall back to `/etc/hosts` reconciliation. |
| `MAILPIT_FAILURE` | system | Mailpit did not start. | Restart Mailpit from Services. Confirm no other process owns the configured mail UI/API port. See [Mailpit](/guides/mailpit). |
| `TUNNEL_FAILURE` | system | cloudflared did not bring up the public tunnel. | Confirm the project is healthy locally first (open its `.test` hostname). Then restart the tunnel. See [Tunnels](/guides/tunnels). |
| `HOSTS_FAILURE` | system | PortBay could not update or read the managed `/etc/hosts` block. When the sub-error is `PermissionDenied`, the action button supplies the exact `sudo` command to run. | Use the "Open Terminal with sudo command" action when it appears, or manually inspect the PortBay-delimited block between `# BEGIN PortBay` and `# END PortBay` in `/etc/hosts` (a dev build uses `# BEGIN PortBay-dev` / `# END PortBay-dev` so it never clobbers an installed app's entries). |
| `IO_FAILURE` | system | A filesystem or OS call failed (read, write, spawn, etc.). | Check that the target path exists and is not locked by another process. The `whatHappened` field includes the specific OS error message. |
| `SIDECAR_DOWN` | system | A required sidecar (process-compose or caddy) is not running or not reachable via its API. The `whatHappened` field names which sidecar. | Use the action button ("Restart process-compose" or "Restart Caddy") that appears with this error. If the sidecar immediately fails again, verify the binary exists under `src-tauri/binaries/` in development builds. |
| `PROJECT_NOT_FOUND` | user | A command referenced a project id that is not in the registry. | Refresh the project list. If the id came from a script or CLI call, confirm it matches exactly (ids are lowercase, hyphenated slugs). |
| `PORT_CONFLICT` | user | The port configured for this project is already bound by another process that PortBay did not manage. The `whatHappened` field identifies the holder. | Stop the conflicting process or edit the project's port in the detail panel, then retry. See [Project Port Conflicts](#project-port-conflicts) below. |
| `BAD_INPUT` | user | User input was malformed, empty where required, or failed validation. The `whatHappened` field names the specific field or constraint. | Read the `whatHappened` message and fix the highlighted input, then retry. |
| `PROJECT_CAP_REACHED` | user | Adding another project would exceed the current tier's project limit (anonymous: 3, free: 6, Pro: unlimited). | Sign in to raise the limit to 6, or upgrade to [PortBay Pro](/pro/) for unlimited projects. |
| `PRO_REQUIRED` | user | A Pro-gated feature was set or changed without an active Pro session. The `whatHappened` field names the feature. Existing configured values are never stripped — only the act of changing them without Pro is rejected. | Upgrade to [PortBay Pro](/pro/) to use this feature. Your existing configuration is unchanged. |
| `INTERNAL` | system | An unexpected failure that did not fit any narrower code. | Note the action you were performing, the project id, and any text in `whatHappened` and `details`, then [file an issue](https://github.com/tribalhouse/portbay/issues). |

---

## Project Port Conflicts

When a project fails with `PORT_CONFLICT`, find what owns the port:

```bash
lsof -nP -iTCP:<port> -sTCP:LISTEN
```

Stop the external process or change the project to use a free port. Do not run two tools that supervise the same port (e.g. ServBay and PortBay on port 443).

---

## Sidecar Failures

1. Open **Services** in the sidebar.
2. Refresh sidecar status.
3. Restart the failed sidecar.
4. Retry the project action.
5. If the sidecar immediately crashes again, open its log from Services and check the last 20 lines.
6. In development builds, verify the sidecar binary exists under `src-tauri/binaries/`.

---

## Hostname Failures

A project hostname that does not resolve in the browser involves four layers — check each in order. Doctor (**Settings → Doctor**, or `portbay doctor`) runs an end-to-end check of the first two layers automatically, so start there before working through this list by hand.

1. **DNS / hosts** — Confirm `/etc/hosts` or dnsmasq resolves the hostname to `127.0.0.1`. Use `ping project.test`, or `dig` against dnsmasq directly — dnsmasq is **not** on port 53 (see [DNS Under VPN / Corporate Networks](#dns-under-vpn--corporate-networks) below for why), so you must pass its actual port: `dig project.test @127.0.0.1 -p 25353` (25353 is the default; the exact port is on the DNS page and in `portbay_dns_status`). A plain `dig project.test @127.0.0.1` without `-p` queries port 53 and fails even when dnsmasq is perfectly healthy.
2. **Caddy route** — Confirm Caddy has an active route for the hostname. Restart Caddy and reconcile routes if needed. See [Caddy and HTTPS](/guides/caddy-https).
3. **Certificate** — If the browser shows a certificate warning, reissue the project cert (Certs panel) and restart Caddy. See [Certificate Re-Trust](#certificate-re-trust-and-browser-warnings) below if the warning persists after reissuing.
4. **Project process** — Confirm the project process is actually listening on its configured port. Check the project log and the start command in the registry.

---

## DNS Under VPN / Corporate Networks

The most common "it worked yesterday" ticket: a VPN client or a corporate MDM profile installs its own DNS configuration — often a full-tunnel VPN or a split-DNS resolver pushed by the network — that silently overrides or shadows the `/etc/resolver/<suffix>` file PortBay installs for wildcard `*.test` resolution.

**Symptoms:** project hostnames resolved fine before connecting to the VPN, then stopped; or they've never worked at all on a managed/corporate machine.

**Why it happens:** macOS resolves a query for `sub.project.test` by picking the *most specific* matching resolver file under `/etc/resolver/`. A VPN client that installs its own resolver for a broader or equally-specific domain (some corporate VPNs install a resolver for the empty domain, effectively becoming the default resolver for everything) can end up ahead of, or instead of, `/etc/resolver/test` in that precedence — the standard behavior of `/etc/resolver/`, not a PortBay bug. dnsmasq itself may also start refusing connections on its normal port if the VPN's own DNS process claims it first, but that shows as a plain port conflict (see below), not this precedence issue.

**Diagnose:**

```bash
# What does the resolver actually pick for the suffix?
scutil --dns | grep -B2 -A6 "test$"

# Confirm the file is still there and points at PortBay's dnsmasq
cat /etc/resolver/test

# Query dnsmasq directly, bypassing system resolution entirely
dig project.test @127.0.0.1 -p 25353   # confirms dnsmasq itself is healthy
```

If `scutil --dns` doesn't show `/etc/resolver/test` taking priority, the VPN's resolver is winning. There is no cross-platform fix PortBay can apply here (the VPN vendor owns that precedence) — the practical options are:

- Add the specific hostname to `/etc/hosts` instead of relying on the wildcard (Doctor's hosts check and the DNS page both support per-hostname entries, which are resolved before any resolver file).
- Disconnect the VPN's full-tunnel DNS override while doing local work, if your VPN client supports split-tunnel DNS.
- Ask your IT team whether the VPN profile can exclude `*.test` (or your configured suffix) from its DNS capture.

---

## Port Conflicts (53 / 80 / 443)

### Port 53 — dnsmasq

PortBay's dnsmasq sidecar does **not** bind port 53 by default; it binds an unprivileged port (25353 by default, scanning upward if taken) precisely so it never needs root and never fights another resolver for port 53. You will only see a port-53 conflict if you've manually pointed something at PortBay's dnsmasq expecting it on 53, or you're running a second local DNS server (a corporate VPN's local resolver, `mDNSResponder` add-ons, another dev tool's bundled dnsmasq) that *does* want port 53 for itself — that's a conflict between those two tools, not with PortBay. Confirm PortBay's actual port from the DNS page or `portbay_dns_status`, and point any manual `dig`/`nslookup`/resolver config at that port, not 53.

### Ports 80 / 443 — Caddy

Doctor proactively checks whether ports 80 and 443 are free before you ever hit a failure — this is the single most common "why won't my HTTPS site load" ticket, because it's easy to have another tool (ServBay, Herd, MAMP/XAMPP, a stray `nginx`, a previous PortBay dev build) still holding one of them.

```bash
lsof -nP -iTCP:443 -sTCP:LISTEN
lsof -nP -iTCP:80 -sTCP:LISTEN
```

Stop the conflicting process, or stop the other local-dev tool entirely — don't run two tools that both try to supervise 80/443 (see [Project Port Conflicts](#project-port-conflicts) above for the general per-project case). Note that as of GSTACK-0718, Caddy's `:443` and `:80` listeners gate any **non-loopback** peer (a LAN device, or a tunnel) behind the project's sharing/tunnel settings — your own browser on the same machine (loopback) is never gated, so this can't be the cause of a same-machine "won't load" symptom; it only affects reaching the project from another device.

---

## Browser HSTS Cache

**Symptom:** a `.test` hostname loaded fine over HTTPS once, was reissued or switched to a different SSL mode, and now the browser refuses to even attempt an insecure fallback or shows a certificate error that persists even after reissuing the certificate and restarting Caddy.

**Why it happens:** browsers cache HTTP Strict Transport Security (HSTS) directives per-hostname, sometimes for up to a year, independent of PortBay or the certificate itself. Once a hostname has sent `Strict-Transport-Security`, the browser refuses plain HTTP to that exact hostname and pins the certificate chain it saw, until the HSTS entry expires or is cleared — this is standard browser behavior on any HSTS-enabled site, not specific to local development.

**Fix:**

- **Chrome/Edge:** visit `chrome://net-internals/#hsts`, enter the hostname under "Delete domain security policies," and delete it.
- **Firefox:** Firefox derives HSTS state from its cache/cert-exception store; clearing "Cookies and Site Data" for the hostname (Site Information panel → Clear Data) also clears HSTS for it.
- **Safari:** Safari doesn't expose a per-site HSTS clear; the practical fix is `sudo dscacheutil -flushcache` for stale DNS plus quitting and relaunching Safari, or waiting out the HSTS max-age. Testing in a private window sidesteps a stuck HSTS entry without clearing browser state.

---

## Certificate Re-Trust and Browser Warnings

If reissuing the project's certificate (Certs panel → row action, or `portbay_reissue_cert`) doesn't clear a browser warning:

1. **Confirm the CA itself is still trusted.** Doctor's "mkcert CA trust" check (Web routing & TLS category) reads the same `rootCA.pem` any `mkcert` binary would report and verifies it against the OS/browser trust store — this catches the case where the CA was removed from Keychain (Keychain Access → System or login keychain → search "mkcert") after PortBay installed it, which nothing else surfaces. If Doctor reports the CA missing or untrusted, reinstall it from **Settings → Domains & HTTPS → Install CA**.
2. **Confirm you're not hitting the browser's HSTS cache** for that hostname — see above. A stale HSTS entry can look identical to a real certificate problem.
3. **Confirm the certificate's SAN actually covers the hostname you're visiting**, especially after enabling `include_wildcard_subdomains` or switching SSL mode — a certificate issued before that change won't retroactively cover a new subdomain until reissued. See [Certificates](/guides/certificates).
4. **Custom certificates** (`SslMode::CustomCertificate`): the certificate only needs to cover the project's own hostname (and `*.hostname` if wildcard subdomains are on) — it does **not** need `localhost`/`127.0.0.1`/`::1` SANs, those are only required on PortBay's own mkcert-issued certificates. If validation fails, the error names exactly which SAN is missing.
5. Restart Caddy after any of the above, since a certificate swap doesn't take effect until Caddy reloads.

---

## Registry Corruption

If `REGISTRY_FAILURE` appears on startup and does not clear after a restart:

```bash
# Back up the current file first
cp ~/Library/Application\ Support/PortBay/registry.json \
   ~/Desktop/registry-backup-$(date +%Y%m%d).json

# Move it aside so PortBay rebuilds on next launch
mv ~/Library/Application\ Support/PortBay/registry.json \
   ~/Library/Application\ Support/PortBay/registry.json.corrupt
```

Relaunch PortBay. Re-add projects via **Add Project** or re-import from Portfiles if you have them. In practice this should be rare: the registry writer holds both an in-process mutex and a cross-process file lock, and a malformed registry is quarantined to `registry.json.unreadable.bak` rather than crashing the app — but if you hit it anyway, the steps above recover cleanly.

---

## Uninstall / Reset

To fully remove PortBay's system-level footprint (not just the app itself) — useful before a clean reinstall, or when handing a machine back to IT:

1. **Stop everything first.** Quit PortBay (or run `portbay stop --all` if the CLI is installed) so no sidecar holds a port during cleanup.
2. **Remove the privileged helper.** From **DNS** in the sidebar, use the uninstall/reset action if present; otherwise remove the LaunchDaemon manually:
   ```bash
   sudo launchctl bootout system/com.portbay-app.portbay.hosts-helper
   sudo rm /Library/LaunchDaemons/com.portbay-app.portbay.hosts-helper.plist
   sudo rm /usr/local/bin/portbay-hosts-helper
   ```
3. **Clean `/etc/hosts`.** Delete everything between (and including) the `# BEGIN PortBay` / `# END PortBay` marker lines. Do this with a text editor, not by hand-deleting individual lines, so you don't leave a mismatched marker behind.
4. **Remove the resolver file.**
   ```bash
   sudo rm /etc/resolver/test   # or your configured suffix
   ```
5. **Remove the trusted CA** from Keychain Access (search "mkcert," delete the certificate from the login keychain) if you don't intend to reinstall PortBay, or want a fresh CA on next install.
6. **Remove app data** (registry, certs, logs, generated configs) at `~/Library/Application Support/PortBay/` — back it up first if you might reinstall and want your projects back.
7. **Reinstall from scratch** re-triggers both first-run authorization prompts (see [First Run](/getting-started/first-run)) and generates a brand-new, freshly-scoped CA.
