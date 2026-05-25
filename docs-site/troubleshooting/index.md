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
| `HOSTS_FAILURE` | system | PortBay could not update or read the managed `/etc/hosts` block. When the sub-error is `PermissionDenied`, the action button supplies the exact `sudo` command to run. | Use the "Open Terminal with sudo command" action when it appears, or manually inspect the PortBay-delimited block between `# PortBay start` and `# PortBay end` in `/etc/hosts`. |
| `IO_FAILURE` | system | A filesystem or OS call failed (read, write, spawn, etc.). | Check that the target path exists and is not locked by another process. The `whatHappened` field includes the specific OS error message. |
| `SIDECAR_DOWN` | system | A required sidecar (process-compose or caddy) is not running or not reachable via its API. The `whatHappened` field names which sidecar. | Use the action button ("Restart process-compose" or "Restart Caddy") that appears with this error. If the sidecar immediately fails again, verify the binary exists under `src-tauri/binaries/` in development builds. |
| `PROJECT_NOT_FOUND` | user | A command referenced a project id that is not in the registry. | Refresh the project list. If the id came from a script or CLI call, confirm it matches exactly (ids are lowercase, hyphenated slugs). |
| `PORT_CONFLICT` | user | The port configured for this project is already bound by another process that PortBay did not manage. The `whatHappened` field identifies the holder. | Stop the conflicting process or edit the project's port in the detail panel, then retry. See [Port Conflicts](#port-conflicts) below. |
| `BAD_INPUT` | user | User input was malformed, empty where required, or failed validation. The `whatHappened` field names the specific field or constraint. | Read the `whatHappened` message and fix the highlighted input, then retry. |
| `PROJECT_CAP_REACHED` | user | Adding another project would exceed the current tier's project limit (anonymous: 3, free: 6, Pro: unlimited). | Sign in to raise the limit to 6, or upgrade to [PortBay Pro](/pro/) for unlimited projects. |
| `PRO_REQUIRED` | user | A Pro-gated feature was set or changed without an active Pro session. The `whatHappened` field names the feature. Existing configured values are never stripped — only the act of changing them without Pro is rejected. | Upgrade to [PortBay Pro](/pro/) to use this feature. Your existing configuration is unchanged. |
| `INTERNAL` | system | An unexpected failure that did not fit any narrower code. | Note the action you were performing, the project id, and any text in `whatHappened` and `details`, then [file an issue](https://github.com/tribalhouse/portbay/issues). |

---

## Port Conflicts

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

A project hostname that does not resolve in the browser involves four layers — check each in order:

1. **DNS / hosts** — Confirm `/etc/hosts` or dnsmasq resolves the hostname to `127.0.0.1`. Use `ping project.test` or `dig project.test @127.0.0.1`.
2. **Caddy route** — Confirm Caddy has an active route for the hostname. Restart Caddy and reconcile routes if needed. See [Caddy and HTTPS](/guides/caddy-https).
3. **Certificate** — If the browser shows a certificate warning, reissue the project cert (Certs panel) and restart Caddy.
4. **Project process** — Confirm the project process is actually listening on its configured port. Check the project log and the start command in the registry.

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

Relaunch PortBay. Re-add projects via **Add Project** or re-import from Portfiles if you have them.
