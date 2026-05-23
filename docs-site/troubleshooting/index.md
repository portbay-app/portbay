# Troubleshooting

Every Tauri command error is serialized as a `CommandError` envelope:

```json
{
  "code": "SIDECAR_DOWN",
  "whatHappened": "process-compose is not running",
  "whyItMatters": "Projects can't start until process-compose is running again.",
  "whoCausedIt": "system",
  "actions": [
    { "label": "Restart process-compose", "command": "sidecars.restart_pc" }
  ]
}
```

## Error Codes

| Code | Meaning | Remediation |
| --- | --- | --- |
| `REGISTRY_FAILURE` | PortBay could not read or write the project registry. | Check file permissions under `~/Library/Application Support/PortBay`, then retry. If the JSON is corrupt, restore from backup or move it aside after saving a copy. |
| `PROCESS_COMPOSE_FAILURE` | The action did not reach Process Compose or Process Compose rejected it. | Restart Process Compose from Services, then retry the project action. |
| `CADDY_FAILURE` | Caddy failed to apply or serve the requested route. | Restart Caddy, reconcile routes, and confirm the target project is listening on the configured port. |
| `DNSMASQ_FAILURE` | dnsmasq failed to start or apply wildcard DNS. | Restart dnsmasq, confirm its resolver file is installed, and fall back to host reconciliation if needed. |
| `MAILPIT_FAILURE` | Mailpit did not start. | Restart Mailpit from Services and check whether another process owns the configured mail UI/API port. |
| `TUNNEL_FAILURE` | cloudflared did not bring up the public tunnel. | Restart the tunnel and confirm the project is healthy locally before retrying a public URL. |
| `HOSTS_FAILURE` | PortBay could not update or read the managed `/etc/hosts` block. | Use the sudo helper action when available, or manually inspect the PortBay-delimited block. |
| `IO_FAILURE` | A filesystem or OS operation failed. | Check path existence, permissions, and whether the target file is locked by another process. |
| `SIDECAR_DOWN` | A required sidecar is not running or not reachable. | Restart the named sidecar. For `process-compose` and `caddy`, PortBay exposes direct restart actions. |
| `PROJECT_NOT_FOUND` | A command referenced a project id not present in the registry. | Refresh the project list, confirm the id, then retry. |
| `PORT_CONFLICT` | The configured port is already owned by another process. | Stop the conflicting process or edit the project to use a free port. |
| `BAD_INPUT` | User input was malformed or incomplete. | Fix the highlighted field or command argument and retry. |
| `INTERNAL` | The failure did not fit a narrower code. | Capture the action, project id, and logs before filing an issue. |

## Port Conflicts

If a project fails with `PORT_CONFLICT`:

```bash
lsof -nP -iTCP:<port> -sTCP:LISTEN
```

Stop the external process or change the project port. Do not let two tools supervise the same port.

## Sidecar Failures

1. Open Services.
2. Refresh status.
3. Restart the failed sidecar.
4. Retry the project action.
5. If the sidecar immediately fails again, verify the sidecar binary exists under `src-tauri/binaries/` in development.

## Hostname Failures

1. Confirm the project hostname is correct.
2. Confirm `/etc/hosts` or dnsmasq resolves the hostname to localhost.
3. Confirm Caddy has a route for the hostname.
4. Confirm the project process is listening on its configured port.
